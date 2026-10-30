import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import milestone_registry


class MilestoneRegistryTests(unittest.TestCase):
    def test_committed_registry_is_source_only_and_anchors_m8(self):
        value = milestone_registry.load()
        self.assertEqual(value['current'], 'M8')
        self.assertEqual(value['milestones'][-1], {'id': 'M8', 'commit': '9ee355f31ac34aae963f16830ea8b7a76fbd5cb1'})

    def test_invalid_or_duplicated_source_identities_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'registry.json'
            value = milestone_registry.load()
            value['milestones'].append(dict(value['milestones'][-1]))
            path.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, 'ordered and unique'):
                milestone_registry.load(path)
            value['milestones'].pop()
            value['milestones'][-1]['commit'] = 'abcdef'
            path.write_text(json.dumps(value))
            with self.assertRaisesRegex(ValueError, 'source identity'):
                milestone_registry.load(path)


if __name__ == '__main__':
    unittest.main()
