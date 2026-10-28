"""Controls for joint resampling of fresh measurement rounds."""
from pathlib import Path
import random
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import bootstrap


class BootstrapTests(unittest.TestCase):
    def test_paired_resampling_and_fixed_seed(self):
        series = {'base': [1, 2, 3, 4], 'candidate': [2, 4, 6, 8]}
        run = lambda: bootstrap.joint_bootstrap(
            series, 4, random.Random(17), [('ratio', 'candidate', 'base')],
            [('difference', 'candidate', 'base')], keep=True)
        result, audit = run()
        self.assertEqual((result, audit), run())
        self.assertEqual(result['ratio']['ratio'], 2)
        self.assertEqual(result['ratio']['ci95'], [2, 2])
        self.assertEqual(len(audit['idx']), bootstrap.B)
        self.assertEqual(result['difference']['difference'], 2.5)

    def test_invalid_denominators_and_rounds_fail_closed(self):
        result, _ = bootstrap.joint_bootstrap(
            {'a': [1, 1], 'b': [0, 0]}, 2, random.Random(3), [('ratio', 'a', 'b')])
        self.assertIn('unresolved', result['ratio'])
        result, _ = bootstrap.joint_bootstrap(
            {'a': [1], 'b': [1]}, 1, random.Random(3), [('ratio', 'a', 'b')])
        self.assertIn('unresolved', result['ratio'])
        with self.assertRaises(ValueError):
            bootstrap.joint_bootstrap({'a': [1], 'b': [1, 2]}, 2, random.Random(3), [])


if __name__ == '__main__':
    unittest.main()
