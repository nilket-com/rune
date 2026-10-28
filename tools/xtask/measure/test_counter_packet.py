"""Build complete synthetic counter packets for closed-input reparse controls."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_sandbox
import counter_cell
import counter_packet
import counter_schedule
import input_closure
import perf_parse


class CounterPacketTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'packet'
        self.root.mkdir()
        (self.root / 'session').mkdir()
        self.tools = Path(__file__).resolve().parent
        self.code = input_closure.create(self.tools, ['counter_packet.py', 'counter_schedule.py', 'counter_cell.py',
                                                     'counter_assess.py', 'input_closure.py', 'perf_parse.py'])
        self.spec = {'seed': 'a', 'cpu': 7, 'pins': {},
                     'binaries': {name: '/logical/bin/' + name for name in counter_schedule.SUBJECTS},
                     'logical_session': '/logical/session', 'logical_inputs': '/logical/inputs',
                     'logical_kernels': '/logical/kernels',
                     'environment': {'PATH': '/usr/bin:/bin', 'HOME': '/logical/home', 'LANG': 'C.UTF-8'},
                     'env_probe': '/logical/env_probe.py', 'path_counts': None}
        self.manifest = {'cells': {}, 'streams': {}}
        for kernel, stream in counter_schedule.KERNELS:
            self.manifest['cells'][kernel + '/' + stream] = {'k1': 1, 'k2': 2, 'expected_checksum': {'1': 0, '2': 0}}
            self.manifest['streams'][stream] = {'path': stream + '.txt'}
        schedule = counter_schedule.workload(10, self.manifest)
        (self.root / 'MANIFEST.json').write_text(json.dumps(self.manifest))
        (self.root / 'SCHEDULE.json').write_text(json.dumps(schedule))
        self.perf = b'1000;;cpu_core/instructions/u;10;100.00;;\n1000;;cpu_core/cpu-cycles/u;10;100.00;;\n'
        self.parsed = perf_parse.parse(self.perf.decode(), 'B1', counter_cell.EVENTS, counter_cell.GROUPS)
        self.records = [{'record': 'header', 'group': counter_packet.GROUP, 'cpu': 7, 'pins': {},
                         'perf_policy': {'perfconfig_absent': True, 'perf_event_paranoid': '2'},
                         'schedule_sha256': hashlib.sha256((self.root / 'SCHEDULE.json').read_bytes()).hexdigest(),
                         'expected_probes': 3, 'expected_environments': 6, 'expected_admissions': 30,
                         'expected_workloads': 150, 'child_timeout_s': 120, 'whole_run_bound_s': 4800}]
        for i in range(3):
            prefix = f'probe-{i}'
            self.raw(prefix, b'')
            child = ['/usr/bin/env', '-u', 'PERF_BUILDID_DIR', '/bin/true']
            self.records.append({'record': 'probe', 'index': i, 'rc': 0, 'argv': self.argv(prefix, child),
                                 'result': perf_parse.classify_probe(0, b'', b'', self.perf, 'B1', counter_cell.EVENTS, counter_cell.GROUPS),
                                 'execution': self.execution(prefix, child)})
        admissions = counter_schedule.admissions(self.manifest)
        for j, name in enumerate(counter_schedule.SUBJECTS):
            prefix = 'env-' + name
            env_stdout = b'{"LANG": 5}'
            self.raw(prefix, env_stdout)
            child = ['/usr/bin/env', '-u', 'PERF_BUILDID_DIR', '/usr/bin/python3', '-I', '-B', self.spec['env_probe']]
            self.records.append({'record': 'environment', 'subject': name, 'rc': 0, 'argv': self.argv(prefix, child),
                                 'perf': self.parsed, 'keys_and_lengths': {'LANG': 5}, 'execution': self.execution(prefix, child)})
            for ident in admissions[j * 5:(j + 1) * 5]:
                self.records.append(self.subject(ident, 'admission-' + name + '-' + ident['kernel'],
                                                 'admission', name + '/' + ident['kernel']))
        for i, ident in enumerate(schedule):
            self.records.append(self.subject(ident, f'row-{i:03}', 'row', i))
        self.records.append({'record': 'footer', 'complete': True, 'workloads': 150, 'stopped': None, 'traceback': None})
        (self.root / 'session/records.jsonl').write_text(''.join(json.dumps(row) + '\n' for row in self.records))

    def raw(self, prefix, stdout):
        for suffix, data in (('.perf', self.perf), ('.stdout', stdout), ('.stderr', b'')):
            (self.root / 'session' / (prefix + suffix)).write_bytes(data)

    def argv(self, prefix, child):
        return counter_packet._perf_argv(Path(self.spec['logical_session']) / (prefix + '.perf'), child)

    def execution(self, prefix, child):
        root = Path(self.spec['logical_session'])
        return {'argv': self.argv(prefix, child), 'cwd': self.spec['logical_kernels'], 'env': self.spec['environment'],
                'cpu': self.spec['cpu'], 'status': 0, 'seconds': 0.1, 'timeout_s': 120,
                'stdout': str(root / (prefix + '.stdout')), 'stderr': str(root / (prefix + '.stderr'))}

    def subject(self, ident, prefix, kind, key):
        role, kernel, stream, k = (ident[key] for key in ('subject', 'kernel', 'stream', 'k'))
        maps = int(role.endswith('-on') and kernel in ('numeric', 'while', 'calls'))
        stdout = f'CHECKSUM 0 invocations={k}\nMAPS anon_exec_mappings={maps} anon_exec_kib={maps * 4}\n'.encode()
        self.raw(prefix, stdout)
        child = ['/usr/bin/env', '-u', 'PERF_BUILDID_DIR', self.spec['binaries'][role], kernel + '.rn', str(k),
                 str(Path(self.spec['logical_inputs']) / (stream + '.txt')), '-']
        return {'record': kind, 'index': key, 'identity': ident, 'rc': 0, 'valid': True,
                'argv': self.argv(prefix, child), 'counters': self.parsed,
                'execution': self.execution(prefix, child),
                'perf_sha256': hashlib.sha256(self.perf).hexdigest(), 'stdout_sha256': hashlib.sha256(stdout).hexdigest()}

    def run_package(self):
        (self.root / 'SPEC.json').write_text(json.dumps(self.spec))
        pin = input_closure.create(self.root, counter_packet.declared(self.spec))
        return json.loads(analysis_sandbox.run(self.root, pin, self.tools, self.code, 'counter_packet.py'))

    def test_complete_packet_in_isolated_package(self):
        result = self.run_package()
        self.assertEqual((result['probes'], result['environments'], result['admissions'], result['workloads']), (3, 6, 30, 150))
        self.assertEqual(len(result['cells']), 10)
        self.assertFalse(result['cells'][0]['instruction_rule']['rejected'])

    def test_raw_drift_and_optional_path_counts(self):
        self.spec['path_counts'] = 'PATH-COUNTS.json'
        with self.assertRaises(ValueError):
            self.run_package()
        (self.root / 'PATH-COUNTS.json').write_text('{"on":{},"off":{}}')
        self.assertEqual(self.run_package()['cells'][0]['representative_path_counts'], {})
        records = self.root / 'session/records.jsonl'
        original = records.read_text()
        changed = [json.loads(line) for line in original.splitlines()]
        changed[40]['execution']['cpu'] = 8
        records.write_text(''.join(json.dumps(row) + '\n' for row in changed))
        with self.assertRaisesRegex(ValueError, 'analyzer failed'):
            self.run_package()
        records.write_text(original)
        (self.root / 'session/row-000.stdout').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'analyzer failed'):
            self.run_package()


if __name__ == '__main__':
    unittest.main()
