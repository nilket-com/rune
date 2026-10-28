"""Boundary and fail-closed controls for counter decisions."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import counter_assess as counter


class CounterTests(unittest.TestCase):
    def test_instruction_threshold_is_exact(self):
        self.assertFalse(counter.assess(201, 200)['rejected'])
        self.assertTrue(counter.assess(202, 200)['rejected'])
        for num, den in ((0, 1), (1, 0), (True, 1), (1.0, 1)):
            with self.assertRaises(ValueError):
                counter.assess(num, den)

    def test_cycles_use_spread_and_strict_threshold(self):
        self.assertFalse(counter.assess_cycles([102] * 5, [100] * 5)['flagged'])
        self.assertTrue(counter.assess_cycles([103] * 5, [100] * 5)['flagged'])
        self.assertFalse(counter.assess_cycles([104] * 5, [98, 99, 100, 101, 102])['flagged'])
        for bad in ([1] * 4, [1, 1, 1, 1, 0], [1, 1, 1, 1, float('nan')]):
            with self.assertRaises(ValueError):
                counter.assess_cycles(bad, [1] * 5)


if __name__ == '__main__':
    unittest.main()
