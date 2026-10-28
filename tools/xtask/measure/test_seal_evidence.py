"""Seal evidence controls reanalyze synthetic fresh capture data."""
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import k1_controller
import counter_controller
import callgrind_controller
import role_evidence
import seal_evidence
import seal_store
import test_k1_controller
import test_counter_controller
import test_callgrind_controller


class SealEvidenceTests(unittest.TestCase):
    def setUp(self):
        fixture = test_k1_controller.K1ControllerTests('test_milestone_capture_is_reparsed')
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.fixture = fixture
        self.snapshot = fixture.root / 'snapshot'
        self.snapshot.mkdir()
        self.request = fixture.root / 'request.json'
        self.request.write_text(json.dumps({'schema': 1, 'profile': 'cgu16', 'roles': fixture.roles}))
        request_pin = role_evidence._pin(self.request)
        self.out = fixture.root / 'capture'
        k1_controller.run(fixture.repo, fixture.host, fixture.roles, 'cgu16', self.out,
                          host_config=fixture.host_config, request_spec=request_pin)
        self.spec = {'schema': 1, 'capture': str(self.out / 'RESULT.json'), 'signing_repo': str(fixture.repo),
                     'commit': 'a' * 40, 'native_source': str(self.snapshot), 'ignored_inputs': []}
        signed = {'status': 'PASS', 'commit': 'a' * 40, 'tree': 'd' * 40}
        patcher = patch.object(seal_evidence.source_export, 'verify_signed_snapshot', return_value=signed)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_fresh_analysis_does_not_use_saved_result_values(self):
        path = self.out / 'RESULT.json'
        capture = json.loads(path.read_text())
        capture['analysis'] = {'stored_value': 'not used as a baseline'}
        path.write_text(json.dumps(capture))
        result = seal_evidence.inspect(self.spec)
        self.assertEqual(result['status'], 'PASS')
        self.assertEqual(result['candidate_commit'], 'a' * 40)
        self.assertIn('on', result['assessment']['ratios'])

    def test_changed_raw_input_or_candidate_commit_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'candidate role'):
            seal_evidence.inspect(dict(self.spec, commit='e' * 40))
        packet = self.out / 'SPEC.json'
        original = packet.read_text()
        changed = json.loads(original)
        runtime = str(Path(k1_controller.__file__).resolve().parent / 'measurement_runtime.py')
        del changed['pins'][runtime]
        packet.write_text(json.dumps(changed))
        with self.assertRaisesRegex(ValueError, 'controller input declaration differs'):
            seal_evidence.inspect(self.spec)
        packet.write_text(original)
        (self.out / 'candidate-off-stat-0.stdout').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'analysis closure differs'):
            seal_evidence.inspect(self.spec)

    def test_seal_create_and_check_reanalyze_fresh_capture(self):
        spec_path = self.fixture.root / 'seal-spec.json'
        spec_path.write_text(json.dumps(self.spec))
        created = seal_store.execute('create', spec_path, self.fixture.root / 'sealed')
        self.assertEqual(created['status'], 'PASS')
        checked = seal_store.execute('check', spec_path, self.fixture.root / 'checked',
                                     seal_path=self.fixture.root / 'sealed/SEAL.json')
        self.assertEqual(checked['status'], 'PASS')
        (self.out / 'candidate-on-stat-0.perf').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'analysis closure differs'):
            seal_store.execute('check', spec_path, self.fixture.root / 'changed',
                               seal_path=self.fixture.root / 'sealed/SEAL.json')


