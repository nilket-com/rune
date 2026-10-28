"""Runtime closure controls for files, directories and symbolic links."""
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_runtime


class RuntimeClosureTests(unittest.TestCase):
    def test_tree_identity_tracks_content_directories_and_links(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'data').write_text('one')
            (root / 'link').symlink_to('data')
            baseline = analysis_runtime._tree(root)
            (root / 'empty').mkdir()
            self.assertNotEqual(analysis_runtime._tree(root)['sha256'], baseline['sha256'])
            (root / 'empty').rmdir()
            (root / 'data').write_text('two')
            self.assertNotEqual(analysis_runtime._tree(root)['sha256'], baseline['sha256'])
            (root / 'data').write_text('one')
            (root / 'link').unlink()
            (root / 'link').symlink_to('other')
            self.assertNotEqual(analysis_runtime._tree(root)['sha256'], baseline['sha256'])

    def test_current_runtime_has_interpreter_and_dependencies(self):
        runtime = analysis_runtime.discover()
        self.assertGreater(runtime['stdlib']['files'], 0)
        self.assertEqual(len(runtime['executables']), 3)
        self.assertGreater(len(runtime['dependencies']), 0)
        self.assertTrue(all(len(item['sha256']) == 64 for item in runtime['dependencies']))


if __name__ == '__main__':
    unittest.main()
