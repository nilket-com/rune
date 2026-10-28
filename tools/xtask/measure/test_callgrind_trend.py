"""CGU16 trend sandbox controls with synthetic current-run inputs."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_sandbox
import callgrind_rules
import callgrind_trend
import gen_inputs
import input_closure


class TrendTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'package'
        self.root.mkdir()
        self.tools = Path(__file__).resolve().parent
        self.code = input_closure.create(self.tools, ['callgrind_trend.py', 'callgrind_rules.py', 'qualification_policy.py', 'callgrind.py', 'gen_inputs.py', 'input_closure.py'])
        self.spec = {'positions': 'line', 'symbol_map': None, 'phases': {}}
        (self.root / 'inputs').write_text('10\n' * 1000)
        for mode in ('on', 'off'):
            cells = {}
            for kernel in callgrind_rules.KERNELS:
                cell = {}
                for role in ('tip', 'candidate', 'milestone'):
                    name = f'{role}-{mode}-{kernel}'
                    value = 101 if role == 'candidate' else 100
                    (self.root / (name + '.cg')).write_text('positions: line\nevents: Ir\n'
                        f'fn=(1) experimental_core::work\n1 {value}\nsummary: {value}\n')
                    checksum = gen_inputs.checksum([gen_inputs.model(kernel, 10)] * 1000)
                    (self.root / (name + '.stdout')).write_text(f'CHECKSUM {checksum} invocations=1000\n')
                    cell[role] = {'profile': name + '.cg', 'stdout': name + '.stdout'}
                cell['input_file'] = 'inputs'
                cells[kernel] = cell
            self.spec['phases'][mode] = cells

    def sandbox(self):
        (self.root / 'SPEC.json').write_text(json.dumps(self.spec))
        pin = input_closure.create(self.root, callgrind_trend.declared(self.spec))
        return json.loads(analysis_sandbox.run(self.root, pin, self.tools, self.code, 'callgrind_trend.py'))

    def test_complete_trends_are_calculated(self):
        result = self.sandbox()
        self.assertEqual(len(result['cells']), 10)
        self.assertEqual(result['cells'][0]['ratios']['milestone']['core'], 1.0)

    def test_symbol_map_and_milestone_are_required_when_declared(self):
        cell = self.spec['phases']['off']['while']
        original = cell['milestone']['profile']
        cell['milestone']['profile'] = cell['tip']['profile']
        with self.assertRaisesRegex(ValueError, 'capture files must be distinct'):
            callgrind_trend.declared(self.spec)
        cell['milestone']['profile'] = original
        self.spec['symbol_map'] = 'SYMBOL-NAME-MAP.json'
        with self.assertRaises(ValueError):
            self.sandbox()
        (self.root / 'SYMBOL-NAME-MAP.json').write_text('{}')
        self.sandbox()
        row = self.spec['phases']['on']['while']['milestone']
        (self.root / row['stdout']).write_text('CHECKSUM 5 invocations=1000\n')
        with self.assertRaisesRegex(ValueError, 'analyzer failed'):
            self.sandbox()


if __name__ == '__main__':
    unittest.main()
