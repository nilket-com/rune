"""Local lock, affinity and deadline primitives for future measurement controllers."""
from contextlib import contextmanager
import fcntl
import math
import os
from pathlib import Path
import signal
import subprocess
import time
import measurement_readiness
import analysis_runtime


def dependency_snapshot(executables):
    """Pin dynamic dependencies and loader configuration of measured tools."""
    paths = [Path(path).resolve(strict=True) for path in executables]
    if not paths or any(not path.is_file() for path in paths):
        raise ValueError('measurement executables are missing')
    libraries = analysis_runtime._dependencies(paths, require_complete=True)
    cache = Path('/etc/ld.so.cache')
    return {'executables': [analysis_runtime._file(path) for path in paths], 'libraries': libraries,
            'loader_cache': analysis_runtime._file(cache) if cache.exists() else None,
            'discovery_tools': [analysis_runtime._file('/usr/bin/ldd'), analysis_runtime._file('/bin/sh')]}


def tool_tree(root):
    """Pin an installation and reject links that escape it."""
    root = Path(root).resolve(strict=True)
    for path in root.rglob('*'):
        if path.is_symlink() and not path.resolve(strict=True).is_relative_to(root):
            raise ValueError('tool installation contains an external link')
    return analysis_runtime._tree(root)


def check_perf_policy(home, *, system_config='/etc/perfconfig', paranoid='/proc/sys/kernel/perf_event_paranoid'):
    if (Path(home) / '.perfconfig').exists() or Path(system_config).exists():
        raise ValueError('perf configuration must be absent')
    value = Path(paranoid).read_text().strip()
    if value != '2':
        raise ValueError('perf_event_paranoid differs from the measurement protocol')
    return {'perfconfig_absent': True, 'perf_event_paranoid': value}


class Lease:
    """An active lock-bound capability for measurement children."""
    def __init__(self, lock, path, cpu):
        self._lock = lock
        self._active = True
        self._ready = False
        self.cpu = cpu
        self.receipt = {'cpu': cpu, 'lock': str(path), 'effective_cpus': sorted(os.sched_getaffinity(0))}

    def execute(self, argv, **kwargs):
        if not self._active or self._lock.closed:
            raise ValueError('measurement lease is no longer active')
        if not self._ready:
            raise ValueError('measurement readiness has not been established')
        return _execute(argv, cpu=self.cpu, **kwargs)

    def ready(self, *, max_load, min_available_bytes):
        if not self._active or self._lock.closed:
            raise ValueError('measurement lease is no longer active')
        self._ready = False
        receipt = measurement_readiness.require_ready(measurement_readiness.snapshot(self.cpu),
                                                       max_load=max_load, min_available_bytes=min_available_bytes)
        self._ready = True
        return receipt


@contextmanager
def lease(lock_path, cpu):
    """Hold the configured measurement lock and check the effective CPU set."""
    path = Path(lock_path)
    if not path.is_absolute() or path.is_symlink():
        raise ValueError('measurement lock must be an absolute regular path')
    if type(cpu) is not int or cpu not in os.sched_getaffinity(0):
        raise ValueError('measurement CPU is outside effective affinity')
    descriptor = os.open(path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
    with os.fdopen(descriptor, 'a+b') as lock:
        import stat
        if not stat.S_ISREG(os.fstat(lock.fileno()).st_mode):
            raise ValueError('measurement lock must be a regular file')
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        capability = Lease(lock, path, cpu)
        try:
            yield capability
        finally:
            capability._active = False


def _execute(argv, *, cwd, env, cpu, stdout_path, stderr_path, timeout, deadline):
    """Run a single pinned child, killing its process group on a deadline."""
    if (not isinstance(argv, list) or not argv or any(not isinstance(x, str) or '\0' in x for x in argv)
            or not Path(argv[0]).is_absolute()):
        raise ValueError('measurement command must be an absolute argv')
    if (not isinstance(env, dict) or any(not isinstance(k, str) or not isinstance(v, str) for k, v in env.items())
            or 'PERF_BUILDID_DIR' in env or 'PATH' not in env or 'HOME' not in env or 'LANG' not in env):
        raise ValueError('measurement environment must be explicit and omit PERF_BUILDID_DIR')
    if type(cpu) is not int or cpu not in os.sched_getaffinity(0):
        raise ValueError('measurement CPU is outside effective affinity')
    if (type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0
            or type(deadline) not in (int, float) or not math.isfinite(deadline) or deadline <= time.monotonic()):
        raise ValueError('measurement deadline has expired or timeout is invalid')
    cwd = Path(cwd).resolve(strict=True)
    stdout_path, stderr_path = (Path(p).absolute() for p in (stdout_path, stderr_path))
    if stdout_path == stderr_path:
        raise ValueError('stdout and stderr paths must be distinct')
    def child_affinity():
        os.sched_setaffinity(0, {cpu})
        if os.sched_getaffinity(0) != {cpu}:
            raise RuntimeError('child CPU affinity differs')
    started = time.monotonic()
    with stdout_path.open('xb') as stdout, stderr_path.open('xb') as stderr:
        process = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                   stdout=stdout, stderr=stderr, start_new_session=True,
                                   preexec_fn=child_affinity)
        try:
            code = process.wait(timeout=min(timeout, max(0.001, deadline - time.monotonic())))
        except BaseException:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
            raise
    return {'argv': argv, 'cwd': str(cwd), 'env': env, 'cpu': cpu, 'status': code,
            'seconds': time.monotonic() - started, 'timeout_s': timeout,
            'stdout': str(stdout_path), 'stderr': str(stderr_path)}
