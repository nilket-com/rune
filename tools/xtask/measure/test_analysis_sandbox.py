"""Sandbox controls use synthetic package inputs, never recorded measurements."""
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_sandbox
import analysis_runtime
import input_closure


class SandboxTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.package = root / 'package'
        self.code = root / 'code'
        self.package.mkdir()
        self.code.mkdir()
        (self.package / 'visible').write_text('allowed')
        (self.package / 'hidden').write_text('secret')
        (self.code / 'analyze.py').write_text(
            "from pathlib import Path\n"
            "import subprocess\n"
            "assert not Path('/inputs/hidden').exists()\n"
            "assert not Path('/home').exists()\n"
            "assert not Path('/usr/share').exists()\n"
            "assert subprocess.run(['/usr/bin/nm', '--version'], capture_output=True).returncode == 0\n"
            "assert subprocess.run(['/usr/bin/objdump', '--version'], capture_output=True).returncode == 0\n"
            "assert subprocess.run(['/usr/bin/nm', '-S', '--defined-only', '-C', '/usr/bin/python3'], capture_output=True).returncode == 0\n"
            "assert subprocess.run(['/usr/bin/objdump', '-Cd', '--no-show-raw-insn', '--start-address=0x0', '--stop-address=0x1', '/usr/bin/python3'], capture_output=True).returncode == 0\n"
            "print(Path('/inputs/visible').read_text())\n")
        self.package_pin = input_closure.create(self.package, ['visible'])
        self.code_pin = input_closure.create(self.code, ['analyze.py'])

    def test_only_declared_package_files_are_visible(self):
        self.assertEqual(analysis_sandbox.run(self.package, self.package_pin,
                                              self.code, self.code_pin, 'analyze.py'), b'allowed\n')

    def test_changed_or_undeclared_inputs_fail(self):
        with self.assertRaisesRegex(ValueError, 'entry is not pinned'):
            analysis_sandbox.run(self.package, self.package_pin, self.code, self.code_pin, 'other.py')
        (self.package / 'visible').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'input changed'):
            analysis_sandbox.run(self.package, self.package_pin, self.code, self.code_pin, 'analyze.py')
        (self.package / 'visible').write_text('allowed')
        runtime = analysis_runtime.discover()
        runtime['stdlib']['sha256'] = '0' * 64
        with self.assertRaisesRegex(ValueError, 'runtime differs'):
            analysis_sandbox.run(self.package, self.package_pin, self.code, self.code_pin, 'analyze.py', runtime=runtime)


if __name__ == '__main__':
    unittest.main()
