"""K1 packet controls use synthetic raw samples and execution receipts."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_sandbox
import counter_cell
import counter_packet
import input_closure
import k1_packet
import perf_parse


class K1PacketTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'packet'
        self.root.mkdir()
        self.tools = Path(__file__).resolve().parent
        self.code = input_closure.create(self.tools, ['k1_packet.py', 'k1.py', 'counter_cell.py', 'counter_packet.py',
                                                      'counter_schedule.py', 'counter_assess.py', 'perf_parse.py', 'input_closure.py'])
        names = [f'{role}-{mode}' for role in ('candidate', 'milestone', 'tip') for mode in ('off', 'on')]
        self.spec = {'profile': 'cgu16', 'cpu': 7, 'pins': {}, 'binaries': {n: '/logical/' + n for n in names},
                     'logical_session': '/logical/session', 'logical_program': '/logical/k1.rn',
                     'logical_input': '/logical/k1.inputs',
                     'environment': {'PATH': '/usr/bin:/bin', 'HOME': '/logical/home', 'LANG': 'C.UTF-8'}}
        self.perf = b'1000;;cpu_core/instructions/u;10;100.00;;\n1000;;cpu_core/cpu-cycles/u;10;100.00;;\n'
        self.parsed = perf_parse.parse(self.perf.decode(), 'B1', counter_cell.EVENTS, counter_cell.GROUPS)

    def build_records(self):
        session = Path(self.spec['logical_session'])
        rows = [{'record': 'header', 'cpu': 7, 'profile': self.spec['profile'], 'pins': {},
                 'perf_policy': {'perfconfig_absent': True, 'perf_event_paranoid': '2'}}]
        for repeat in range(5):
            for name in k1_packet.subjects(self.spec['binaries']):
                prefix = f'{name}-stat-{repeat}'
                for suffix, content in (('.perf', self.perf), ('.stdout', b'CHECKSUM 0 invocations=1\nMAPS anon_exec_mappings=0 anon_exec_kib=0\n'), ('.stderr', b'')):
                    (self.root / (prefix + suffix)).write_bytes(content)
                child = ['/usr/bin/env', '-u', 'PERF_BUILDID_DIR', self.spec['binaries'][name],
                         self.spec['logical_program'], '1', self.spec['logical_input'], '-']
                argv = counter_packet._perf_argv(session / (prefix + '.perf'), child)
                execution = {'argv': argv, 'cwd': str(session), 'env': self.spec['environment'], 'cpu': 7,
                             'status': 0, 'seconds': 0.1, 'timeout_s': 120,
                             'stdout': str(session / (prefix + '.stdout')), 'stderr': str(session / (prefix + '.stderr'))}
                rows.append({'record': 'sample', 'name': name, 'repeat': repeat, 'argv': argv,
                             'execution': execution, 'counters': self.parsed})
        rows.append({'record': 'footer', 'complete': True})
        (self.root / 'SPEC.json').write_text(json.dumps(self.spec))
        (self.root / 'records.jsonl').write_text(''.join(json.dumps(row) + '\n' for row in rows))

    def sandbox(self):
        pin = input_closure.create(self.root, k1_packet.declared(self.spec))
        return json.loads(analysis_sandbox.run(self.root, pin, self.tools, self.code, 'k1_packet.py'))

    def test_milestone_packet_is_reparsed(self):
        self.build_records()
        result = self.sandbox()
        self.assertEqual(result['ratios']['on']['comparisons']['milestone']['instructions'], '1')
        self.assertIsNone(result['ratios']['on']['tier1_pass'])
        (self.root / 'milestone-on-stat-0.stdout').write_text('bad')
        with self.assertRaisesRegex(ValueError, 'analyzer failed'):
            self.sandbox()

    def test_tier1_without_milestone_and_incomplete_pair(self):
        self.spec['profile'] = 'cgu1'
        self.spec['binaries'] = {k: v for k, v in self.spec['binaries'].items() if not k.startswith('milestone')}
        self.build_records()
        self.assertTrue(self.sandbox()['ratios']['off']['tier1_pass'])
        self.spec['binaries']['milestone-off'] = '/logical/milestone-off'
        with self.assertRaisesRegex(ValueError, 'both milestone'):
            k1_packet.declared(self.spec)


if __name__ == '__main__':
    unittest.main()
