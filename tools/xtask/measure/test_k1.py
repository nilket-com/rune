"""K1 exact-threshold and input-coverage controls."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import k1


def samples(candidate=201, tip=200):
    return {f'{role}-{config}': {'instructions': [value] * 5, 'cycles': [value * 2] * 5}
            for role, value in [('candidate', candidate), ('tip', tip)]
            for config in ('on', 'off')}


class K1Tests(unittest.TestCase):
    def test_exact_threshold(self):
        result = k1.summarize(samples(), tier1=True)
        self.assertEqual(result['ratios']['on']['comparisons']['tip']['instructions'], '201/200')
        self.assertTrue(result['ratios']['on']['tier1_pass'])
        self.assertFalse(k1.summarize(samples(candidate=202), tier1=True)['ratios']['on']['tier1_pass'])
        self.assertIsNone(k1.summarize(samples())['ratios']['on']['tier1_pass'])

    def test_missing_or_invalid_samples_fail(self):
        data = samples()
        data['candidate-on']['instructions'] = [1] * 4
        with self.assertRaises(ValueError):
            k1.summarize(data)
        data = samples()
        data['tip-on']['cycles'][0] = 0
        with self.assertRaises(ValueError):
            k1.summarize(data)
        data = samples()
        data['milestone-on'] = data['tip-on']
        with self.assertRaises(ValueError):
            k1.summarize(data)


if __name__ == '__main__':
    unittest.main()
