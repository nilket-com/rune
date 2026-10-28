"""Atomic persistence for a previously validated ledger transition.

This is an internal primitive, not an evidence or seal validator. A caller must
check the sealed evidence while holding the same lock before it invokes append.
"""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile

import ledger_transition


def append(path, lock_path, expected_sha256, entry, milestone, check_evidence):
    """Recheck evidence under the lock, then atomically replace the ledger."""
    path = Path(path).absolute()
    lock_path = Path(lock_path).absolute()
    if path == lock_path or path.is_symlink() or lock_path.is_symlink():
        raise ValueError('ledger and lock must be distinct regular paths')
    if not callable(check_evidence):
        raise ValueError('an evidence validation callback is required')
    flags = os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK
    with os.fdopen(os.open(lock_path, flags, 0o600), 'r+b') as lock:
        if not stat.S_ISREG(os.fstat(lock.fileno()).st_mode):
            raise ValueError('ledger lock must be a regular file')
        fcntl.flock(lock, fcntl.LOCK_EX)
        with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), 'rb') as stream:
            old_stat = os.fstat(stream.fileno())
            if not stat.S_ISREG(old_stat.st_mode):
                raise ValueError('ledger must be a regular file')
            original = stream.read()
        if hashlib.sha256(original).hexdigest() != expected_sha256:
            raise ValueError('ledger changed since it was read')
        before = json.loads(original)
        after, status = ledger_transition.transition(before, entry, milestone)
        check_evidence()
        with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), 'rb') as stream:
            current = os.fstat(stream.fileno())
            if not stat.S_ISREG(current.st_mode) or hashlib.sha256(stream.read()).hexdigest() != expected_sha256:
                raise ValueError('ledger changed during evidence validation')
        if status == 'already-present':
            return status
        data = (json.dumps(after, indent=2, ensure_ascii=False, allow_nan=False) + '\n').encode()
        name = None
        try:
            with tempfile.NamedTemporaryFile(dir=path.parent, prefix='.ledger-', delete=False) as out:
                name = out.name
                os.fchmod(out.fileno(), old_stat.st_mode & 0o777)
                out.write(data)
                out.flush()
                os.fsync(out.fileno())
            os.replace(name, path)
            name = None
            directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        finally:
            if name is not None:
                os.unlink(name)
        return status
