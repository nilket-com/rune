"""Capture all callgrind phases using synthetic child profiles."""
from contextlib import contextmanager
import hashlib
import json
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import callgrind_controller
import callgrind_capture
import gen_inputs
import input_closure

OBSERVATION = {'cpu': 7, 'load': 0.0, 'mem_available_bytes': 10000, 'core_throttle_count': 0, 'busy': []}
PROFILE = 'positions: instr line\nevents: Ir\nfn=(1) experimental_core::work\n0x1 1 10000\nsummary: 10000\n'


class FakeLease:
    def __init__(self, bad=False):
        self.bad = bad

    def ready(self, **kwargs):
        return {'status': 'READY', 'observation': OBSERVATION, 'limits': kwargs}

    def execute(self, argv, **kwargs):
        profile = next(value.split('=', 1)[1] for value in argv if value.startswith('--callgrind-out-file='))
        Path(profile).write_text(PROFILE)
        program, stream = Path(argv[-4]).stem, Path(argv[-2])
        xs = [int(line) for line in stream.read_text().splitlines()[:1000]]
        if program == 'control_is':
            values = [0 if x == 0 else 300 for x in xs]
        elif program == 'control_branch':
            values = [0 if x == 0 else 150 for x in xs]
        else:
            values = [gen_inputs.model(program, x) for x in xs]
        checksum = gen_inputs.checksum(values)
        stdout = 'bad\n' if self.bad else f'CHECKSUM {checksum} invocations=1000\nMAPS anon_exec_mappings=0 anon_exec_kib=0\n'
        Path(kwargs['stdout_path']).write_text(stdout)
        Path(kwargs['stderr_path']).write_bytes(b'')
        return {'argv': argv, 'cwd': str(kwargs['cwd']), 'env': kwargs['env'], 'cpu': 7,
                'status': 0, 'seconds': 0.01, 'timeout_s': kwargs['timeout'],
                'stdout': str(kwargs['stdout_path']), 'stderr': str(kwargs['stderr_path'])}


class CallgrindControllerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.repo, self.inputs = self.root / 'repo', self.root / 'inputs'
        self.repo.mkdir()
        self.inputs.mkdir()
        manifest = {'streams': {}}
        for stream in set(callgrind_controller.STREAMS.values()):
            data = b'0\n' * 1000
            name = stream + '.txt'
            (self.inputs / name).write_bytes(data)
            manifest['streams'][stream] = {'path': name, 'sha256': hashlib.sha256(data).hexdigest()}
        (self.inputs / 'MANIFEST.json').write_text(json.dumps(manifest))
        self.roles = {}
        for name in callgrind_controller.required_roles():
            role, profile, mode = name.split('-')
            binary, build, source = self.root / name, self.root / (name + '.build'), self.root / (name + '-source')
            binary.write_text('binary')
            hot = source / 'crates/rune/src/experimental_core/heap/hot.rs'
            hot.parent.mkdir(parents=True)
            hot.write_text('source')
            build.write_text(json.dumps({'source': {'source': str(source)}}))
            self.roles[name] = {'name': name, 'role': role, 'profile': profile, 'native': mode == 'on',
                                'commit': {'candidate': 'a', 'tip': 'b', 'base': 'c', 'milestone': 'e'}[role] * 40,
                                'binary': {'path': str(binary)}, 'build': {'path': str(build)}}
        valgrind_root = self.root / 'valgrind'
        valgrind_root.mkdir()
        valgrind = valgrind_root / 'valgrind'
        valgrind.write_text('synthetic valgrind')
        self.host = {'measurement_cpu': 7, 'measurement_lock': str(self.root / 'lock'),
                     'measurement_max_load': 1.5, 'measurement_min_available_bytes': 1024,
                     'valgrind_path': str(valgrind), 'valgrind_root': str(valgrind_root), 'valgrind_identity': 'valgrind test'}
        self.host_config = self.root / 'hosts.json'
        self.host_config.write_text(json.dumps({'schema': 1, 'measurement': self.host}))
        self.symbol_map = self.root / 'symbols.json'
        self.symbol_map.write_text('{}')
        self.bad = False
        @contextmanager
        def lease(lock, cpu):
            yield FakeLease(self.bad)
        patches = [patch.object(callgrind_controller.measurement_runtime, 'lease', lease),
                   patch.object(callgrind_controller.measurement_readiness, 'snapshot', return_value=OBSERVATION),
                   patch.object(callgrind_controller.input_verification, 'verify',
                                return_value={'seed': 1, 'closure': input_closure.create(self.inputs, ['MANIFEST.json'])}),
                   patch.object(callgrind_controller, 'valgrind_version', return_value=SimpleNamespace(returncode=0, stdout='valgrind test\n')),
                   patch.object(callgrind_controller.role_evidence, 'check',
                                side_effect=lambda repo, role: {'name': role['name'], 'role': role['role'],
                                    'commit': role['commit'], 'profile': role['profile'], 'source_tree': 'd' * 40})]
        for item in patches:
            item.start()
            self.addCleanup(item.stop)

    def test_all_profiles_are_captured_and_reparsed(self):
        out = self.root / 'capture'
        result = callgrind_controller.run(self.repo, self.host, self.roles, self.inputs, out,
                                          host_config=self.host_config, symbol_map=self.symbol_map)
        self.assertEqual(result['status'], 'PASS')
        self.assertEqual(result['inputs'], str(self.inputs))
        self.assertEqual(result['runs'], 68)
        self.assertIn(str(out / 'cgu1/guard/off/tip-binary'), result['generated_pins'])
        self.assertIn(str(self.inputs / 'loop_main.txt'), result['input_pins'])
        self.assertEqual(len(json.loads((out / 'RUNS.json').read_text())), 68)
        self.assertEqual(len(result['analysis']['cgu16']['cells']), 10)
        self.assertEqual(result['capture_receipt']['captures'], 68)
        records = out / 'RUNS.json'
        original = records.read_text()
        altered = json.loads(original)
        altered[0]['role'] = 'candidate-cgu1-off'
        records.write_text(json.dumps(altered))
        with self.assertRaisesRegex(ValueError, 'journal or role differs'):
            callgrind_capture.verify(out, roles=self.roles, host=self.host, inputs=self.inputs,
                                     code=Path(callgrind_controller.__file__).parent)
        records.write_text(original)
        profile = out / 'cgu1/tip-is-0.callgrind'
        profile.write_text('changed')
        with self.assertRaisesRegex(ValueError, 'raw capture changed'):
            callgrind_capture.verify(out, roles=self.roles, host=self.host, inputs=self.inputs,
                                     code=Path(callgrind_controller.__file__).parent)
        profile.write_text(PROFILE)
        (out / 'cgu1/guard/off/tip-binary').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'guard input differs'):
            callgrind_capture.verify(out, roles=self.roles, host=self.host, inputs=self.inputs,
                                     code=Path(callgrind_controller.__file__).parent)

    def test_bad_checksum_preserves_failure_receipt(self):
        self.bad = True
        out = self.root / 'failed'
        with self.assertRaisesRegex(ValueError, 'checksum differs'):
            callgrind_controller.run(self.repo, self.host, self.roles, self.inputs, out, host_config=self.host_config)
        self.assertEqual(json.loads((out / 'FAILURE.json').read_text())['status'], 'FAIL')
        self.assertFalse((out / 'RESULT.json').exists())
        self.assertEqual(len((out / 'RUNS.jsonl').read_text().splitlines()), 1)

    def test_analysis_failure_preserves_verified_capture_context(self):
        out = self.root / 'analysis-failed'
        with patch.object(callgrind_controller.analysis_package, 'analyze',
                          side_effect=ValueError('injected analyzer failure')):
            with self.assertRaisesRegex(ValueError, 'injected analyzer failure'):
                callgrind_controller.run(self.repo, self.host, self.roles, self.inputs, out,
                                         host_config=self.host_config)
        context = json.loads((out / 'CAPTURE-CONTEXT.json').read_text())
        self.assertEqual(context['status'], 'CAPTURED')
        self.assertEqual(context['readiness']['observation'], OBSERVATION)
        self.assertEqual(context['after_capture'], OBSERVATION)
        self.assertEqual(context['capture_receipt']['captures'], 68)
        self.assertEqual(set(context['analysis_manifests']), {'cgu1', 'cgu16'})
        self.assertEqual(len(json.loads((out / 'RUNS.json').read_text())), 68)
        self.assertFalse((out / 'RESULT.json').exists())
        self.assertEqual(json.loads((out / 'FAILURE.json').read_text())['status'], 'FAIL')

    def test_qualifications_require_fresh_proof_and_are_reported(self):
        qualifications = json.loads((Path(callgrind_controller.__file__).parent / 'qualifications/s8.json').read_text())
        qualifications['candidate_commit'] = 'a' * 40
        proof = {'schema': 1, 'tip_commit': 'b' * 40, 'candidate_commit': 'a' * 40,
                 'control_bytecode_equal': True, 'indexing_source_equal': True, 'entry_source_equal': True,
                 'control_bytecode_sha256': 'c' * 64}
        out = self.root / 'qualified'
        with patch.object(callgrind_controller.control_proof, 'prove', return_value=proof) as recompile:
            result = callgrind_controller.run(self.repo, self.host, self.roles, self.inputs, out,
                                              host_config=self.host_config, qualifications=qualifications,
                                              record='S8', baseline_milestone='M7')
        recompile.assert_called_once()
        self.assertEqual(result['analysis']['cgu1']['off']['qualifications'], qualifications['entries'])
        self.assertEqual(result['analysis']['cgu1']['off']['qualification_proof'], proof)
        with patch.object(callgrind_controller.control_proof, 'prove', return_value=dict(proof, entry_source_equal=False)):
            with self.assertRaisesRegex(ValueError, 'proof'):
                callgrind_controller.run(self.repo, self.host, self.roles, self.inputs, self.root / 'unproved',
                                         host_config=self.host_config, qualifications=qualifications,
                                         record='S8', baseline_milestone='M7')


if __name__ == '__main__':
    unittest.main()
