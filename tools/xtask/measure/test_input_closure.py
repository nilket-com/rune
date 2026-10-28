"""Explicit input-closure controls using synthetic temporary files."""
from pathlib import Path
import hashlib
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import input_closure


class ClosureTests(unittest.TestCase):
    def test_declared_inputs_and_optional_branch(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'required').write_text('required')
            (root / 'optional').write_text('optional')
            basic = input_closure.create(root, ['required'])
            extended = input_closure.create(root, ['required', 'optional'])
            self.assertTrue(input_closure.verify(root, basic))
            self.assertTrue(input_closure.verify(root, extended))
            (root / 'optional').unlink()
            self.assertTrue(input_closure.verify(root, basic))
            with self.assertRaises(ValueError):
                input_closure.verify(root, extended)
            (root / 'required').write_text('changed')
            with self.assertRaises(ValueError):
                input_closure.verify(root, basic)

    def test_paths_duplicates_and_symlinks_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'file').write_text('data')
            (root / 'link').symlink_to(root / 'file')
            for names in (['file', 'file'], ['../file'], ['/absolute'], ['link']):
                with self.assertRaises(ValueError):
                    input_closure.create(root, names)
            pin = input_closure.create(root, ['file'])
            pin['extra'] = True
            with self.assertRaises(ValueError):
                input_closure.verify(root, pin)

    def test_change_during_read_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / 'file'
            path.write_bytes(b'original')
            original = hashlib.sha256

            class ChangingDigest:
                def __init__(self):
                    self.inner = original()

                def update(self, data):
                    self.inner.update(data)
                    path.write_bytes(b'modified')

                def hexdigest(self):
                    return self.inner.hexdigest()

            with patch.object(input_closure.hashlib, 'sha256', ChangingDigest):
                with self.assertRaises(ValueError):
                    input_closure.create(root, ['file'])


if __name__ == '__main__':
    unittest.main()
