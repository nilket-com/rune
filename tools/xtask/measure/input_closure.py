"""Create and verify explicit, source-relative build or analysis inputs."""
import hashlib
import os
from pathlib import Path
import stat


def _relative(name):
    if not isinstance(name, str) or not name or '\\' in name or '\0' in name:
        raise ValueError('invalid input path')
    path = Path(name)
    if path.is_absolute() or any(part in ('', '.', '..') for part in name.split('/')):
        raise ValueError('input path must be normalized and relative')
    return path


def _pin(root, name):
    root = Path(root).resolve(strict=True)
    path = root / _relative(name)
    for part in (path, *path.parents):
        if part == root:
            break
        if part.is_symlink():
            raise ValueError(f'linked input is not allowed: {name}')
    digest = hashlib.sha256()
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    except OSError as error:
        raise ValueError(f'input file cannot be opened safely: {name}') from error
    with os.fdopen(descriptor, 'rb') as stream:
        before = os.fstat(stream.fileno())
        if not stat.S_ISREG(before.st_mode):
            raise ValueError(f'input is not a regular file: {name}')
        for block in iter(lambda: stream.read(1 << 20), b''):
            digest.update(block)
        after = os.fstat(stream.fileno())
    try:
        current = path.stat(follow_symlinks=False)
    except OSError as error:
        raise ValueError(f'input disappeared while reading: {name}') from error
    identity = lambda value: (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
    if identity(before) != identity(after) or identity(after) != identity(current):
        raise ValueError(f'input changed while reading: {name}')
    return {'path': name, 'sha256': digest.hexdigest(), 'bytes': after.st_size}


def create(root, paths):
    """Pin every declared file; callers choose optional paths before calling."""
    if (not isinstance(paths, (list, tuple)) or any(not isinstance(name, str) for name in paths)
            or len(paths) != len(set(paths))):
        raise ValueError('input paths must be a unique list')
    return {'schema': 1, 'files': [_pin(root, name) for name in sorted(paths)]}


def verify(root, manifest):
    """Recheck every pin, rejecting unknown fields, missing files and changes."""
    if not isinstance(manifest, dict) or set(manifest) != {'schema', 'files'} or manifest['schema'] != 1:
        raise ValueError('invalid input closure schema')
    files = manifest['files']
    if not isinstance(files, list):
        raise ValueError('input closure files must be a list')
    names = []
    for record in files:
        if not isinstance(record, dict) or set(record) != {'path', 'sha256', 'bytes'}:
            raise ValueError('invalid input pin')
        if type(record['bytes']) is not int or record['bytes'] < 0:
            raise ValueError('invalid input size')
        if not isinstance(record['sha256'], str) or len(record['sha256']) != 64 or any(c not in '0123456789abcdef' for c in record['sha256']):
            raise ValueError('invalid input digest')
        names.append(record['path'])
        if _pin(root, record['path']) != record:
            raise ValueError(f'input changed: {record["path"]}')
    if names != sorted(set(names)):
        raise ValueError('input pins must be unique and sorted')
    return True
