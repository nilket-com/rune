#!/usr/bin/env python3
"""B1 calls probe (revision 3 of the P2 probe package): externally supplied, deterministic
input streams (derived from the P0 harness's gen_inputs.py; this file's sha256 is in PINS.json).

usage: gen_inputs.py OUT_DIR [SEED]

Per stream, writes K_MAX integers drawn i.i.d. from a seeded PRNG (Python's Mersenne
Twister via getrandbits), the per-input expected results of each kernel that uses the
stream (an independent Python model), the expected checksum at K = 1, K1 and K2 of every
cell that uses it, and MANIFEST.json with all hashes. The seed is drawn once from
os.urandom unless given, and recorded. Inputs are generated once, before any timing, and are
never regenerated after a result exists.

Codex corrections implemented here:

- K1 and K2 read the SAME file: the K1 inputs are exactly the first K1 lines of the K2
  inputs (shared prefix by construction). Every line is an i.i.d. draw from the same
  range, so the tail (lines K1+1..K2) has the prefix's distribution; the manifest records
  prefix and tail summary statistics and a two-sample Kolmogorov-Smirnov statistic as a
  descriptive check (reported, not a gate).
- Per-iteration work is derived from the ACTUAL tail inputs: the manifest records, per
  cell, the sum over the tail of the loop-iteration count of each input. The model counts
  iterations by running the loop; for both `while` and `numeric` one invocation with input
  x runs the body exactly max(x, 0) times and evaluates the header compare max(x, 0) + 1
  times. Both counts are recorded and asserted against the closed forms.
- The `tiny` stream (x in [0, 3]) is a fixed-cost-dominated DIAGNOSTIC, not a pure
  entry+exit measurement: it also contains harness, invoke and checksum work and 0..3 loop
  iterations.

Revision 3 (B1 calls probe, CALLS-PROBE-PLAN.md section 2): the cell table only. The
`while/tiny` and `numeric/tiny` cells are dropped and `calls/tiny` (the `tiny` stream, K1
1,000 / K2 101,000) is added; `calls/calls_main` keeps its stream and K1 100 / K2 10,100. The
five streams, their ranges, their draw order and every file format are unchanged. For calls
(`for i in 0..x { total = add(total, i) }`) one invocation with input x runs the loop body
(one inlined `add`) max(x, 0) times and the header IterNext max(x, 0) + 1 times; both are
counted literally per distinct value, asserted against the closed forms and recorded as
`tail_body_iterations` / `tail_header_evaluations` (`header_kind` IterNext). The calls model
result x * (x - 1) / 2 is also checked against a literal run of the loop per distinct value.

The models run the kernels' loops literally (no closed form for while/numeric), memoised
per distinct input value.

Bounds: every intermediate of every kernel stays within +-2^31 (asserted), so LuaJIT's
doubles, PUC Lua's integers and Rune's checked i64 agree exactly. LuaJIT performs no
overflow or fuel checks; Rune performs both.
"""
import bisect, functools, hashlib, json, math, os, random, sys

K1, K2 = 1_000, 101_000
K1_CONTROL, K2_CONTROL = 100, 10_100
K_MAX = K2

# Streams: (low, high) inclusive, fixed before any timing.
STREAMS = {
	"loop_main": (9_000, 11_000),
	"tiny": (0, 3),
	"calls_main": (9_000, 11_000),
	"fib_main": (18, 21),
	"shared_rows_main": (0, 1_000_000),
}

# Cells: (kernel, stream, K1, K2).
CELLS = [
	("calls", "calls_main", K1_CONTROL, K2_CONTROL),
	("calls", "tiny", K1, K2),
	("while", "loop_main", K1, K2),
	("numeric", "loop_main", K1, K2),
	("fib", "fib_main", K1_CONTROL, K2_CONTROL),
	("shared_rows", "shared_rows_main", K1, K2),
]


@functools.lru_cache(maxsize=None)
def loop_counts(x):
	"""Runs the while/numeric loop shape literally: (body iterations, header evaluations)."""
	n, body, header = 0, 0, 0
	while True:
		header += 1
		if not n < x:
			break
		body += 1
		n += 1
	return body, header


@functools.lru_cache(maxsize=None)
def range_counts(x):
	"""Runs the calls kernel's `for i in 0..x` shape literally: (body iterations, IterNext
	evaluations, the folded total). The range is empty for x <= 0."""
	i, body, iternext, total = 0, 0, 0, 0
	while True:
		iternext += 1
		if not i < x:
			break
		total = total + i  # the inlined add(total, i)
		body += 1
		i += 1
	return body, iternext, total


