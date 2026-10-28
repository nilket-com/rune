"""Discover and pin the explicit Python and binutils runtime for analyzers."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sysconfig
import stat

import input_closure


def _file(path):
    path = Path(path).resolve(strict=True)
    if not path.is_file():
        raise ValueError('runtime input is not a regular file')
    pin = input_closure._pin(path.parent, path.name)
    return {'path': str(path), 'sha256': pin['sha256'], 'bytes': pin['bytes'],
            'mode': stat.S_IMODE(path.stat().st_mode)}


def _tree(root):
    root = Path(root).resolve(strict=True)
    entries = []
    for directory, dirs, files in os.walk(root, followlinks=False):
        for name in sorted(dirs + files):
            path = Path(directory) / name
            relative = str(path.relative_to(root))
            if path.is_symlink():
                entries.append({'path': relative, 'link': os.readlink(path)})
            elif path.is_file():
                pin = _file(path)
                entries.append({'path': relative, 'sha256': pin['sha256'], 'bytes': pin['bytes'], 'mode': pin['mode']})
            elif path.is_dir():
                entries.append({'path': relative, 'directory': True, 'mode': stat.S_IMODE(path.stat().st_mode)})
            else:
                raise ValueError('nonregular Python runtime input')
    entries.sort(key=lambda item: item['path'])
    encoded = json.dumps(entries, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()
    return {'root': str(root), 'files': len(entries), 'sha256': hashlib.sha256(encoded).hexdigest()}


def _dependencies(executables, *, require_complete=False):
    dependencies = {}
    for executable in executables:
        result = subprocess.run(['/usr/bin/ldd', str(executable)], stdin=subprocess.DEVNULL,
                                capture_output=True, text=True, env={'PATH': '/usr/bin:/bin', 'LANG': 'C'}, check=False)
        if result.returncode not in (0, 1):
            raise ValueError('cannot inspect analyzer runtime dependencies')
        if require_complete and 'not found' in result.stdout:
            raise ValueError('runtime dependency is missing')
        for line in result.stdout.splitlines():
            match = re.search(r'(?:=>\s*)?(/[^\s]+)\s+\(0x[0-9a-f]+\)', line)
            if match:
                target = match[1]
                pin = _file(target)
                previous = dependencies.setdefault(target, pin)
                if previous != pin:
                    raise ValueError('runtime dependency changed while discovering it')
    return [{'target': target, **pin} for target, pin in sorted(dependencies.items())]


def discover():
    stdlib = Path(sysconfig.get_path('stdlib')).resolve(strict=True)
    dynload = Path(sysconfig.get_config_var('DESTSHARED')).resolve(strict=True)
    if not dynload.is_relative_to(stdlib) or not stdlib.is_relative_to('/usr'):
        raise ValueError('unsupported Python runtime layout')
    executables = [Path('/usr/bin/python3'), Path('/usr/bin/nm'), Path('/usr/bin/objdump')]
    extensions = sorted(path for path in dynload.glob('*.so') if path.is_file())
    if not extensions:
        raise ValueError('Python runtime extension modules are missing')
    return {'stdlib': _tree(stdlib), 'executables': [_file(path) for path in executables],
            'dependencies': _dependencies([*executables, *extensions]), 'sandbox': _file('/usr/bin/bwrap'),
            'discovery_tools': [_file('/usr/bin/ldd'), _file('/bin/sh')]}
