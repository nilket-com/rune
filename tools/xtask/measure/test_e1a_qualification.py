"""The one-record entry exception cannot hide a different regression."""
import copy
import json
from pathlib import Path
import unittest

import callgrind_rules
import e1a_qualification as e1a


def profiles(cell):
	functions = {e1a.INVOKE: dict(self_Ir=158000, calls=1000),
				 e1a.INIT: dict(self_Ir=115, calls=1),
				 'experimental_core::hot': dict(self_Ir=1800000, calls=100000),
				 'outside': dict(self_Ir=100000000, calls=1)}
	tip = dict(functions=functions, total_Ir=sum(row['self_Ir'] for row in functions.values()))
	candidate = copy.deepcopy(tip)
	for entry in e1a.POLICY['entries']:
		if cell in entry['cells']:
			delta = entry['cells'][cell]
			candidate['functions'][entry['symbol']]['self_Ir'] += delta
			candidate['total_Ir'] += delta
	return tip, candidate


class E1aTests(unittest.TestCase):
	def test_checked_in_policy_matches_the_named_review(self):
		self.assertEqual(json.loads((Path(__file__).parent / 'qualifications/e1a.json').read_text()), e1a.POLICY)

	def test_all_ten_exact_cells_leave_raw_profiles_unchanged(self):
		for mode in ('off', 'on'):
			for name in e1a.KERNELS:
				with self.subTest(mode=mode, name=name):
					cell = mode + '/' + name
					tip, candidate = profiles(cell)
					saved = copy.deepcopy(candidate)
					adjusted, accounting = e1a.qualify(tip, candidate, cell, e1a.POLICY)
					self.assertEqual(adjusted, tip)
					self.assertEqual(candidate, saved)
					self.assertEqual(accounting['raw_total_Ir'], candidate['total_Ir'])

	def test_unchanged_analyzer_rejects_raw_and_accepts_only_qualified_cost(self):
		tip, candidate = profiles('on/while')
		kwargs = dict(invocations=1000, iterations=None,
					  controls={key: {'core': 0, 'whole': 0} for key in ('is', 'branch')})
		with self.assertRaisesRegex(ValueError, 'bound'):
			callgrind_rules.assess(tip, candidate, tip, **kwargs)
		adjusted, _ = e1a.qualify(tip, candidate, 'on/while', e1a.POLICY)
		self.assertEqual(callgrind_rules.assess(tip, adjusted, tip, **kwargs)['core_change'], '0')

	def test_one_instruction_above_or_below_approved_cost_is_rejected(self):
		for delta in (-1, 1):
			tip, candidate = profiles('on/while')
			candidate['functions'][e1a.INVOKE]['self_Ir'] += delta
			candidate['total_Ir'] += delta
			with self.assertRaisesRegex(ValueError, 'exact entry'):
				e1a.qualify(tip, candidate, 'on/while', e1a.POLICY)

	def test_other_positive_core_cost_and_changed_call_counts_are_rejected(self):
		for mutation in ('hot', 'calls'):
			tip, candidate = profiles('on/while')
			if mutation == 'hot':
				candidate['functions']['experimental_core::hot']['self_Ir'] += 1
				candidate['total_Ir'] += 1
			else:
				candidate['functions'][e1a.INVOKE]['calls'] += 1
			with self.assertRaises(ValueError):
				e1a.qualify(tip, candidate, 'on/while', e1a.POLICY)

	def test_no_inheritance_wrong_cell_or_other_path_delta(self):
		tip, candidate = profiles('on/while')
		for changed in ('candidate_commit', 'tip_commit', 'record'):
			policy = copy.deepcopy(e1a.POLICY)
			policy[changed] = 'other'
			with self.assertRaisesRegex(ValueError, 'policy'):
				e1a.qualify(tip, candidate, 'on/while', policy)
		for cell in ('on/fib', 'off/while', 'on/unknown'):
			with self.assertRaises(ValueError):
				e1a.qualify(tip, candidate, cell, e1a.POLICY)

	def test_raw_whole_process_bound_is_not_adjusted(self):
		tip, candidate = profiles('on/while')
		candidate['total_Ir'] += 200000
		with self.assertRaisesRegex(ValueError, 'whole-process'):
			e1a.qualify(tip, candidate, 'on/while', e1a.POLICY)

	def test_whole_control_deltas_keep_the_existing_analyzer_policy(self):
		e1a.validate_controls({'is': {'core': 0, 'whole': 12}, 'branch': {'core': 0, 'whole': -10}})

	def test_entry_costs_must_cancel_in_core_controls(self):
		for delta in (-1, 1):
			with self.assertRaisesRegex(ValueError, 'core controls'):
				e1a.validate_controls({'is': {'core': delta, 'whole': 0}, 'branch': {'core': 0, 'whole': 0}})

	def test_missing_proof_is_rejected_before_profiles(self):
		with self.assertRaisesRegex(ValueError, 'proof'):
			e1a.analyze(Path('/does-not-exist'), {})


if __name__ == '__main__':
	unittest.main()