class OtherProtocolSealTests(unittest.TestCase):
    def fixture(self, cls, method):
        fixture = cls(method)
        fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        snapshot = fixture.root / 'snapshot'
        snapshot.mkdir()
        signed = {'status': 'PASS', 'commit': 'a' * 40, 'tree': 'd' * 40}
        patcher = patch.object(seal_evidence.source_export, 'verify_signed_snapshot', return_value=signed)
        patcher.start()
        self.addCleanup(patcher.stop)
        return fixture, snapshot

    def seal_spec(self, fixture, snapshot, out):
        return {'schema': 1, 'capture': str(out / 'RESULT.json'), 'signing_repo': str(fixture.repo),
                'commit': 'a' * 40, 'native_source': str(snapshot), 'ignored_inputs': []}

    def test_counter_capture_binds_selected_path_counts(self):
        f, snapshot = self.fixture(test_counter_controller.ControllerTests, 'test_complete_capture_is_independently_reparsed')
        request = f.root / 'request.json'
        request.write_text(json.dumps({'schema': 1, 'roles': f.roles, 'inputs': str(f.inputs),
                                       'seed': 'a', 'path_counts': str(f.counts)}))
        out = f.root / 'capture'
        counter_controller.run(f.repo, f.host, f.roles, f.inputs, 'a', out, host_config=f.host_config,
                               path_counts=f.counts, request_spec=role_evidence._pin(request))
        self.assertEqual(seal_evidence.inspect(self.seal_spec(f, snapshot, out))['protocol'], 'counter2-v1')
        (out / 'PATH-COUNTS.json').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'path-count input differs'):
            seal_evidence.inspect(self.seal_spec(f, snapshot, out))

    def test_callgrind_capture_binds_roles_and_input_streams(self):
        f, snapshot = self.fixture(test_callgrind_controller.CallgrindControllerTests, 'test_all_profiles_are_captured_and_reparsed')
        request = f.root / 'request.json'
        request.write_text(json.dumps({'schema': 1, 'roles': f.roles, 'inputs': str(f.inputs),
                                       'symbol_map': str(f.symbol_map)}))
        out = f.root / 'capture'
        callgrind_controller.run(f.repo, f.host, f.roles, f.inputs, out, host_config=f.host_config,
                                 symbol_map=f.symbol_map, request_spec=role_evidence._pin(request))
        result = seal_evidence.inspect(self.seal_spec(f, snapshot, out))
        self.assertEqual(result['protocol'], 'callgrind-v1')
        self.assertEqual(len(result['capture_journals']), 2)
        (out / 'cgu1/data/loop_main.txt').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'generated input changed'):
            seal_evidence.inspect(self.seal_spec(f, snapshot, out))

    def test_qualified_callgrind_seal_rebuilds_its_proof(self):
        f, snapshot = self.fixture(test_callgrind_controller.CallgrindControllerTests, 'test_all_profiles_are_captured_and_reparsed')
        policy = json.loads((Path(callgrind_controller.__file__).parent / 'qualifications/s8.json').read_text())
        policy['candidate_commit'] = 'a' * 40
        proof = {'schema': 1, 'tip_commit': 'b' * 40, 'candidate_commit': 'a' * 40,
                 'control_bytecode_equal': True, 'indexing_source_equal': True, 'entry_source_equal': True,
                 'control_bytecode_sha256': 'c' * 64}
        request = f.root / 'qualified-request.json'
        request.write_text(json.dumps({'schema': 1, 'roles': f.roles, 'inputs': str(f.inputs),
                                       'symbol_map': None, 'qualifications': policy,
                                       'record': 'S8', 'baseline_milestone': 'M7'}))
        out = f.root / 'qualified-capture'
        with patch.object(callgrind_controller.control_proof, 'prove', return_value=proof) as recompile:
            callgrind_controller.run(f.repo, f.host, f.roles, f.inputs, out, host_config=f.host_config,
                                     qualifications=policy, record='S8', baseline_milestone='M7',
                                     request_spec=role_evidence._pin(request))
            result = seal_evidence.inspect(self.seal_spec(f, snapshot, out))
            self.assertEqual(recompile.call_count, 2)
        self.assertEqual(result['assessment']['qualifications'], policy)
        with patch.object(seal_evidence.control_proof, 'prove', return_value=dict(proof, entry_source_equal=False)):
            with self.assertRaisesRegex(ValueError, 'proof'):
                seal_evidence.inspect(self.seal_spec(f, snapshot, out))


if __name__ == '__main__':
    unittest.main()
