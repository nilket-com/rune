import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ledger_store


class LedgerStoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        root = Path(self.temp.name)
        self.ledger = root / 'ledger.json'
        self.lock = root / 'ledger.lock'
        self.before = {'ledger': 'test', 'milestone_criterion': 'test',
                       'milestones': [{'id': 'M7'}], 'entries': []}
        self.entry = {'record': 'R', 'subject': 'test', 'date': '2026-01-01',
                      'commit': 'a' * 40,
                      'milestone': {'due': True, 'name': 'M8', 'anchor': 'M7'}}
        self.milestone = {'id': 'M8', 'commit': 'a' * 40}
        self.ledger.write_text(json.dumps(self.before))
        self.digest = hashlib.sha256(self.ledger.read_bytes()).hexdigest()

    def tearDown(self):
        self.temp.cleanup()

    def test_entry_and_milestone_are_persisted_together(self):
        calls = []
        result = ledger_store.append(self.ledger, self.lock, self.digest, self.entry,
                                     self.milestone, lambda: calls.append(True))
        self.assertEqual(result, 'appended')
        after = json.loads(self.ledger.read_text())
        self.assertEqual(after['entries'], [self.entry])
        self.assertEqual(after['milestones'], self.before['milestones'] + [self.milestone])
        self.assertEqual(calls, [True])
        with self.assertRaisesRegex(ValueError, 'changed'):
            ledger_store.append(self.ledger, self.lock, self.digest, self.entry,
                                self.milestone, lambda: None)

    def test_failure_does_not_change_ledger(self):
        def reject():
            raise ValueError('evidence rejected')
        original = self.ledger.read_bytes()
        with self.assertRaisesRegex(ValueError, 'evidence rejected'):
            ledger_store.append(self.ledger, self.lock, self.digest, self.entry,
                                self.milestone, reject)
        self.assertEqual(self.ledger.read_bytes(), original)
        self.assertEqual(sorted(p.name for p in self.ledger.parent.iterdir()),
                         ['ledger.json', 'ledger.lock'])

    def test_concurrent_ledger_change_during_evidence_is_not_overwritten(self):
        changed = b'{"external": true}\n'
        def update():
            self.ledger.write_bytes(changed)
        with self.assertRaisesRegex(ValueError, 'changed during evidence'):
            ledger_store.append(self.ledger, self.lock, self.digest, self.entry, self.milestone, update)
        self.assertEqual(self.ledger.read_bytes(), changed)


if __name__ == '__main__':
    unittest.main()
