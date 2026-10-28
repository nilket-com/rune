"""Local measurement primitives tested with short, synthetic children."""
from pathlib import Path
import os
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import measurement_runtime


class RuntimeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.cpu = min(os.sched_getaffinity(0))
        self.env = {'PATH': '/usr/bin:/bin', 'HOME': str(self.root), 'LANG': 'C.UTF-8'}

    def test_lock_and_child_affinity(self):
        lock = self.root / 'lock'
        with measurement_runtime.lease(lock, self.cpu) as lease:
            self.assertEqual(lease.receipt['cpu'], self.cpu)
            with self.assertRaises(BlockingIOError):
                with measurement_runtime.lease(lock, self.cpu):
                    pass
            with self.assertRaisesRegex(ValueError, 'readiness'):
                lease.execute(['/bin/true'], cwd=self.root, env=self.env, stdout_path=self.root / 'pre-out',
                              stderr_path=self.root / 'pre-err', timeout=1, deadline=time.monotonic() + 2)
            with patch.object(measurement_runtime.measurement_readiness, 'snapshot',
                              return_value={'load': 0, 'mem_available_bytes': 100, 'busy': []}):
                lease.ready(max_load=1, min_available_bytes=1)
            result = lease.execute(
                ['/usr/bin/python3', '-I', '-c', 'import os; print(sorted(os.sched_getaffinity(0)))'],
                cwd=self.root, env=self.env, stdout_path=self.root / 'out',
                stderr_path=self.root / 'err', timeout=10, deadline=time.monotonic() + 20)
        self.assertEqual(result['status'], 0)
        self.assertEqual((self.root / 'out').read_text().strip(), str([self.cpu]))
        self.assertEqual((self.root / 'err').read_bytes(), b'')
        with self.assertRaisesRegex(ValueError, 'no longer active'):
            lease.execute(['/bin/true'], cwd=self.root, env=self.env, stdout_path=self.root / 'out2',
                          stderr_path=self.root / 'err2', timeout=1, deadline=time.monotonic() + 2)

    def test_deadline_and_environment_fail_closed(self):
        with measurement_runtime.lease(self.root / 'lock', self.cpu) as lease:
            with patch.object(measurement_runtime.measurement_readiness, 'snapshot',
                              return_value={'load': 0, 'mem_available_bytes': 100, 'busy': []}):
                lease.ready(max_load=1, min_available_bytes=1)
            with self.assertRaises(subprocess.TimeoutExpired):
                lease.execute(['/usr/bin/python3', '-c', 'import time; time.sleep(10)'],
                              cwd=self.root, env=self.env, stdout_path=self.root / 'out',
                              stderr_path=self.root / 'err', timeout=0.1, deadline=time.monotonic() + 1)
            with self.assertRaisesRegex(ValueError, 'environment'):
                lease.execute(['/bin/true'], cwd=self.root, env=dict(self.env, PERF_BUILDID_DIR='bad'),
                              stdout_path=self.root / 'out2', stderr_path=self.root / 'err2',
                              timeout=1, deadline=time.monotonic() + 2)

    def test_perf_policy_rejects_configuration_and_wrong_paranoid(self):
        paranoid = self.root / 'paranoid'
        paranoid.write_text('2\n')
        system_config = self.root / 'system-perfconfig'
        self.assertEqual(measurement_runtime.check_perf_policy(self.root, system_config=system_config,
                                                                paranoid=paranoid)['perf_event_paranoid'], '2')
        (self.root / '.perfconfig').write_text('settings')
        with self.assertRaisesRegex(ValueError, 'configuration'):
            measurement_runtime.check_perf_policy(self.root, system_config=system_config, paranoid=paranoid)
        (self.root / '.perfconfig').unlink()
        paranoid.write_text('3\n')
        with self.assertRaisesRegex(ValueError, 'paranoid'):
            measurement_runtime.check_perf_policy(self.root, system_config=system_config, paranoid=paranoid)

    def test_measurement_runtime_dependencies_are_pinned(self):
        snapshot = measurement_runtime.dependency_snapshot(['/usr/bin/python3'])
        self.assertTrue(snapshot['libraries'])
        self.assertEqual(len(snapshot['executables']), 1)
        self.assertTrue(all(len(pin['sha256']) == 64 for pin in snapshot['libraries']))
        with self.assertRaisesRegex(ValueError, 'executables are missing'):
            measurement_runtime.dependency_snapshot([])
        tools = self.root / 'tools'
        tools.mkdir()
        (self.root / 'external').write_text('outside')
        (tools / 'link').symlink_to(self.root / 'external')
        with self.assertRaisesRegex(ValueError, 'external link'):
            measurement_runtime.tool_tree(tools)


if __name__ == '__main__':
    unittest.main()
