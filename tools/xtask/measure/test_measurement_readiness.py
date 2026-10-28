"""Readiness controls with synthetic proc and CPU observations."""
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import measurement_readiness


class ReadinessTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.proc, self.sys = root / 'proc', root / 'sys'
        self.proc.mkdir()
        (self.proc / 'loadavg').write_text('1.25 0 0 0/0 0\n')
        (self.proc / 'meminfo').write_text('MemAvailable: 5000 kB\n')
        path = self.sys / 'devices/system/cpu/cpu7/thermal_throttle'
        path.mkdir(parents=True)
        (path / 'core_throttle_count').write_text('2\n')

    def test_declared_limits_and_competing_process(self):
        observed = measurement_readiness.snapshot(7, proc_root=self.proc, sys_root=self.sys)
        self.assertEqual(observed['core_throttle_count'], 2)
        self.assertEqual(measurement_readiness.require_ready(observed, max_load=1.5, min_available_bytes=1024)['status'], 'READY')
        busy = self.proc / '123456'
        busy.mkdir()
        (busy / 'comm').write_text('rustc\n')
        observed = measurement_readiness.snapshot(7, proc_root=self.proc, sys_root=self.sys)
        with self.assertRaisesRegex(ValueError, 'not ready'):
            measurement_readiness.require_ready(observed, max_load=1.5, min_available_bytes=1024)

    def test_missing_or_exceeded_limits_fail(self):
        observed = measurement_readiness.snapshot(7, proc_root=self.proc, sys_root=self.sys)
        with self.assertRaisesRegex(ValueError, 'not ready'):
            measurement_readiness.require_ready(observed, max_load=1, min_available_bytes=1024)
        with self.assertRaisesRegex(ValueError, 'declared'):
            measurement_readiness.require_ready(observed, max_load=float('nan'), min_available_bytes=1024)
        (self.proc / 'meminfo').write_text('MemFree: 10 kB\n')
        with self.assertRaisesRegex(ValueError, 'available memory'):
            measurement_readiness.snapshot(7, proc_root=self.proc, sys_root=self.sys)


if __name__ == '__main__':
    unittest.main()
