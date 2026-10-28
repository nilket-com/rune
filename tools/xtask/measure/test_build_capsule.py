"""Exercise the offline build capsule with temporary, synthetic inputs."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_capsule


class CapsuleTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir='/tmp')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for name in ('out', 'source', 'harness', 'toolchain', 'cache'):
            (self.root / name).mkdir()
        (self.root / 'source' / 'input').write_text('source')
        (self.root / 'harness' / 'marker').write_text('harness')
        (self.root / 'cache' / 'registry').mkdir()
        (self.root / 'cache' / 'registry' / 'cache-marker').write_text('cache')

    def test_capsule_masks_home_and_tmp_and_makes_inputs_read_only(self):
        nested_harness = self.root / 'out' / 'harness'
        nested_harness.mkdir()
        (nested_harness / 'marker').write_text('harness')
        args, env = build_capsule.prepare(self.root / 'out', self.root / 'source', nested_harness,
                                          self.root / 'toolchain', self.root / 'cache')
        secret = self.root / 'hidden-marker'
        secret.write_text('hidden')
        program = '''from pathlib import Path
import os
assert not Path(os.environ['SECRET_PATH']).exists()
assert Path('/tmp/rune-build/source/input').read_text() == 'source'
assert Path('/tmp/rune-build/harness/marker').read_text() == 'harness'
try:
 Path('/tmp/rune-build/out/harness/marker').write_text('changed')
 raise AssertionError('harness was writable through output alias')
except OSError:
 pass
assert Path('/tmp/rune-build/out/cargo-home/config.toml').read_text() == '[net]\\noffline = true\\n'
assert Path('/tmp/rune-build/out/cargo-home/registry/cache-marker').read_text() == 'cache'
try:
 Path('/tmp/rune-build/out/cargo-home/registry/cache-marker').write_text('changed')
 raise AssertionError('cache was writable')
except OSError:
 pass
try:
 Path('/tmp/rune-build/source/input').write_text('changed')
 raise AssertionError('source was writable')
except OSError:
 pass
Path('/tmp/rune-build/out/proof').write_text('ok')
'''
        env['SECRET_PATH'] = str(secret)
        result = subprocess.run(args + ['--', '/usr/bin/python3', '-I', '-B', '-c', program],
                                env=env, stdin=subprocess.DEVNULL, capture_output=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr.decode(errors='replace'))
        self.assertEqual((self.root / 'out' / 'proof').read_text(), 'ok')
        self.assertEqual((self.root / 'source' / 'input').read_text(), 'source')

    def test_rejects_output_overlapping_source(self):
        with self.assertRaises(ValueError):
            build_capsule.prepare(self.root / 'source', self.root / 'source',
                                  self.root / 'harness', self.root / 'toolchain', self.root / 'cache')
        (self.root / 'harness' / '.cargo').mkdir()
        (self.root / 'harness' / '.cargo' / 'config.toml').write_text('[net]\noffline = false\n')
        with self.assertRaises(ValueError):
            build_capsule.prepare(*(self.root / name for name in ('out', 'source', 'harness', 'toolchain', 'cache')))


if __name__ == '__main__':
    unittest.main()
