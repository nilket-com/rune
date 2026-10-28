"""ELF comment identity controls; tool expectations are supplied by the caller."""
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import tool_identity


class ToolIdentityTests(unittest.TestCase):
    def test_exact_compiler_and_linker(self):
        text = '  [ 0] rustc version example\n  [12] Linker: LLD example\n'
        self.assertEqual(tool_identity.elf_identity(text, 'rustc version example', 'Linker: LLD example'),
                         {'rustc': 'rustc version example', 'linker': 'Linker: LLD example'})
        for bad in (text.replace('LLD example', 'mold'), text + '  [13] GNU ld other\n',
                    text.replace('rustc version example', 'rustc version other')):
            with self.assertRaises(ValueError):
                tool_identity.elf_identity(bad, 'rustc version example', 'Linker: LLD example')

    def test_missing_expected_identity_is_rejected(self):
        with self.assertRaises(ValueError):
            tool_identity.elf_identity('', '', 'Linker: LLD example')

    def test_actual_invocations_and_wrapper(self):
        line = '1.25 execve("/tools/rustc", ["/tools/rustc", "--crate-name", "app"], 0x1) = 0'
        self.assertEqual(tool_identity.parse_exec_line(line)['argv'][-1], 'app')
        self.assertIsNone(tool_identity.parse_exec_line(line.replace('= 0', '= -1')))
        trace = {'path': 'trace-1'}
        rows = [dict(path='/tools/rustc', resolved='/tools/rustc', argv=['rustc', '--crate-name', 'app'], trace=trace),
                dict(path='/tools/ld.lld', resolved='/tools/ld.lld', argv=['ld.lld'], trace=trace),
                dict(path='/tools/rust-lld', resolved='/tools/rust-lld', argv=['rust-lld'], trace=trace)]
        result = tool_identity.validate_invocations(rows, '/tools/rustc', '/tools/rust-lld', '/tools/ld.lld')
        self.assertEqual(len(result['linkers']), 2)
        with self.assertRaises(ValueError):
            tool_identity.validate_invocations(rows[:-1], '/tools/rustc', '/tools/rust-lld', '/tools/ld.lld')
        rows[1]['trace'] = {'path': 'other'}
        with self.assertRaises(ValueError):
            tool_identity.validate_invocations(rows, '/tools/rustc', '/tools/rust-lld', '/tools/ld.lld')

    def test_sysroot_inventory_detects_library_changes(self):
        with tempfile.TemporaryDirectory() as root:
            lib = Path(root) / 'lib' / 'rustlib' / 'test-host' / 'lib'
            lib.mkdir(parents=True)
            (lib / 'std.rlib').write_bytes(b'standard library')
            (Path(root) / 'lib' / 'compiler.so').write_bytes(b'compiler library')
            before = tool_identity.sysroot_identity(root, 'test-host')
            self.assertEqual(len(before['files']), 2)
            (lib / 'std.rlib').write_bytes(b'changed')
            self.assertNotEqual(before['sha256'], tool_identity.sysroot_identity(root, 'test-host')['sha256'])
            with self.assertRaises(ValueError):
                tool_identity.sysroot_identity(root, '../escape')

    def test_actual_rune_feature_state(self):
        on = [{'argv': ['rustc', '--crate-name', 'rune', '--cfg', 'feature="native-code"']}]
        off = [{'argv': ['rustc', '--crate-name', 'rune']}]
        self.assertEqual(tool_identity.validate_rune_feature(on, True), 1)
        self.assertEqual(tool_identity.validate_rune_feature(off, False), 1)
        for records, native in ((on, False), (off, True), ([], False)):
            with self.assertRaises(ValueError):
                tool_identity.validate_rune_feature(records, native)


if __name__ == '__main__':
    unittest.main()
