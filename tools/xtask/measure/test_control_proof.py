"""The qualification proof must come from live, matching source and compilation."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import control_proof


class ControlProofTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.roles = {}
        for name, commit in (('tip', 'a' * 40), ('candidate', 'b' * 40)):
            source = self.root / name / 'source'
            core = source / 'crates/rune/src/experimental_core'
            (core / 'heap').mkdir(parents=True)
            (core / 'heap/indexing.rs').write_text('index source')
            (core / 'execute.rs').write_text('entry source')
            receipt = self.root / name / 'RESULT.json'
            receipt.write_text(json.dumps({'source': {'source': str(source)}}))
            self.roles[name + '-cgu1-off'] = {'commit': commit, 'build': {'path': str(receipt)}}
        self.kernels = self.root / 'kernels'
        self.kernels.mkdir()
        p = patch.object(control_proof.source_export, 'verify_signed', return_value={})
        p.start()
        self.addCleanup(p.stop)
        p = patch.object(control_proof.milestone_registry, 'load', return_value={
            'milestones': [{'id': 'M7', 'commit': 'a' * 40}]})
        p.start()
        self.addCleanup(p.stop)

    def test_rebuilds_both_roles_and_compares_live_output(self):
        with patch.object(control_proof, '_role', side_effect=[b'live code', b'live code']) as build:
            proof = control_proof.prove(self.root, self.roles, self.kernels)
        self.assertEqual(build.call_count, 2)
        self.assertTrue(proof['control_bytecode_equal'])
        self.assertTrue(proof['entry_source_equal'])
        with patch.object(control_proof, '_role', side_effect=[b'live code', b'other code']):
            with self.assertRaisesRegex(ValueError, 'bytecode differs'):
                control_proof.prove(self.root, self.roles, self.kernels)

    def test_changed_entry_or_indexing_source_fails_before_compilation(self):
        core = self.root / 'candidate/source/crates/rune/src/experimental_core'
        for file in ('execute.rs', 'heap/indexing.rs'):
            path = core / file
            before = path.read_text()
            path.write_text(before + ' changed')
            with patch.object(control_proof, '_role') as build:
                with self.assertRaisesRegex(ValueError, 'source differs'):
                    control_proof.prove(self.root, self.roles, self.kernels)
                build.assert_not_called()
            path.write_text(before)

    def test_qualification_is_bound_to_m7(self):
        self.roles['tip-cgu1-off']['commit'] = 'c' * 40
        with self.assertRaisesRegex(ValueError, 'M7 baseline'):
            control_proof.prove(self.root, self.roles, self.kernels)


if __name__ == '__main__':
    unittest.main()
