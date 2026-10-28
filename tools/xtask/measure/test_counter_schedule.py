"""Deterministic schedule controls over synthetic input metadata."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import counter_schedule


class CounterScheduleTests(unittest.TestCase):
    def setUp(self):
        self.manifest = {'cells': {kernel + '/' + stream: {'k1': 2, 'k2': 7}
                                   for kernel, stream in counter_schedule.KERNELS}}

    def test_exact_schedule_and_repeat_set(self):
        rows = counter_schedule.workload(123, self.manifest)
        self.assertEqual(rows, counter_schedule.workload(123, self.manifest))
        self.assertNotEqual(rows, counter_schedule.workload(124, self.manifest))
        self.assertEqual(len(rows), 150)
        self.assertEqual(len({(row['subject'], row['kernel'], row['repeat']) for row in rows}), 150)
        self.assertEqual(len(counter_schedule.admissions(self.manifest)), 30)
        self.assertTrue(all(row['k'] == 7 for row in rows))

    def test_rejects_missing_and_invalid_input_counts(self):
        with self.assertRaises(ValueError):
            counter_schedule.workload(-1, self.manifest)
        self.manifest['cells']['fib/fib_main']['k2'] = True
        with self.assertRaises(ValueError):
            counter_schedule.workload(1, self.manifest)
        self.manifest['cells']['fib/fib_main']['k1'] = 0
        with self.assertRaises(ValueError):
            counter_schedule.admissions(self.manifest)


if __name__ == '__main__':
    unittest.main()
