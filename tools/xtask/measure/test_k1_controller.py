"""Exercise K1 capture and independent reparse with synthetic subprocesses."""
from contextlib import contextmanager
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import k1_controller

PERF = b'1000;;cpu_core/instructions/u;10;100.00;;\n1000;;cpu_core/cpu-cycles/u;10;100.00;;\n'
OBSERVATION = {'cpu': 7, 'load': 0.0, 'mem_available_bytes': 10000, 'core_throttle_count': 0, 'busy': []}


class FakeLease:
    def __init__(self, *, bad=False, reject=False):
        self.bad, self.reject = bad, reject

    def ready(self, **kwargs):
        return {'status': 'READY', 'observation': OBSERVATION}

    def execute(self, argv, **kwargs):
        child = argv[argv.index('--') + 1:]
        perf = PERF.replace(b'1000;;cpu_core/instructions', b'1010;;cpu_core/instructions') if self.reject and '/candidate-' in child[3] else PERF
        Path(argv[argv.index('-o') + 1]).write_bytes(perf)
        stdout = b'bad\n' if self.bad else b'CHECKSUM 0 invocations=1\nMAPS anon_exec_mappings=0 anon_exec_kib=0\n'
        Path(kwargs['stdout_path']).write_bytes(stdout)
        Path(kwargs['stderr_path']).write_bytes(b'')
        return {'argv': argv, 'cwd': str(kwargs['cwd']), 'env': kwargs['env'], 'cpu': 7,
                'status': 0, 'seconds': 0.01, 'timeout_s': kwargs['timeout'],
                'stdout': str(kwargs['stdout_path']), 'stderr': str(kwargs['stderr_path'])}


class K1ControllerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.roles = {}
        for role in ('candidate', 'tip', 'milestone'):
            for mode in ('off', 'on'):
                name = role + '-' + mode
                binary, build = self.root / name, self.root / (name + '.build')
                binary.write_text('binary')
                build.write_text('build')
                self.roles[name] = {'name': name, 'role': role, 'profile': 'cgu16', 'native': mode == 'on',
                                    'commit': {'candidate': 'a', 'tip': 'b', 'milestone': 'c'}[role] * 40,
                                    'binary': {'path': str(binary)}, 'build': {'path': str(build)}}
        self.host = {'measurement_cpu': 7, 'measurement_lock': str(self.root / 'lock'),
                     'measurement_max_load': 1.5, 'measurement_min_available_bytes': 1024}
        self.host_config = self.root / 'hosts.json'
        self.host_config.write_text(json.dumps({'schema': 1, 'measurement': self.host}))
        self.bad = self.reject = False
        @contextmanager
        def lease(lock, cpu):
            yield FakeLease(bad=self.bad, reject=self.reject)
        patches = [patch.object(k1_controller.measurement_runtime, 'lease', lease),
                   patch.object(k1_controller.measurement_runtime, 'check_perf_policy',
                                return_value={'perfconfig_absent': True, 'perf_event_paranoid': '2'}),
                   patch.object(k1_controller.measurement_readiness, 'snapshot', return_value=OBSERVATION),
                   patch.object(k1_controller.role_evidence, 'check',
                                side_effect=lambda repo, role: {'name': role['name'], 'role': role['role'],
                                    'commit': role['commit'], 'profile': role['profile'], 'source_tree': 'd' * 40})]
        for item in patches:
            item.start()
            self.addCleanup(item.stop)

    def test_milestone_capture_is_reparsed(self):
        out = self.root / 'capture'
        result = k1_controller.run(self.repo, self.host, self.roles, 'cgu16', out, host_config=self.host_config)
        self.assertEqual(result['status'], 'PASS')
        self.assertEqual(result['code_root'], str(Path(k1_controller.__file__).parent))
        self.assertEqual(len((out / 'records.jsonl').read_text().splitlines()), 32)
        self.assertEqual(result['analysis']['ratios']['on']['comparisons']['milestone']['instructions'], '1')

    def test_bad_sample_has_failure_receipt(self):
        self.bad = True
        out = self.root / 'failed'
        with self.assertRaisesRegex(ValueError, 'K1 sample failed'):
            k1_controller.run(self.repo, self.host, self.roles, 'cgu16', out, host_config=self.host_config)
        self.assertEqual(json.loads((out / 'FAILURE.json').read_text())['status'], 'FAIL')
        self.assertFalse((out / 'RESULT.json').exists())

    def test_tier1_instruction_regression_is_rejected(self):
        self.reject = True
        roles = {name: dict(role, profile='cgu1') for name, role in self.roles.items() if not name.startswith('milestone')}
        result = k1_controller.run(self.repo, self.host, roles, 'cgu1', self.root / 'reject', host_config=self.host_config)
        self.assertEqual(result['status'], 'REJECT')


if __name__ == '__main__':
    unittest.main()
