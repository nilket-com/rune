"""Synthetic role receipt checks; full builds are exercised separately."""
import json
import hashlib
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parent))
import role_evidence


class RoleEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.source = root / 'source'
        self.source.mkdir()
        (self.source / 'crates/rune').mkdir(parents=True)
        (self.source / 'crates/rune/Cargo.toml').write_text('[features]\nexecution=[]\nnative-code=[]\n')
        self.binary = root / 'binary'
        self.binary.write_bytes(b'synthetic')
        rustc, cargo, lld, wrapper = [root / name for name in ('rustc', 'cargo', 'rust-lld', 'ld.lld')]
        for path in (rustc, cargo, lld, wrapper):
            path.write_text(path.name)
        trace = root / 'exec.trace.1'
        compiler_argv = [str(rustc), '--crate-name', 'rune']
        linker_argv = [str(lld), '-flavor', 'gnu']
        trace.write_text('1 execve(' + json.dumps(str(rustc)) + ', ' + json.dumps(compiler_argv)
                         + ', 0x0) = 0\n1 execve(' + json.dumps(str(lld)) + ', '
                         + json.dumps(linker_argv) + ', 0x0) = 0\n')
        trace_pin = role_evidence._pin(trace)
        compiler = {'path': str(rustc), 'resolved': str(rustc), 'argv': compiler_argv, 'trace': trace_pin}
        linker = {'path': str(lld), 'resolved': str(lld), 'argv': linker_argv, 'trace': trace_pin}
        self.commit = 'a' * 40
        self.tree = 'b' * 40
        self.identity = {'rustc': 'rustc version fixture', 'linker': 'Linker: fixture'}
        harness = root / 'run/harness'
        (harness / 'src').mkdir(parents=True)
        recipe_files = {}
        for name in ('Cargo.toml', 'Cargo.lock', 'src/main.rs'):
            path = harness / name
            path.write_text(name)
            pin = role_evidence._pin(path)
            recipe_files[name] = {'sha256': pin['sha256'], 'bytes': pin['bytes']}
        self.harness = harness
        self.implementation = [{'path': str(root / 'worker.py'), 'sha256': 'c' * 64, 'bytes': 1}]
        self.sysroot = {'root': 'synthetic', 'files': [], 'sha256': 'd' * 64}
        self.registry = {'root': str(root / 'registry'), 'crates': []}
        host_file = root / 'hosts.json'
        host_contents = json.dumps({'schema': 1, 'measurement': {'rustc_identity': self.identity['rustc'],
                                                                  'linker_identity': self.identity['linker']}})
        host_file.write_text(host_contents)
        self.result = {'schema': 1, 'status': 'PASS', 'role': 'candidate', 'profile': 'cgu1', 'native': False,
                       'binary': role_evidence._pin(self.binary),
                       'source': {'status': 'PASS', 'commit': self.commit, 'source': str(self.source),
                                  'tree': self.tree, 'files': []},
                       'recipe': {'status': 'PASS', 'commit': self.commit, 'tree': self.tree, 'profile': 'cgu1',
                                  'execution_feature': 'execution', 'native_feature': 'native-code',
                                  'files': recipe_files, 'harness_inputs': {'fixture': 'e' * 64}},
                       'invocations': {'compilers': [compiler], 'linkers': [linker], 'linker_drivers': [],
                                       'trace_files': [trace_pin]},
                       'tool_pins': [role_evidence._pin(x) for x in (rustc, cargo, lld, wrapper)],
                       'rune_compilations': 1, 'elf_identity': self.identity, 'implementation': self.implementation,
                       'host': 'fixture-host', 'sysroot': self.sysroot, 'registry': self.registry,
                       'host_config': {'path': str(host_file), 'contents': host_contents,
                                       'sha256': hashlib.sha256(host_contents.encode()).hexdigest()}}
        self.receipt = root / 'RESULT.json'
        self.write_receipt()
        self.role = {'name': 'candidate-off', 'role': 'candidate', 'profile': 'cgu1', 'native': False,
                     'commit': self.commit, 'build': role_evidence._pin(self.receipt),
                     'binary': role_evidence._pin(self.binary)}
        self.patches = [patch.object(role_evidence.source_export, 'verify', return_value={'tree': self.tree, 'files': []}),
                        patch.object(role_evidence.source_export, 'verify_signed', return_value={'status': 'PASS'}),
                        patch.object(role_evidence.build_worker, '_trace', return_value=self.result['invocations']),
                        patch.object(role_evidence.build_worker, '_implementation_pins', return_value=self.implementation),
                        patch.object(role_evidence.build_recipe, '_harness_inputs', return_value={'fixture': 'e' * 64}),
                        patch.object(role_evidence.tool_identity, 'sysroot_identity', return_value=self.sysroot),
                        patch.object(role_evidence.cargo_cache, 'verify', return_value=self.registry),
                        patch.object(role_evidence.subprocess, 'run', return_value=SimpleNamespace(returncode=0, stdout='')),
                        patch.object(role_evidence.tool_identity, 'elf_identity', return_value=self.identity)]
        for item in self.patches:
            item.start()
            self.addCleanup(item.stop)

    def write_receipt(self):
        self.receipt.write_text(json.dumps(self.result))

    def test_role_binds_commit_build_and_actual_traces(self):
        checked = role_evidence.check(self.source, self.role)
        self.assertEqual(checked['commit'], self.commit)
        self.assertEqual(checked['signed']['status'], 'PASS')
        trace = Path(self.result['invocations']['trace_files'][0]['path'])
        trace.write_text(trace.read_text().replace('--crate-name', '--bad-flag'))
        with self.assertRaisesRegex(ValueError, 'execution trace changed'):
            role_evidence.check(self.source, self.role)

    def test_changed_binary_or_wrong_role_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'build receipt differs'):
            role_evidence.check(self.source, dict(self.role, role='tip'))
        self.binary.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'binary pin differs'):
            role_evidence.check(self.source, self.role)

    def test_host_identity_drift_is_rejected(self):
        Path(self.result['host_config']['path']).write_text('{}')
        with self.assertRaisesRegex(ValueError, 'host configuration differs'):
            role_evidence.check(self.source, self.role)

    def test_changed_recipe_is_rejected(self):
        (self.harness / 'Cargo.lock').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'recipe input differs'):
            role_evidence.check(self.source, self.role)


if __name__ == '__main__':
    unittest.main()
