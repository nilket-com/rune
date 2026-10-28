import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import record_policy
import record_store
import test_record_policy


class RecordStoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.ledger = self.root / 'ledger.json'
        self.hosts = self.root / 'hosts.json'
        self.spec = self.root / 'spec.json'
        self.before = {'ledger': 'test', 'milestone_criterion': 'test',
                       'milestones': [{'id': 'M7', 'commit': 'd' * 40}], 'entries': []}
        self.ledger.write_text(json.dumps(self.before))
        self.hosts.write_text(json.dumps({'schema': 1, 'measurement': {'measurement_lock': str(self.root / 'lock')}}))
        pin = {'path': str(self.ledger), 'sha256': hashlib.sha256(self.ledger.read_bytes()).hexdigest(),
               'bytes': len(self.ledger.read_bytes())}
        fixture = test_record_policy.RecordPolicyTests(); fixture.setUp()
        host_hash = hashlib.sha256(self.hosts.read_bytes()).hexdigest()
        self.registry_path = self.root / 'milestones.json'
        self.registry_path.write_text(json.dumps({'schema': 1, 'current': 'M7', 'milestones': [{'id': 'M7', 'commit': 'd' * 40}]}))
        registry_patch = patch.object(record_store.milestone_registry, 'REGISTRY', self.registry_path)
        registry_patch.start()
        self.addCleanup(registry_patch.stop)
        self.verified = [{'spec': {'path': f'spec{i}'}, 'seal': {'path': f'seal{i}'},
                          'evidence': dict(proof, host_config={'sha256': host_hash})}
                         for i, proof in enumerate(fixture.proofs)]
        coverage = record_policy.validate([x['evidence'] for x in self.verified], fixture.commit)
        evidence = {'seals': [{'spec': x['spec'], 'seal': x['seal'], 'protocol': x['evidence']['protocol'],
                               'assessment': x['evidence']['assessment']} for x in self.verified], 'coverage': coverage}
        self.data = {'schema': 1, 'ledger': pin, 'expected_entries': 0,
                     'entry': {'record': 'R', 'subject': 'test', 'date': '2026-01-01',
                               'commit': fixture.commit, 'evidence': evidence,
                               'milestone': {'due': True, 'name': 'M8', 'anchor': 'M7'}},
                     'milestone': {'id': 'M8', 'commit': fixture.commit},
                     'seals': [{'spec': str(i), 'seal': str(i)} for i in range(5)]}
        self.save()

    def save(self):
        self.spec.write_text(json.dumps(self.data))

    def tearDown(self):
        self.temp.cleanup()

    def test_valid_seals_append_entry_and_milestone(self):
        with patch.object(record_store.seal_store, 'verify', side_effect=self.verified):
            result = record_store.execute('append', self.spec, self.hosts, self.root / 'out')
        self.assertEqual(result['transition'], 'appended')
        after = json.loads(self.ledger.read_text())
        self.assertEqual(after['entries'], [self.data['entry']])
        self.assertEqual(after['milestones'], self.before['milestones'] + [self.data['milestone']])

    def test_mismatched_evidence_cannot_mutate_ledger(self):
        self.data['entry']['evidence']['coverage']['cycle_flags'] = ['fake']
        self.save()
        original = self.ledger.read_bytes()
        with patch.object(record_store.seal_store, 'verify', side_effect=self.verified):
            with self.assertRaisesRegex(ValueError, 'exactly reproduce'):
                record_store.execute('append', self.spec, self.hosts, self.root / 'out')
        self.assertEqual(self.ledger.read_bytes(), original)

    def test_check_rejects_stale_ledger_pin(self):
        self.data = {k: self.data[k] for k in ('schema', 'ledger', 'expected_entries')}
        self.save()
        self.ledger.write_text('{}')
        with self.assertRaisesRegex(ValueError, 'declared pin'):
            record_store.execute('check', self.spec, self.hosts, self.root / 'out')


if __name__ == '__main__':
    unittest.main()
