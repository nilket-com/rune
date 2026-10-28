"""Synthetic controls for isolated build-worker preflight and process receipts."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_worker


class WorkerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        for name in ('repo', 'toolchain', 'cache', 'run'):
            (self.root / name).mkdir()
        self.options = dict(role='candidate', toolchain=self.root / 'toolchain',
                            cargo_cache=self.root / 'cache', lock_path=self.root / 'lock',
                            cpus=[min(os.sched_getaffinity(0))], jobs=1, timeout=10,
                            expected_rustc='rustc version example', expected_linker='Linker: LLD example')

    def test_preflight_rejects_invalid_and_overlapping_inputs(self):
        for key, value in [('role', 'unknown'), ('cpus', []), ('jobs', 0), ('timeout', 0),
                           ('expected_rustc', ''), ('lock_path', self.root / 'repo' / 'lock')]:
            options = dict(self.options, **{key: value})
            with self.assertRaises(ValueError):
                build_worker.build(self.root / 'repo', 'HEAD', 'cgu1', True, self.root / 'out', **options)
        with self.assertRaises(ValueError):
            build_worker.build(self.root / 'repo', 'HEAD', 'cgu1', True, self.root / 'repo' / 'out', **self.options)
        with self.assertRaises(ValueError):
            build_worker.build(self.root / 'repo', 'HEAD', 'cgu1', True, self.root / 'out',
                               host_config=self.root / 'host.json', **self.options)
        self.assertFalse((self.root / 'out').exists())

    def test_capture_records_success_and_failure(self):
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(self.root), 'LANG': 'C.UTF-8'}
        command = ['/usr/bin/python3', '-I', '-B', '-c', 'print("hello")']
        self.assertEqual(build_worker._capture(command, self.root / 'run', 'ok', env=env), 'hello\n')
        receipt = json.loads((self.root / 'run' / 'ok.json').read_text())
        self.assertEqual(receipt['status'], 0)
        self.assertEqual(receipt['stdout']['bytes'], 6)
        with self.assertRaises(ValueError):
            build_worker._capture(['/usr/bin/false'], self.root / 'run', 'failure', env=env)
        self.assertNotEqual(json.loads((self.root / 'run' / 'failure.json').read_text())['status'], 0)
        with self.assertRaises(ValueError):
            build_worker._capture(['/usr/bin/python3', '-I', '-c', 'import time; time.sleep(10)'],
                                  self.root / 'run', 'timeout', env=env, timeout=0.05)
        self.assertIn('TimeoutExpired', json.loads((self.root / 'run' / 'timeout.json').read_text())['failure'])

    def test_memory_budget_requires_a_kernel_limit(self):
        root = self.root / 'cgroup'
        child = root / 'parent' / 'child'
        child.mkdir(parents=True)
        (root / 'memory.max').write_text('max')
        (root / 'parent' / 'memory.max').write_text('1000')
        (child / 'memory.max').write_text('max')
        self.assertEqual(build_worker.attest_memory(root, '/parent/child', 1200)['effective_max_bytes'], 1000)
        with self.assertRaises(ValueError):
            build_worker.attest_memory(root, '/parent/child', 900)
        with self.assertRaises(ValueError):
            build_worker.attest_memory(root, '/../outside', 1200)

    def test_isolated_cli_help(self):
        script = Path(build_worker.__file__).resolve()
        result = subprocess.run(['/usr/bin/python3', '-I', '-B', str(script), '--help'],
                                stdin=subprocess.DEVNULL, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0)
        self.assertIn(b'--revision', result.stdout)


if __name__ == '__main__':
    unittest.main()
