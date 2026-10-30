"""One-record E1a Callgrind replay; original captures and rules stay immutable.

The exception removes only the exactly reviewed entry costs for the core bound.
Raw costs, whole-process limits, controls and the 72-Ir/invocation bound remain
visible and checked. Later candidates cannot inherit this qualification.
"""
import copy
import hashlib
import json
import re
from pathlib import Path
import sys
from fractions import Fraction

sys.path.insert(0, str(Path(__file__).resolve().parent))
import callgrind
import callgrind_packet
import callgrind_rules
import gen_inputs
import input_closure

TIP = '3eea6d307a5df84a24f5f75d2e4fc9362254a80b'
CANDIDATE = '1d1bb786e5de14beea771af8cf4f63af33fe3bdd'
INVOKE = '<rune::experimental_core::execute::Executor>::invoke'
INIT = '<rune::experimental_core::execute::Executor>::with_limits'
KERNELS = ('while', 'numeric', 'calls', 'fib', 'shared_rows')
POLICY = {'schema': 1, 'record': 'E1a', 'baseline_milestone': 'M8',
		  'candidate_commit': CANDIDATE, 'tip_commit': TIP,
		  'entries': [
			  {'rule': 'native-policy-entry', 'symbol': INVOKE,
			   'cells': {'on/' + k: 3000 if k in KERNELS[:3] else 2002 for k in KERNELS}, 'calls': 1000},
			  {'rule': 'native-policy-initialization', 'symbol': INIT,
			   'cells': {mode + '/' + k: 1 for mode in ('off', 'on') for k in KERNELS}, 'calls': 1}],
		  'proof': 'live-hot-closure-and-native-source-equal'}


def require(ok, message):
	if not ok:
		raise ValueError(message)


def qualify(tip, candidate, cell, policy):
	require(policy == POLICY, 'unreviewed E1a policy or candidate')
	expected = {entry['symbol']: (entry['cells'][cell], entry['calls'])
				for entry in policy['entries'] if cell in entry['cells']}
	require(expected, 'cell outside E1a policy')
	differences = callgrind.core_differences(tip, candidate)
	positive = {row['function']: row for row in differences if row['delta'] > 0}
	require(set(positive) == set(expected), 'E1a positive core symbols differ')
	for symbol, (delta, calls) in expected.items():
		row = positive[symbol]
		require(row['delta'] == delta and row['tip_calls'] == row['candidate_calls'] == calls,
				'E1a exact entry cost or call count differs')
	group = [row for row in differences if row['function'] != callgrind_rules.ITER
			 and row['calls'] is not None and row['calls'] <= 1002]
	require(sum(row['delta'] for row in group) <= 72000, 'raw per-invocation bound exceeded')
	require(Fraction(candidate['total_Ir'] - tip['total_Ir'], tip['total_Ir']) <= Fraction(1, 1000),
			'raw whole-process bound exceeded')
	adjusted = copy.deepcopy(candidate)
	allowance = sum(delta for delta, _ in expected.values())
	for symbol, (delta, _) in expected.items():
		adjusted['functions'][symbol]['self_Ir'] -= delta
	adjusted['total_Ir'] -= allowance
	return adjusted, {'cell': cell, 'qualified_entry_Ir': allowance,
					  'raw_core': callgrind.core_total(candidate),
					  'raw_total_Ir': candidate['total_Ir'], 'raw_differences': differences}


def validate_controls(deltas):
	# The existing rules assess whole-process controls themselves. E1a adds
	# no new whole-control bound; its entry costs must cancel in core deltas.
	require(all(row['core'] == 0 for row in deltas.values()), 'E1a core controls changed')


