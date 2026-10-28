"""S8 qualifications must not admit unproved or broader regressions."""
import copy
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import callgrind_rules
import qualification_policy as policy


def profile(costs):
    return {'total_Ir': sum(costs.values()), 'functions': {
        name: {'self_Ir': cost, 'calls': 1000} for name, cost in costs.items()}}


class QualificationTests(unittest.TestCase):
    def setUp(self):
        self.qual = json.loads((Path(__file__).parent / 'qualifications/s8.json').read_text())
        self.qual['candidate_commit'] = 'b' * 40
        self.proof = {'schema': 1, 'tip_commit': 'a' * 40, 'candidate_commit': 'b' * 40,
                      'control_bytecode_equal': True, 'indexing_source_equal': True, 'entry_source_equal': True,
                      'control_bytecode_sha256': 'c' * 64}
        self.zero = {'core': 0, 'whole': 0}

    def test_policy_and_live_proof_are_required(self):
        self.assertEqual(policy.validate(self.qual, self.proof), self.qual)
        for field in ('control_bytecode_equal', 'indexing_source_equal', 'entry_source_equal'):
            bad = dict(self.proof, **{field: False})
            with self.assertRaisesRegex(ValueError, 'proof'):
                policy.validate(self.qual, bad)
        bad = copy.deepcopy(self.qual)
        bad['entries'][1]['max_ir_per_call'] = 2
        with self.assertRaisesRegex(ValueError, 'unreviewed'):
            policy.validate(bad, self.proof)
        roles = {'tip-cgu1-off': {'commit': 'a' * 40}, 'candidate-cgu1-off': {'commit': 'b' * 40}}
        policy.bind_roles(self.proof, roles)
        with self.assertRaisesRegex(ValueError, 'measured roles'):
            policy.bind_roles(dict(self.proof, candidate_commit='d' * 40), roles)

    def test_record_and_milestone_must_match_the_request(self):
        policy.bind_record(self.qual, 'S8', 'M7', 'b' * 40)
        with self.assertRaisesRegex(ValueError, 'record'):
            policy.bind_record(self.qual, 'other', 'M7', 'b' * 40)
        with self.assertRaisesRegex(ValueError, 'milestone'):
            policy.bind_record(self.qual, 'S8', 'M6', 'b' * 40)

    def test_candidate_must_match_the_measurement(self):
        with self.assertRaisesRegex(ValueError, 'candidate'):
            policy.bind_record(self.qual, 'S8', 'M7', 'a' * 40)

    def test_controls_must_be_nonpositive_and_confined_to_named_symbol(self):
        low = profile({policy.CONTROL: 1000})
        high = profile({policy.CONTROL: 1100})
        lower = profile({policy.CONTROL: 1090})
        sample = {'tip0': low, 'tip100': high, 'candidate0': low, 'candidate100': lower}
        profiles = {'is': sample, 'branch': sample}
        controls = {'is': {'core': -10, 'whole': -10}, 'branch': {'core': -10, 'whole': -10}}
        self.assertTrue(policy.controls_qualified(profiles, controls))
        self.assertFalse(policy.controls_qualified(profiles, dict(controls, **{'is': {'core': 1, 'whole': 1}})))
        self.assertFalse(policy.controls_qualified(profiles, dict(controls, **{'is': {'core': -1, 'whole': 1}})))
        changed = profile({policy.CONTROL: 1090, 'experimental_core::other': 1})
        self.assertFalse(policy.controls_qualified(dict(profiles, **{'is': dict(sample, candidate100=changed)}), controls))

    def test_per_iteration_exception_is_named_bounded_and_cell_specific(self):
        row = {'function': policy.INDEX, 'tip_calls': 11000, 'candidate_calls': 11000, 'delta': 11000}
        self.assertTrue(policy.iteration_qualified(row, 'off/shared_rows'))
        self.assertFalse(policy.iteration_qualified(row, 'off/while'))
        self.assertFalse(policy.iteration_qualified(dict(row, delta=11001), 'off/shared_rows'))
        self.assertFalse(policy.iteration_qualified(dict(row, candidate_calls=10999), 'off/shared_rows'))
        self.assertFalse(policy.iteration_qualified(dict(row, function='experimental_core::other'), 'off/shared_rows'))

    def test_qualified_hot_growth_still_obeys_existing_cell_limit(self):
        tip = profile({policy.INDEX: 374000, 'experimental_core::rest': 30000000})
        candidate = profile({policy.INDEX: 385000, 'experimental_core::rest': 30000000})
        for item in (tip, candidate):
            item['functions'][policy.INDEX]['calls'] = 11000
        controls = {'is': self.zero, 'branch': self.zero}
        kwargs = dict(invocations=1000, iterations=None, controls=controls, guard=None,
                      allow_residue=False, allow_control_growth=True, qualification=self.qual,
                      control_qualified=True, cell='off/shared_rows')
        self.assertEqual(callgrind_rules._assess(tip, candidate, tip, **kwargs)['rule'], '(iv)')
        with self.assertRaisesRegex(ValueError, 'per-iteration'):
            callgrind_rules._assess(tip, candidate, tip, **dict(kwargs, cell='off/while'))
        oversized = profile({policy.INDEX: 385000, 'experimental_core::rest': 30050000})
        oversized['functions'][policy.INDEX]['calls'] = 11000
        with self.assertRaisesRegex(ValueError, 'core threshold'):
            callgrind_rules._assess(tip, oversized, tip, **kwargs)
        candidate['functions'][policy.INDEX]['self_Ir'] += 1
        candidate['total_Ir'] += 1
        with self.assertRaisesRegex(ValueError, 'per-iteration'):
            callgrind_rules._assess(tip, candidate, tip, **kwargs)

    def test_entry_setup_or_nonqualified_control_still_fails(self):
        tip = profile({'experimental_core::invoke_args': 10000})
        candidate = profile({'experimental_core::invoke_args': 10001})
        controls = {'is': {'core': -1, 'whole': -1}, 'branch': self.zero}
        with self.assertRaisesRegex(ValueError, 'zero controls'):
            callgrind_rules._assess(tip, candidate, tip, invocations=1000, iterations=None,
                                    controls=controls, guard=None, allow_residue=False,
                                    allow_control_growth=True, qualification=self.qual,
                                    control_qualified=True, cell='off/while')
        tip = profile({policy.CONTROL: 10000})
        candidate = profile({policy.CONTROL: 10001})
        with self.assertRaisesRegex(ValueError, 'zero controls'):
            callgrind_rules._assess(tip, candidate, tip, invocations=1000, iterations=None,
                                    controls=controls, guard=None, allow_residue=False,
                                    allow_control_growth=True, qualification=self.qual,
                                    control_qualified=True, cell='off/while')


if __name__ == '__main__':
    unittest.main()
