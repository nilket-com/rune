"""Fresh seal creation, rechecking and failure controls."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import seal_store


class SealStoreTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for name in ('repo', 'source', 'capture'):
            (self.root / name).mkdir()
        (self.root / 'capture/RESULT.json').write_text('{}')
        self.spec = {'schema': 1, 'capture': str(self.root / 'capture/RESULT.json'),
                     'signing_repo': str(self.root / 'repo'), 'native_source': str(self.root / 'source'),
                     'commit': 'a' * 40, 'ignored_inputs': []}
        self.spec_path = self.root / 'seal-spec.json'
        self.spec_path.write_text(json.dumps(self.spec))
        self.evidence = {'candidate_commit': 'a' * 40, 'protocol': 'k1-v1', 'assessment': {'ok': True}}
        patcher = patch.object(seal_store.seal_evidence, 'inspect', return_value=self.evidence)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_create_and_recheck_fresh_evidence(self):
        created = seal_store.execute('create', self.spec_path, self.root / 'created')
        self.assertEqual(created['status'], 'PASS')
        checked = seal_store.execute('check', self.spec_path, self.root / 'checked',
                                     seal_path=self.root / 'created/SEAL.json')
        self.assertEqual(checked['status'], 'PASS')
        self.assertTrue((self.root / 'checked/COMPLETE.json').is_file())
        self.spec_path.write_text(json.dumps(dict(self.spec, commit='b' * 40)))
        with self.assertRaisesRegex(ValueError, 'sealed evidence differs'):
            seal_store.execute('check', self.spec_path, self.root / 'changed',
                               seal_path=self.root / 'created/SEAL.json')
        self.assertEqual(json.loads((self.root / 'changed/FAILURE.json').read_text())['status'], 'FAIL')

    def test_refuses_overlapping_output_and_invalid_operation(self):
        with self.assertRaisesRegex(ValueError, 'overlaps'):
            seal_store.execute('create', self.spec_path, self.root / 'repo/output')
        with self.assertRaisesRegex(ValueError, 'invalid seal operation'):
            seal_store.execute('check', self.spec_path, self.root / 'out')


if __name__ == '__main__':
    unittest.main()
