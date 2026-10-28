"""Parse current perf counter data using an explicit protocol event table."""
import math

SEP = ';'

MARKERS = ('<not counted>', '<not supported>')

class ParseError(Exception): pass

def _num(s, what):
	try: v = int(s)
	except ValueError:
		try: v = float(s)
		except ValueError: raise ParseError(('not a number', what, s))
	if isinstance(v, float) and not math.isfinite(v): raise ParseError(('non-finite', what, s))
	return v

def parse(text, group, event_table, groups):
	"""Parse one `perf stat --no-scale -x';' -o` file for `group`. Returns {'events': {key: rec}, 'extra': [lines]}.
	rec = {'value': int|float|None, 'marker': str|None, 'runtime_ns': int|None, 'pcnt': float|None,
	       'available': bool, 'reason': str|None, 'line': raw line}.
	A value is available only when it is a plain non-negative number, its time_running is a positive integer AND its
	pcnt == 100.00 (perf's rounded running/enabled; no multiplexing seen): <not counted>/<not supported>, a missing or
	zero runtime, a missing pcnt or pcnt < 100 make it unavailable with a reason, never zero. A negative or non-integer
	runtime, or a pcnt outside [0, 100], raises ParseError. Blank lines, '#' header lines and metric-only lines (empty event field) are skipped. A requested event
	that is missing or appears twice raises ParseError; a line naming an unrequested event goes to 'extra'."""
	want = groups[group]
	accept = {}
	for k in want:
		for n in event_table[k][1]:
			accept[n] = k
	events, extra = {}, []
	for line in text.splitlines():
		if not line.strip() or line.startswith('#'): continue
		f = line.split(SEP)
		if len(f) < 3: raise ParseError(('short line', line))
		value, _unit, name = f[0].strip(), f[1].strip(), f[2].strip()
		if not name:
			continue  # metric-only line
		k = accept.get(name)
		if k is None:
			extra.append(line); continue
		if k in events: raise ParseError(('duplicate event', k, line))
		rec = {'value': None, 'marker': None, 'runtime_ns': None, 'pcnt': None, 'available': False, 'reason': None,
			'line': line}
		if value in MARKERS:
			rec['marker'] = value; rec['reason'] = value
		else:
			v = _num(value, k)
			if v < 0: raise ParseError(('negative count', k, line))
			rec['value'] = v
		if len(f) > 3 and f[3].strip():
			rt = _num(f[3].strip(), k + ' runtime')
			if not isinstance(rt, int) or rt < 0: raise ParseError(('bad runtime', k, line))
			rec['runtime_ns'] = rt
		if len(f) > 4 and f[4].strip():
			rec['pcnt'] = float(_num(f[4].strip(), k + ' pcnt'))
			if not 0.0 <= rec['pcnt'] <= 100.0: raise ParseError(('pcnt out of range', k, line))
		if rec['marker'] is None:
			if not rec['runtime_ns']: rec['reason'] = 'runtime missing or zero'
			elif rec['pcnt'] is None: rec['reason'] = 'no pcnt field'
			elif rec['pcnt'] != 100.0: rec['reason'] = f"pcnt {f[4].strip()} (multiplexed or off cpu_core)"
			else: rec['available'] = True
		events[k] = rec
	missing = [k for k in want if k not in events]
	if missing: raise ParseError(('missing events', missing))
	return {'events': events, 'extra': extra}

def probe_ok(parsed):
	"""Operability, proven per group on /bin/true: every requested event available, no extra event lines, and one
	shared time_running across the group (members scheduled together)."""
	ev = parsed['events']
	rts = {r['runtime_ns'] for r in ev.values()}
	why = [f"{k}: {r['reason']}" for k, r in ev.items() if not r['available']]
	if parsed['extra']: why.append(f"extra lines: {parsed['extra']}")
	if len(rts) != 1 or None in rts or 0 in rts: why.append(f'group runtimes differ or zero: {sorted(map(str, rts))}')
	return not why, why

def decode(raw):
	"""The perf -o file as text, losslessly: strict UTF-8 or ParseError (never a replacement character)."""
	try: return raw.decode('utf-8')
	except UnicodeDecodeError as e: raise ParseError(('invalid utf-8', repr(e)))

def classify_probe(status, out, err, perf_raw, group, event_table, groups):
	"""The whole classification of one /bin/true probe from its raw evidence (exit status, stdout/stderr/perf bytes).
	Used by the controller to decide and by the analyzer to recompute; both must agree field for field."""
	c = {'perf_raw': None, 'perf': None, 'parse_error': None,
		'output_ok': status == 0 and out == b'' and err == b''}
	try:
		c['perf_raw'] = decode(perf_raw)
		c['perf'] = parse(c['perf_raw'], group, event_table, groups)
	except ParseError as e:
		c['parse_error'] = repr(e)
	ok, why = probe_ok(c['perf']) if c['perf'] else (False, [c['parse_error']])
	if not c['output_ok']:
		ok, why = False, why + [f'status {status} stdout_bytes {len(out)} stderr_bytes {len(err)}']
	c['probe_ok'], c['probe_why'] = ok, why
	return c

def workload_problems(parsed):
	"""Fail-closed checks for a workload row: no unrequested event lines, and every AVAILABLE counter shares one
	time_running (the group was scheduled together). Unavailable counters stay None with their reason."""
	why = []
	if parsed['extra']: why.append(f"extra lines: {parsed['extra']}")
	rts = {r['runtime_ns'] for r in parsed['events'].values() if r['available']}
	if len(rts) > 1: why.append(f'available runtimes differ: {sorted(rts)}')
	return why
