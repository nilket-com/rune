"""Callgrind parser controls over synthetic current-run input."""
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import callgrind

PROFILE = '''positions: instr line
events: Ir
fn=(1) caller
0x10 1 5
cfn=(2) callee
calls=3 0x20 2
0x10 1 99
fn=(2)
0x20 2 7
summary: 12
'''


class CallgrindTests(unittest.TestCase):
    def test_self_costs_and_incoming_calls(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'profile'
            path.write_text(PROFILE)
            self.assertEqual(callgrind.parse(path), {
                'total_Ir': 12,
                'functions': {'caller': {'self_Ir': 5, 'calls': 0},
                              'callee': {'self_Ir': 7, 'calls': 3}},
                'self_sum_verified': True,
            })
            for bad in (PROFILE.replace('summary: 12', 'summary: 13'),
                        PROFILE.replace('events: Ir', 'events: Dr'),
                        PROFILE.replace('cfn=(2) callee\n', '')):
                path.write_text(bad)
                with self.assertRaises(ValueError):
                    callgrind.parse(path)

    def test_line_positions(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'profile'
            path.write_text('positions: line\nevents: Ir\nfn=(1) main\n1 12\nsummary: 12\n')
            self.assertEqual(callgrind.parse(path, positions='line')['total_Ir'], 12)
            with self.assertRaises(ValueError):
                callgrind.parse(path)
            with self.assertRaises(ValueError):
                callgrind.parse(path, positions='unexpected')

    def test_projection_and_changed_core_functions(self):
        before = {'total_Ir': 7, 'self_sum_verified': True, 'functions': {
            'old::experimental_core::a': {'self_Ir': 3, 'calls': 1},
            'new::experimental_core::a': {'self_Ir': 4, 'calls': 2}}}
        projected = callgrind.project_symbols(before, {'old': 'new'})
        self.assertEqual(callgrind.core_total(projected), 7)
        self.assertEqual(projected['functions']['new::experimental_core::a'], {'self_Ir': 7, 'calls': 3})
        after = {'functions': {'new::experimental_core::a': {'self_Ir': 8, 'calls': 4}}}
        self.assertEqual(callgrind.core_differences(projected, after)[0]['delta'], 1)
        with self.assertRaises(ValueError):
            callgrind.project_symbols(dict(before, total_Ir=8), {})

    def test_exact_cost_changes(self):
        tip = {'total_Ir': 200, 'functions': {'experimental_core::a': {'self_Ir': 100, 'calls': 1}}}
        candidate = {'total_Ir': 198, 'functions': {'experimental_core::a': {'self_Ir': 101, 'calls': 1}}}
        result = callgrind.compare_cost(tip, candidate)
        self.assertEqual(result['core']['change'], '1/100')
        self.assertEqual(result['whole']['change'], '-1/100')
        with self.assertRaises(ValueError):
            callgrind.compare_cost(dict(tip, total_Ir=0), candidate)


if __name__ == '__main__':
    unittest.main()
