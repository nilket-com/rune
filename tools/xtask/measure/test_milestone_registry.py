import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import milestone_registry


class MilestoneRegistryTests(unittest.TestCase):
    def test_committed_registry_is_source_only_and_anchors_m7(self):
        value = milestone_registry.load()
        self.assertEqual(value['milestones'][-1], {'id': 'M7', 'commit': '38ae7e1fb0b43f245e904d9cb281a80b0e17ca16'})

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