def analyze(root, proof):
	root = Path(root)
	policy = json.loads((Path(__file__).parent / 'qualifications/e1a.json').read_text())
	require(policy == POLICY, 'unreviewed E1a policy')
	require(isinstance(proof, dict) and proof.get('tip_commit') == TIP
			and proof.get('candidate_commit') == CANDIDATE
			and all(isinstance(proof.get(key), str) and re.fullmatch('[0-9a-f]{64}', proof[key]) for key in
					('hot_closure_sha256', 'native_source_sha256', 'accounting_sha256')),
			'missing live E1a source/accounting proof')
	spec = json.loads((root / 'SPEC.json').read_text())
	callgrind_packet.declared(spec)
	require(spec['symbol_map'] is None and 'qualifications' not in spec, 'unexpected inherited qualification')
	def read(row):
		return callgrind.parse(root / row['profile'], spec['positions'])
	def checksum(row, expected):
		require((root / row['stdout']).read_text().splitlines()[0] ==
				f'CHECKSUM {expected} invocations=1000', 'live checksum differs')
	def guard(paths):
		return {key: str(root / path) for key, path in paths.items()}
	controls = {}
	for name, rows in spec['controls']['profiles'].items():
		for label, row in rows.items():
			value = 0 if label.endswith('0') and not label.endswith('100') else (300 if name == 'is' else 150)
			checksum(row, gen_inputs.checksum([value] * 1000))
		controls[name] = {label: read(row) for label, row in rows.items()}
	deltas, _ = callgrind_rules.assess_controls(controls, guard(spec['controls']['guard']))
	validate_controls(deltas)
	result = {}
	for mode, phase in spec['phases'].items():
		cells, accounting = {}, {}
		for name, cell in phase['cells'].items():
			values = [int(x) for x in (root / cell['input_file']).read_text().splitlines()[:1000]]
			require(len(values) == 1000, 'incomplete input stream')
			expected = gen_inputs.checksum([gen_inputs.model(name, x) for x in values])
			for role in ('tip', 'candidate', 'reference'):
				checksum(cell[role], expected)
			before, after, reference = (read(cell[role]) for role in ('tip', 'candidate', 'reference'))
			adjusted, accounting[name] = qualify(before, after, mode + '/' + name, policy)
			cells[name] = dict(tip=before, candidate=adjusted, reference=reference,
							   iterations=sum(values) if name in ('while', 'numeric', 'calls') else None,
							   guard=guard(phase['guard']))
		# Run the existing analyzer, without changing any of its bounds or code.
		result[mode] = dict(assessment=callgrind_rules.assess_phase_with_controls(
			cells, controls, guard=guard(spec['controls']['guard'])), raw_accounting=accounting)
	return result


