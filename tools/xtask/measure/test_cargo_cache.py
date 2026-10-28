"""Synthetic locked crate archive and extracted-source integrity controls."""
import hashlib
import io
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import cargo_cache


class CargoCacheTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.cache = self.root / 'cache'
        self.src = self.cache / 'registry/src/index/fixture-1.0.0'
        self.src.mkdir(parents=True)
        (self.src / 'src').mkdir()
        (self.src / 'src/lib.rs').write_bytes(b'pub fn value() {}\n')
        (self.src / '.cargo-ok').write_text('ok')
        archive = self.cache / 'registry/cache/index/fixture-1.0.0.crate'
        archive.parent.mkdir(parents=True)
        with tarfile.open(archive, 'w:gz') as tar:
            data = b'pub fn value() {}\n'
            member = tarfile.TarInfo('fixture-1.0.0/src/lib.rs')
            member.size = len(data)
            tar.addfile(member, io.BytesIO(data))
        self.archive = archive
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        self.lock = self.root / 'Cargo.lock'
        self.lock.write_text('version = 4\n[[package]]\nname = "fixture"\nversion = "1.0.0"\n'
                             'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
                             f'checksum = "{checksum}"\n')

    def test_extracted_source_must_match_locked_archive(self):
        result = cargo_cache.verify(self.lock, self.cache)
        self.assertEqual(len(result['crates']), 1)
        (self.src / 'src/lib.rs').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'source differs'):
            cargo_cache.verify(self.lock, self.cache)

    def test_extra_source_and_changed_archive_fail(self):
        (self.src / 'extra').write_text('untracked')
        with self.assertRaisesRegex(ValueError, 'inventory differs'):
            cargo_cache.verify(self.lock, self.cache)
        (self.src / 'extra').unlink()
        self.archive.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'archive checksum differs'):
            cargo_cache.verify(self.lock, self.cache)


if __name__ == '__main__':
    unittest.main()
