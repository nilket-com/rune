"""Structural rule controls use synthetic fresh profiles."""
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import callgrind_rules
import callgrind_guard


def profile(core=10000, whole=None, calls=1, name='experimental_core::work'):
    return {'total_Ir': core if whole is None else whole, 'functions': {name: {'self_Ir': core, 'calls': calls}}}


class RulesTests(unittest.TestCase):
    def setUp(self):
        self.controls = {'is': {'core': 0, 'whole': 0}, 'branch': {'core': 0, 'whole': 0}}

    def assess(self, tip, candidate, reference=None, **kwargs):
        return callgrind_rules.assess(tip, candidate, reference or tip, invocations=1000,
                                      iterations=100000, controls=self.controls, **kwargs)

    def test_stable_or_bounded_groups(self):
        self.assertEqual(self.assess(profile(), profile())['rule'], 'none')
        self.assertEqual(self.assess(profile(), profile(10005))['rule'], "(v')")
        with self.assertRaisesRegex(ValueError, 'bound'):
            self.assess(profile(), profile(10020))

    def test_growth_and_unported_exceptions_fail_closed(self):
        with self.assertRaisesRegex(ValueError, 'per-iteration'):
            self.assess(profile(calls=2000), profile(12000, calls=2000))
        with self.assertRaisesRegex(ValueError, 'guard proof'):
            self.assess(profile(name=callgrind_rules.ITER), profile(10001, name=callgrind_rules.ITER))
        with self.assertRaisesRegex(ValueError, 'residue proof'):
            self.assess(profile(10000, 10000), profile(10000, 10020))
        swap = next(iter(callgrind_rules.SWAP))
        with self.assertRaisesRegex(ValueError, 'swap group grows'):
            self.assess(profile(name=swap), profile(10001, name=swap))
        with self.assertRaisesRegex(ValueError, 'swap call count'):
            self.assess(profile(calls=3, name=swap), profile(9999, calls=3, name=swap))

    def test_residue_requires_all_five_cells_and_stable_installation(self):
        tip = profile(10000, 10000)
        candidate = profile(10000, 10200)
        candidate['functions']['<rune::compile::context::Context>::install_extra'] = {'self_Ir': 200, 'calls': 2}
        cells = {name: {'tip': tip, 'candidate': candidate, 'reference': tip, 'iterations': None}
                 for name in callgrind_rules.KERNELS}
        self.assertTrue(callgrind_rules.assess_phase(cells, controls=self.controls)['residue_proved'])
        with self.assertRaisesRegex(ValueError, 'exactly the five'):
            callgrind_rules.assess_phase(dict(list(cells.items())[:-1]), controls=self.controls)
        bad = dict(candidate, functions=dict(candidate['functions']))
        bad['functions']['external'] = {'self_Ir': 100, 'calls': 1000}
        cells['calls'] = dict(cells['calls'], candidate=bad)
        with self.assertRaisesRegex(ValueError, 'scales with invocations'):
            callgrind_rules.assess_phase(cells, controls=self.controls)

    def test_iterator_growth_requires_and_uses_fresh_proof(self):
        tip = profile(name=callgrind_rules.ITER)
        candidate = profile(10001, name=callgrind_rules.ITER)
        paths = {key: key for key in ('tip_source', 'candidate_source', 'tip_binary', 'candidate_binary')}
        with patch.object(callgrind_guard, 'prove', return_value={'proof': 'fresh'}) as check:
            result = callgrind_rules.assess(tip, candidate, tip, invocations=1000, iterations=100000,
                                            controls=self.controls, guard=paths)
        self.assertEqual(result['guard'], {'proof': 'fresh'})
        check.assert_called_once_with('tip_source', 'candidate_source', 'tip_binary', 'candidate_binary', candidate)

    def test_positive_control_requires_four_profile_guard(self):
        zero = profile(name=callgrind_rules.ITER)
        growth = profile(10001, name=callgrind_rules.ITER)
        normal = {'tip0': zero, 'tip100': zero, 'candidate0': zero, 'candidate100': zero}
        positive = dict(normal, candidate100=growth)
        profiles = {'is': positive, 'branch': normal}
        with self.assertRaisesRegex(ValueError, 'control growth requires'):
            callgrind_rules.assess_controls(profiles)
        paths = {key: key for key in ('tip_source', 'candidate_source', 'tip_binary', 'candidate_binary')}
        with patch.object(callgrind_guard, 'prove', return_value={'proof': 'fresh'}):
            values, proofs = callgrind_rules.assess_controls(profiles, paths)
        self.assertEqual(values['is']['core'], 1)
        self.assertEqual(proofs['is'], {'proof': 'fresh'})
        bad = dict(growth, functions=dict(growth['functions']))
        bad['functions']['experimental_core::extra'] = {'self_Ir': 1, 'calls': 1}
        profiles['is'] = dict(positive, candidate100=bad)
        with patch.object(callgrind_guard, 'prove', return_value={'proof': 'fresh'}):
            with self.assertRaisesRegex(ValueError, 'unpermitted other core'):
                callgrind_rules.assess_controls(profiles, paths)


if __name__ == '__main__':
    unittest.main()
