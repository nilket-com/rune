"""Joint fixed-sample bootstrap for live measurement results."""
import math
import statistics

B = 10000

def pct(sorted_xs, q):
	i = min(len(sorted_xs) - 1, max(0, int(q * len(sorted_xs))))
	return sorted_xs[i]

def finite_pos(x):
	return x is not None and isinstance(x, (int, float)) and math.isfinite(x) and x > 0

def joint_bootstrap(series, n, rng, ratios, diffs=(), keep=False):
	"""series: {name: [value per used round, 0..n-1]}, the SAME n rounds for every name.
	ratios: [(id, num, den)]; diffs: [(id, a, b)] (median(a) - median(b)). Each of B resamples
	draws ONE index vector and applies it to every series; every statistic of that resample comes
	from those medians. Denominators must be finite and > 0 at the point, in every resample, and
	at their own bootstrap lower bound. Returns ({id: record}, keep_data or None)."""
	for name, xs in series.items():
		if len(xs) != n:
			raise ValueError("unequal round sets: the joint bootstrap needs complete tuples")
	ids = [r[0] for r in ratios] + [d[0] for d in diffs]
	if n < 2:
		return {i: {"unresolved": f"fewer than 2 complete rounds ({n})"} for i in ids}, None
	names = sorted(series)
	if any(not all(isinstance(x, (int, float)) and math.isfinite(x) for x in series[nm]) for nm in names):
		return {i: {"unresolved": "a non-finite value in the series"} for i in ids}, None
	point = {nm: statistics.median(series[nm]) for nm in names}
	draws = {i: [] for i in ids}
	med_draws = {nm: [] for nm in names}
	failed = {}
	idx_log = [] if keep else None
	for rid, num, den in ratios:
		if not finite_pos(point[den]):
			failed[rid] = "point estimate"
	for b in range(B):
		idx = rng.choices(range(n), k=n)
		if keep:
			idx_log.append(idx)
		med = {nm: statistics.median(series[nm][i] for i in idx) for nm in names}
		for nm in names:
			med_draws[nm].append(med[nm])
		for rid, num, den in ratios:
			if not finite_pos(med[den]):
				failed.setdefault(rid, f"bootstrap resample {b}")
				draws[rid].append(None)
			else:
				draws[rid].append(med[num] / med[den])
		for did, a, c in diffs:
			draws[did].append(med[a] - med[c])
	out = {}
	for rid, num, den in ratios:
		rec = {"rounds": n, "method": f"joint percentile bootstrap over complete round tuples, B={B}; one index vector per resample for every subject of the cell"}
		den_lo = pct(sorted(med_draws[den]), 0.025)
		if rid in failed:
			rec["unresolved"] = f"denominator {den} not finite and strictly positive ({failed[rid]})"
		elif not finite_pos(den_lo):
			rec["unresolved"] = f"denominator {den}: its own bootstrap lower bound is not finite and strictly positive"
		else:
			xs = sorted(draws[rid])
			rec.update({"ratio": point[num] / point[den], "ci95": [pct(xs, 0.025), pct(xs, 0.975)],
				"denominator_lower_bound": den_lo})
		out[rid] = rec
	for did, a, c in diffs:
		xs = sorted(draws[did])
		out[did] = {"rounds": n, "difference": point[a] - point[c], "ci95": [pct(xs, 0.025), pct(xs, 0.975)],
			"method": f"joint percentile bootstrap, B={B}"}
	return out, ({"draws": draws, "idx": idx_log, "point_medians": point} if keep else None)
