"""Exercise the complete capture/reparse path with synthetic subprocesses."""
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import counter_controller
import counter_schedule

PERF = b'1000;;cpu_core/instructions/u;10;100.00;;\n1000;;cpu_core/cpu-cycles/u;10;100.00;;\n'
OBSERVATION = {'cpu': 7, 'load': 0.0, 'mem_available_bytes': 10000, 'core_throttle_count': 0, 'busy': []}


class FakeLease:
    def __init__(self, bad=False, reject=False):
        self.bad = bad
        self.reject = reject

    def ready(self, **kwargs):
        return {'status': 'READY', 'observation': OBSERVATION}

    def execute(self, argv, **kwargs):
        perf_path = Path(argv[argv.index('-o') + 1])
        child = argv[argv.index('--') + 1:]
        perf = PERF
        if self.reject and len(child) > 3 and '/candidate-' in child[3]:
            perf = PERF.replace(b'1000;;cpu_core/instructions', b'1010;;cpu_core/instructions')
        perf_path.write_bytes(perf)
        if child[-1] == '/bin/true':
            stdout = b''
        elif child[-1].endswith('env_probe.py'):
            stdout = b'{"LANG": 5}'
        else:
            binary, kernel, k = child[3:6]
            maps = int(binary.endswith('-on') and kernel in ('while.rn', 'numeric.rn', 'calls.rn'))
            stdout = f'CHECKSUM 0 invocations={k}\nMAPS anon_exec_mappings={maps} anon_exec_kib={maps * 4}\n'.encode()
            if self.bad:
                stdout = b'bad checksum\n'
        Path(kwargs['stdout_path']).write_bytes(stdout)
        Path(kwargs['stderr_path']).write_bytes(b'')
        return {'argv': argv, 'cwd': str(kwargs['cwd']), 'env': kwargs['env'], 'cpu': 7,
                'status': 0, 'seconds': 0.01, 'timeout_s': kwargs['timeout'],
                'stdout': str(kwargs['stdout_path']), 'stderr': str(kwargs['stderr_path'])}


class ControllerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.repo, self.inputs = self.root / 'repo', self.root / 'inputs'
        self.repo.mkdir()
        self.inputs.mkdir()
        manifest = {'cells': {}, 'streams': {}}
        for kernel, stream in counter_schedule.KERNELS:
            data = b'0\n0\n'
            filename = stream + '.txt'
            (self.inputs / filename).write_bytes(data)
            manifest['streams'][stream] = {'path': filename, 'sha256': hashlib.sha256(data).hexdigest()}
            manifest['cells'][kernel + '/' + stream] = {'k1': 1, 'k2': 2, 'expected_checksum': {'1': 0, '2': 0}}
        (self.inputs / 'MANIFEST.json').write_text(json.dumps(manifest))
        self.roles = {}
        for name in counter_schedule.SUBJECTS:
            role, config = name.rsplit('-', 1)
            binary, build = self.root / name, self.root / (name + '.build')
            binary.write_text('binary')
            build.write_text('build')
            self.roles[name] = {'name': name, 'role': role, 'profile': 'cgu1', 'native': config == 'on',
                                'commit': {'candidate': 'a', 'tip': 'b', 'base': 'c'}[role] * 40,
                                'binary': {'path': str(binary)}, 'build': {'path': str(build)}}
        self.host = {'measurement_cpu': 7, 'measurement_lock': str(self.root / 'lock'),
                     'measurement_max_load': 1.5, 'measurement_min_available_bytes': 1024}
        self.host_config = self.root / 'hosts.json'
        self.host_config.write_text(json.dumps({'schema': 1, 'measurement': self.host}))
        self.counts = self.root / 'paths.json'
        self.counts.write_text('{"on":{},"off":{}}')
        self.bad = False
        self.reject = False
        @contextmanager
        def lease(lock, cpu):
            self.assertEqual((str(lock), cpu), (self.host['measurement_lock'], 7))
            yield FakeLease(self.bad, self.reject)
        self.patches = [patch.object(counter_controller.measurement_runtime, 'lease', lease),
                        patch.object(counter_controller.measurement_runtime, 'check_perf_policy',
                                     return_value={'perfconfig_absent': True, 'perf_event_paranoid': '2'}),
                        patch.object(counter_controller.measurement_readiness, 'snapshot', return_value=OBSERVATION),
                        patch.object(counter_controller.input_verification, 'verify',
                                     return_value={'seed': 1, 'closure': counter_controller.input_closure.create(self.inputs, ['MANIFEST.json'])}),
                        patch.object(counter_controller.role_evidence, 'check',
                                     side_effect=lambda repo, role: {'name': role['name'], 'role': role['role'],
                                         'commit': role['commit'], 'profile': role['profile'], 'source_tree': 'd' * 40})]
        for item in self.patches:
            item.start()
            self.addCleanup(item.stop)

    def test_complete_capture_is_independently_reparsed(self):
        out = self.root / 'capture'
        request = self.root / 'request.json'
        request.write_text('{}')
        request_pin = counter_controller._pin(request)
        result = counter_controller.run(self.repo, self.host, self.roles, self.inputs, 'a', out,
                                        host_config=self.host_config, path_counts=self.counts, request_spec=request_pin)
        self.assertEqual(result['status'], 'PASS')
        self.assertEqual(result['inputs'], str(self.inputs))
        self.assertEqual(result['request_spec'], request_pin)
        self.assertEqual(result['analysis']['workloads'], 150)
        self.assertEqual(len((out / 'session/records.jsonl').read_text().splitlines()), 191)
        self.assertEqual(len(result['analysis']['cells']), 10)

    def test_bad_workload_never_produces_passing_receipt(self):
        self.bad = True
        out = self.root / 'failed'
        with self.assertRaisesRegex(ValueError, 'admission or workload failed'):
            counter_controller.run(self.repo, self.host, self.roles, self.inputs, 'a', out, host_config=self.host_config)
        self.assertFalse((out / 'RESULT.json').exists())
        self.assertEqual(json.loads((out / 'FAILURE.json').read_text())['status'], 'FAIL')
        footer = json.loads((out / 'session/records.jsonl').read_text().splitlines()[-1])
        self.assertFalse(footer['complete'])

    def test_instruction_regression_is_reported_as_rejected(self):
        self.reject = True
        result = counter_controller.run(self.repo, self.host, self.roles, self.inputs, 'a', self.root / 'reject',
                                        host_config=self.host_config)
        self.assertEqual(result['status'], 'REJECT')

    def test_command_pins_a_relative_request_and_checks_host_digest(self):
        request = self.root / 'request.json'
        request.write_text(json.dumps({'schema': 1, 'roles': self.roles, 'inputs': str(self.inputs),
                                       'seed': 'a', 'path_counts': None}))
        digest = hashlib.sha256(self.host_config.read_bytes()).hexdigest()
        argv = ['counter_controller.py', '--repo', str(self.repo), '--spec', 'request.json',
                '--out', 'out', '--hosts', str(self.host_config), '--hosts-sha256', digest]
        original = Path.cwd()
        try:
            os.chdir(self.root)
            with patch.object(sys, 'argv', argv), patch.object(counter_controller, 'run',
                    return_value={'status': 'PASS', 'out': str(self.root / 'out')}) as run:
                counter_controller.main()
            self.assertEqual(run.call_args.kwargs['request_spec']['path'], str(request))
            argv[-1] = '0' * 64
            with patch.object(sys, 'argv', argv):
                with self.assertRaisesRegex(ValueError, 'host configuration differs'):
                    counter_controller.main()
        finally:
            os.chdir(original)


if __name__ == '__main__':
    unittest.main()
