"""Commit export controls using only a temporary Git repository."""
from pathlib import Path
import hashlib
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import source_export


class ExportTests(unittest.TestCase):
    def git(self, *args):
        source_export._git(self.repo, *args)

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.git('init', '-q')
        self.git('config', 'user.name', 'Test')
        self.git('config', 'user.email', 'test@example.invalid')
        (self.repo / '.gitignore').write_text('ignored\n')
        (self.repo / 'input').write_text('committed\n')
        self.git('add', '.gitignore', 'input')
        self.git('commit', '-qm', 'fixture')

    def test_commit_export_ignores_dirty_worktree_and_detects_drift(self):
        (self.repo / 'input').write_text('working-tree edit\n')
        (self.repo / 'ignored').write_text('ignored data\n')
        result = source_export.export(self.repo, 'HEAD', self.root / 'out')
        exported = Path(result['source'])
        self.assertEqual((exported / 'input').read_text(), 'committed\n')
        self.assertEqual(result['status'], 'PASS')
        self.assertEqual(source_export.verify(exported, result['commit'])['files'], result['files'])
        (exported / 'ignored').write_text('unexpected\n')
        with self.assertRaises(ValueError):
            source_export.verify(exported, result['commit'])
        (exported / 'ignored').unlink()
        (exported / 'input').write_text('modified\n')
        with self.assertRaises(ValueError):
            source_export.verify(exported, result['commit'])
        with self.assertRaises(FileExistsError):
            source_export.export(self.repo, 'HEAD', self.root / 'out')

    def test_rejects_nested_output_and_option_revision(self):
        with self.assertRaises(ValueError):
            source_export.export(self.repo, 'HEAD', self.repo / 'out')
        with self.assertRaises(ValueError):
            source_export.export(self.repo, '--help', self.root / 'out')
        with self.assertRaises(ValueError):
            source_export.verify(self.repo, '--help')
        exported = source_export.export(self.repo, 'HEAD', self.root / 'unsigned')
        with self.assertRaises(ValueError):
            source_export.verify_signed(self.repo, exported['source'], exported['commit'])

    def test_signed_standalone_snapshot_and_declared_extra(self):
        key = self.root / 'signing-key'
        subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(key)], check=True)
        signers = self.root / 'allowed-signers'
        signers.write_text('test@example.invalid ' + key.with_suffix('.pub').read_text())
        self.git('config', 'gpg.format', 'ssh')
        self.git('config', 'user.signingkey', str(key))
        self.git('config', 'gpg.ssh.allowedSignersFile', str(signers))
        self.git('commit', '--allow-empty', '-S', '-qm', 'signed snapshot')
        commit = source_export._git(self.repo, 'rev-parse', 'HEAD').decode().strip()
        snapshot = self.root / 'snapshot'
        snapshot.mkdir()
        shutil.copy(self.repo / 'input', snapshot / 'input')
        shutil.copy(self.repo / '.gitignore', snapshot / '.gitignore')
        extra = b'declared\n'
        (snapshot / 'ignored').write_bytes(extra)
        pins = [{'path': 'ignored', 'sha256': hashlib.sha256(extra).hexdigest(),
                 'bytes': len(extra), 'git_mode': '100644'}]
        observed = source_export.verify_signed_snapshot(self.repo, snapshot, commit, pins)
        self.assertEqual(observed['ignored_files'], 1)
        with self.assertRaisesRegex(ValueError, 'invalid ignored input'):
            source_export.verify_signed_snapshot(self.repo, snapshot, commit,
                                                 [dict(pins[0], bytes=True)])
        (snapshot / 'ignored').write_bytes(b'changed\n')
        with self.assertRaisesRegex(ValueError, 'ignored input differs'):
            source_export.verify_signed_snapshot(self.repo, snapshot, commit, pins)
        (snapshot / 'ignored').unlink()
        with self.assertRaisesRegex(ValueError, 'missing or undeclared'):
            source_export.verify_signed_snapshot(self.repo, snapshot, commit, pins)
        (snapshot / 'input').write_text('changed\n')
        with self.assertRaisesRegex(ValueError, 'snapshot differs'):
            source_export.verify_signed_snapshot(self.repo, snapshot, commit, [])
        (snapshot / 'input').write_text('committed\n')
        (snapshot / 'unexpected').write_text('unlisted\n')
        with self.assertRaisesRegex(ValueError, 'missing or undeclared'):
            source_export.verify_signed_snapshot(self.repo, snapshot, commit, [])
        (snapshot / 'unexpected').unlink()
        (snapshot / 'input').unlink()
        (snapshot / 'input').symlink_to(self.repo / 'input')
        with self.assertRaisesRegex(ValueError, 'linked source'):
            source_export.verify_signed_snapshot(self.repo, snapshot, commit, [])


if __name__ == '__main__':
    unittest.main()
