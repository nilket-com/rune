"""K1 closure controls using synthetic current-run counter profiles."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_sandbox
import input_closure
import k1_analysis


class K1AnalysisTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'package'
        self.root.mkdir()
        self.tools = Path(__file__).resolve().parent
        self.code = input_closure.create(self.tools, ['k1_analysis.py', 'k1.py', 'counter_cell.py',
                                                      'counter_assess.py', 'perf_parse.py', 'input_closure.py'])
        self.spec = {'samples': {}, 'tier1': True}
        for role in ('candidate', 'tip', 'milestone'):
            for config in ('on', 'off'):
                subject = f'{role}-{config}'
                self.spec['samples'][subject] = []
                for i in range(5):
                    name = f'{subject}-{i}.perf'
                    self.spec['samples'][subject].append(name)
                    (self.root / name).write_text('1000;;cpu_core/instructions/u;10;100.00;;\n'
                                                  '1000;;cpu_core/cpu-cycles/u;10;100.00;;\n')

    def sandbox(self):
        (self.root / 'SPEC.json').write_text(json.dumps(self.spec))
        pin = input_closure.create(self.root, k1_analysis.declared(self.spec))
        return json.loads(analysis_sandbox.run(self.root, pin, self.tools, self.code, 'k1_analysis.py'))

    def test_required_subjects_only(self):
        self.spec['samples'] = {k: v for k, v in self.spec['samples'].items() if not k.startswith('milestone')}
        result = self.sandbox()
        self.assertTrue(result['ratios']['on']['tier1_pass'])
        self.assertNotIn('milestone', result['ratios']['on']['comparisons'])

    def test_milestones_and_missing_profile(self):
        result = self.sandbox()
        self.assertEqual(result['ratios']['off']['comparisons']['milestone']['instructions'], '1')
        absent = self.spec['samples']['milestone-on'][0]
        omitted = input_closure.create(self.root, [name for name in k1_analysis.declared(self.spec)
                                                   if name != absent])
        with self.assertRaisesRegex(ValueError, 'analyzer failed'):
            analysis_sandbox.run(self.root, omitted, self.tools, self.code, 'k1_analysis.py')
        (self.root / absent).unlink()
        with self.assertRaises(ValueError):
            self.sandbox()


if __name__ == '__main__':
    unittest.main()