@functools.lru_cache(maxsize=None)
def model(kernel, x):
	if kernel == "while":
		n = 0
		while n < x:
			n += 1
		return n
	if kernel == "numeric":
		n, total = 0, 0
		while n < x:
			total += n % 17
			n += 1
		return total
	if kernel == "calls":
		return x * (x - 1) // 2
	if kernel == "fib":
		a, b = 0, 1
		for _ in range(x):
			a, b = b, a + b
		return a
	if kernel == "shared_rows":
		a = x % 7 + x % 13 + 1
		b = x % 11
		c = x % 17
		return a * 1_000_000 + b * 10_000 + c * 100 + 2 * 10 + 3
	raise ValueError(kernel)


def checksum(results):
	c = 0
	for r in results:
		c = (c * 31 + r) % 1_000_000_007
	return c


def summary(xs):
	n = len(xs)
	mean = sum(xs) / n
	var = sum((x - mean) ** 2 for x in xs) / (n - 1) if n > 1 else 0.0
	return {"n": n, "mean": mean, "sd": math.sqrt(var), "min": min(xs), "max": max(xs), "sum": sum(xs)}


def ks(a, b):
	"""Two-sample Kolmogorov-Smirnov statistic D (descriptive only)."""
	a, b = sorted(a), sorted(b)
	d = 0.0
	for v in sorted(set(a) | set(b)):
		fa = bisect.bisect_right(a, v) / len(a)
		fb = bisect.bisect_right(b, v) / len(b)
		d = max(d, abs(fa - fb))
	return d


def sha(path):
	with open(path, "rb") as stream:
		return hashlib.sha256(stream.read()).hexdigest()


def main(out_dir, seed=None):
	seed = int.from_bytes(os.urandom(8), "little") if seed is None else seed
	rng = random.Random(seed)
	os.makedirs(out_dir, exist_ok=False)
	manifest = {"seed": seed, "k_max": K_MAX, "streams": {}, "cells": {}}

	xs_by_stream = {}
	for stream, (lo, hi) in STREAMS.items():
		xs = [lo + rng.getrandbits(32) % (hi - lo + 1) for _ in range(K_MAX)]
		xs_by_stream[stream] = xs
		path = os.path.join(out_dir, f"{stream}.inputs.txt")
		with open(path, "w") as f:
			f.write("\n".join(map(str, xs)) + "\n")
		manifest["streams"][stream] = {"path": os.path.basename(path), "sha256": sha(path), "range": [lo, hi]}

	for kernel, stream, k1, k2 in CELLS:
		xs = xs_by_stream[stream][:k2]
		results = [model(kernel, x) for x in xs]
		assert all(abs(r) < 2**31 for r in results), (kernel, stream)
		path = os.path.join(out_dir, f"{kernel}.{stream}.expected.txt")
		with open(path, "w") as f:
			f.write("\n".join(map(str, results)) + "\n")
		prefix, tail = xs[:k1], xs[k1:k2]
		cell = {
			"kernel": kernel,
			"stream": stream,
			"k1": k1,
			"k2": k2,
			"expected_path": os.path.basename(path),
			"expected_sha256": sha(path),
			"expected_checksum": {str(k): checksum(results[:k]) for k in (1, k1, k2)},
			"prefix_inputs": summary(prefix),
			"tail_inputs": summary(tail),
			"tail_vs_prefix_ks_d": ks(prefix, tail),
		}
		if kernel == "calls":
			assert all(range_counts(x)[2] == r for x, r in zip(xs, results)), (kernel, stream)
			body = header = 0
			for x in tail:
				b, h, _ = range_counts(x)
				assert b == max(x, 0) and h == max(x, 0) + 1, (kernel, x, b, h)
				body += b
				header += h
			cell["tail_body_iterations"] = body
			cell["tail_header_evaluations"] = header
			cell["header_kind"] = "IterNext"
		if kernel in ("while", "numeric"):
			body = header = 0
			for x in tail:
				b, h = loop_counts(x)
				assert b == max(x, 0) and h == max(x, 0) + 1, (kernel, x, b, h)
				body += b
				header += h
			# `while` returns its iteration count, a direct check of the loop semantics.
			if kernel == "while":
				assert all(model("while", x) == loop_counts(x)[0] for x in tail[:1000])
			cell["tail_body_iterations"] = body
			cell["tail_header_evaluations"] = header
		manifest["cells"][f"{kernel}/{stream}"] = cell

	with open(os.path.join(out_dir, "MANIFEST.json"), "w") as f:
		json.dump(manifest, f, indent=1, sort_keys=True)


if __name__ == "__main__":
	main(sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else None)
