"""Controls for the live measurement schedule; no historical results are required."""
import collections
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import protocol


class ScheduleTests(unittest.TestCase):
    def setUp(self):
        self.manifest = {'cells': {
            kernel + '/' + stream: {'k1': 3, 'k2': 7}
            for kernel, stream in protocol.KERNELS
        }}

    def test_fixed_seed_and_complete_balanced_schedule(self):
        rows = protocol.schedule(12345, self.manifest)
        self.assertEqual(rows, protocol.schedule(12345, self.manifest))
        self.assertNotEqual(rows, protocol.schedule(12346, self.manifest))
        self.assertEqual(len(rows), 240)
        counts = collections.Counter((r['kernel'], r['config'], r['endpoint']) for r in rows)
        self.assertEqual(set(counts.values()), {10})
        self.assertEqual(len(counts), 24)
        for index, row in enumerate(rows):
            self.assertEqual(row['round_index'], index)
            self.assertCountEqual([b['subject'] for b in row['blocks']], row['subjects'])
            for block in row['blocks']:
                self.assertCountEqual(block['order'], [1] if row['endpoint'] == 'cold' else [3, 7])

    def test_native_mapping_expectations(self):
        for kernel, _ in protocol.KERNELS:
            for subject in protocol.SUBJECTS:
                expected = int(subject in ('cand-on', 'base-on', 'base-on-prime')
                               and kernel in ('numeric', 'while', 'calls'))
                self.assertEqual(protocol.expected_maps(subject, kernel), expected)


if __name__ == '__main__':
    unittest.main()
