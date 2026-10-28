"""Counter closure branch controls using synthetic fresh counter profiles."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_sandbox
import counter_cell
import input_closure


class CounterCellTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'package'
        self.root.mkdir()
        self.tools = Path(__file__).resolve().parent
        self.code = input_closure.create(self.tools, ['counter_cell.py', 'counter_assess.py',
                                                      'perf_parse.py', 'input_closure.py'])
        self.spec = {'tip': [], 'candidate': [], 'config': 'on', 'path_counts': None}
        for role, instructions in [('tip', 1000), ('candidate', 1005)]:
            for i in range(5):
                name = f'{role}-{i}.perf'
                self.spec[role].append(name)
                (self.root / name).write_text(f'{instructions};;cpu_core/instructions/u;10;100.00;;\n'
                                              '1000;;cpu_core/cpu-cycles/u;10;100.00;;\n')

    def sandbox(self):
        (self.root / 'SPEC.json').write_text(json.dumps(self.spec))
        pin = input_closure.create(self.root, counter_cell.declared(self.spec))
        return json.loads(analysis_sandbox.run(self.root, pin, self.tools, self.code, 'counter_cell.py'))

    def test_without_path_counts(self):
        self.assertFalse(self.sandbox()['instruction_rule']['rejected'])

    def test_selected_path_counts_are_required(self):
        self.spec['path_counts'] = 'PATH-COUNTS.json'
        with self.assertRaises(ValueError):
            self.sandbox()
        (self.root / 'PATH-COUNTS.json').write_text('{"on":{"candidate":3}}')
        self.assertEqual(self.sandbox()['representative_path_counts'], {'candidate': 3})
        omitted = input_closure.create(self.root, ['SPEC.json', *self.spec['tip'], *self.spec['candidate']])
        with self.assertRaisesRegex(ValueError, 'analyzer failed'):
            analysis_sandbox.run(self.root, omitted, self.tools, self.code, 'counter_cell.py')


if __name__ == '__main__':
    unittest.main()
