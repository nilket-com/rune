"""Closure branch controls with synthetic callgrind profiles."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_sandbox
import callgrind_cell
import input_closure


class CellTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'package'
        self.root.mkdir()
        self.tools = Path(__file__).resolve().parent
        self.code = input_closure.create(self.tools, ['callgrind.py', 'callgrind_cell.py', 'input_closure.py'])
        for name, value in [('tip.cg', 10), ('candidate.cg', 11)]:
            (self.root / name).write_text('positions: line\nevents: Ir\n'
                                          f'fn=(1) old::experimental_core::work\n1 {value}\nsummary: {value}\n')
        self.spec = {'tip': 'tip.cg', 'candidate': 'candidate.cg',
                     'positions': 'line', 'symbol_map': None}

    def sandbox(self):
        (self.root / 'SPEC.json').write_text(json.dumps(self.spec))
        pin = input_closure.create(self.root, callgrind_cell.declared(self.spec))
        return json.loads(analysis_sandbox.run(self.root, pin, self.tools, self.code, 'callgrind_cell.py'))

    def test_without_symbol_map(self):
        result = self.sandbox()
        self.assertEqual(result['cost']['core']['change'], '1/10')
        self.assertEqual(result['differences'][0]['function'], 'old::experimental_core::work')

    def test_symbol_map_is_required_when_selected(self):
        self.spec['symbol_map'] = 'SYMBOL-NAME-MAP.json'
        with self.assertRaises(ValueError):
            self.sandbox()
        (self.root / 'SYMBOL-NAME-MAP.json').write_text('{"old":"new"}')
        candidate = self.root / 'candidate.cg'
        candidate.write_text(candidate.read_text().replace('old::', 'new::'))
        result = self.sandbox()
        self.assertEqual(result['differences'][0]['function'], 'new::experimental_core::work')
        candidate.write_text(candidate.read_text().replace('new::', 'old::'))
        result = self.sandbox()
        self.assertEqual({item['function'] for item in result['differences']},
                         {'old::experimental_core::work', 'new::experimental_core::work'})
        (self.root / 'SPEC.json').write_text(json.dumps(self.spec))
        omitted = input_closure.create(self.root, ['SPEC.json', 'tip.cg', 'candidate.cg'])
        with self.assertRaisesRegex(ValueError, 'analyzer failed'):
            analysis_sandbox.run(self.root, omitted, self.tools, self.code, 'callgrind_cell.py')


if __name__ == '__main__':
    unittest.main()