def seal(capture, repo, final_commit, accounting, output, check=None):
	"""Recheck immutable captured evidence, signed sources and the qualified replay."""
	import subprocess
	import analysis_package
	import analysis_sandbox
	import callgrind_capture
	import role_evidence
	import input_verification
	import measurement_readiness
	import measurement_runtime
	capture, repo, accounting = (Path(p).resolve(strict=True) for p in (capture, repo, accounting))
	context_path = capture / 'CAPTURE-CONTEXT.json'
	context = json.loads(context_path.read_text())
	require(context['status'] == 'CAPTURED' and context['runs'] == 68, 'incomplete original capture')
	pins = []
	def pin(path):
		observed = role_evidence._pin(Path(path).resolve(strict=True)); pins.append(observed); return observed
	pin(context_path); pin(capture / 'FAILURE.json')
	for item in [*context['input_pins'].values(), *context['generated_pins'].values()]:
		require(pin(item['path']) == item, 'original controller input changed')
	request = json.loads(Path(context['request_spec']['path']).read_text())
	require(pin(context['request_spec']['path']) == context['request_spec'], 'original request changed')
	roles = request['roles']
	host = context['host']
	require(input_verification.verify(context['inputs']) == context['input_proof'], 'input regeneration differs')
	require(measurement_readiness.require_ready(context['readiness']['observation'],
				max_load=host['measurement_max_load'], min_available_bytes=host['measurement_min_available_bytes'])
			== context['readiness'], 'original capture readiness differs')
	require(context['after_capture']['cpu'] == host['measurement_cpu'], 'capture CPU differs')
	require(measurement_runtime.tool_tree(Path(host['valgrind_root'])) == context['valgrind_tree'],
			'Valgrind installation differs')
	runtime_paths = [host['valgrind_path'], *(entry['binary']['path'] for entry in roles.values())]
	require(measurement_runtime.dependency_snapshot(runtime_paths) == context['runtime_dependencies'],
			'capture runtime dependencies differ')
	require(all(entry['commit'] == CANDIDATE for entry in roles.values() if entry['role'] == 'candidate'),
			'qualification candidate changed')
	require(all(entry['commit'] == TIP for entry in roles.values() if entry['role'] == 'tip'), 'tip changed')
	checked = {name: role_evidence.check(repo, role) for name, role in roles.items()}
	callgrind_capture.verify(capture, roles=roles, host=context['host'], inputs=context['inputs'], code=context['code_root'])
	for profile, manifest in context['analysis_manifests'].items():
		input_closure.verify(capture / profile, manifest['package'])
		input_closure.verify(Path(__file__).parent, manifest['code'])
		pin(capture / profile / 'SPEC.json')
	for filename in ('RUNS.json', 'RUNS.jsonl'):
		pin(capture / filename)
	def git(*args):
		return subprocess.check_output(['git', '-C', str(repo), *args])
	for revision in (TIP, CANDIDATE, final_commit):
		subprocess.run(['git', '-C', str(repo), 'verify-commit', revision], check=True, capture_output=True)
	allowed = {'plans/E1a-formatting-boundary.txt', 'tools/xtask/measure/qualifications/e1a.json',
			   'tools/xtask/measure/e1a_qualification.py', 'tools/xtask/measure/test_e1a_qualification.py'}
	changed = git('diff', '--name-only', CANDIDATE, final_commit).decode().splitlines()
	require(set(changed) <= allowed, 'production/harness/lock inputs changed since measured source')
	require(not git('diff', final_commit, '--', *sorted(allowed)), 'qualification implementation is uncommitted')
	def source(revision, path):
		return git('show', revision + ':' + path)
	path = 'crates/rune/src/experimental_core/execute.rs'
	def hot(data):
		return data[data.index(b'\tfn invoke_args<'):data.index(b'\tfn invoke_native(')]
	before, after = (hot(source(revision, path)) for revision in (TIP, CANDIDATE))
	require(before == after, 'interpreter hot closure differs')
	native_paths = ['crates/rune/src/experimental_native', 'crates/rune/src/experimental_core/region']
	native = [git('ls-tree', '-r', revision, '--', *native_paths) for revision in (TIP, CANDIDATE)]
	require(native[0] == native[1] and native[0], 'native implementation differs')
	accounting_pin = pin(accounting)
	original_accounting = json.loads(accounting.read_text())
	require(original_accounting.get('candidate') == CANDIDATE and original_accounting.get('tip') == TIP,
			'accounting source binding differs')
	facts = dict(tip_commit=TIP, candidate_commit=CANDIDATE,
				 hot_closure_sha256=hashlib.sha256(before).hexdigest(),
				 native_source_sha256=hashlib.sha256(native[0]).hexdigest(), accounting_sha256=accounting_pin['sha256'])
	code = Path(__file__).parent
	manifest = context['analysis_manifests']['cgu1']
	code_manifest = input_closure.create(code, [row['path'] for row in manifest['code']['files']] +
										['e1a_qualification.py', 'qualifications/e1a.json'])
	analysis = json.loads(analysis_sandbox.run(capture / 'cgu1', manifest['package'], code, code_manifest,
											  'e1a_qualification.py', verified_facts=facts))
	observed = {row['cell']: row for row in original_accounting['rows']}
	require(set(observed) == {mode + '/' + name for mode in ('off', 'on') for name in KERNELS},
			'accounting inventory differs')
	for mode, phase in analysis.items():
		for name, row in phase['raw_accounting'].items():
			original = observed[mode + '/' + name]
			require(row['raw_core'] == original['core_candidate']
					and row['raw_differences'] == original['all_core_differences'],
					'replayed raw accounting differs')
	diagnostic_manifest = analysis_package.inspect(capture / 'cgu16', 'callgrind-cgu16-v1')
	diagnostic = analysis_package.analyze(capture / 'cgu16', diagnostic_manifest)
	packet = dict(schema=1, status='PASS', record='E1a', qualification=POLICY, measured_candidate=CANDIDATE,
				  final_commit=final_commit, tooling_only_delta=changed, production_inputs_identical=True,
				  original_failure_preserved=True, pins=pins, roles=checked, proof=facts,
				  analyzer_code=code_manifest, analysis=analysis, cgu16_diagnostic_only=diagnostic)
	require(input_closure.create(code, [row['path'] for row in code_manifest['files']]) == code_manifest,
			'qualified analyzer implementation changed during replay')
	for item in pins:
		require(role_evidence._pin(Path(item['path'])) == item, 'evidence changed during replay')
	if check is not None:
		require(json.loads(Path(check).read_text()) == packet, 'qualified replay seal changed')
	Path(output).write_text(json.dumps(packet, indent=2, allow_nan=False) + '\n')
	print('PASS qualified E1a replay; receipt', output, flush=True)


if __name__ == '__main__':
	if len(sys.argv) == 2:
		print(json.dumps(analyze('/inputs', json.loads(Path(sys.argv[1]).read_text())), allow_nan=False))
	else:
		import argparse
		parser = argparse.ArgumentParser(description=__doc__)
		for name in ('capture', 'repo', 'final-commit', 'accounting', 'output'):
			parser.add_argument('--' + name, required=True)
		parser.add_argument('--check')
		seal(**vars(parser.parse_args()))
