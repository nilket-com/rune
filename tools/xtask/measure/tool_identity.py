"""Validate tool identities observed in a freshly built ELF comment section."""
import re
import json
import hashlib
from pathlib import Path


def parse_exec_line(line):
    """Parse a successful strace execve record; other lines are not invocations."""
    match = re.match(r'^\S+ execve\(("(?:[^"\\]|\\.)*"), (\[.*\]), .*\)\s+= 0$', line)
    if not match:
        return None
    path, argv = json.loads(match[1]), json.loads(match[2])
    if not isinstance(path, str) or not isinstance(argv, list) or not all(isinstance(arg, str) for arg in argv):
        raise ValueError('invalid traced execution record')
    return {'path': path, 'argv': argv}


def validate_invocations(rows, rustc, lld, wrapper=None):
    """Validate actual compiler and linker executions against supplied identities."""
    compilers = [r for r in rows if '--crate-name' in r['argv'] and Path(r['path']).name == 'rustc']
    if not compilers or any(r['resolved'] != rustc for r in compilers):
        raise ValueError('missing or unexpected actual compiler invocation')
    linkers = [r for r in rows if Path(r['path']).name in ('rust-lld', 'ld.lld', 'ld', 'mold')]
    if not linkers or not any(r['resolved'] == lld for r in linkers):
        raise ValueError('actual rust-lld invocation is missing')
    for row in linkers:
        name = Path(row['path']).name
        if name == 'rust-lld' and row['resolved'] == lld:
            continue
        if (name == 'ld.lld' and wrapper is not None and row['resolved'] == wrapper
                and any(other['resolved'] == lld and other['trace']['path'] == row['trace']['path']
                        for other in linkers)):
            continue
        raise ValueError('unexpected linker or wrapper execution')
    driver_pattern = r'(?:cc|c\+\+|gcc(?:-\d+)?|g\+\+(?:-\d+)?|clang(?:\+\+)?(?:-\d+)?)'
    drivers = [r for r in rows if re.fullmatch(driver_pattern, Path(r['path']).name)]
    return {'compilers': compilers, 'linkers': linkers, 'linker_drivers': drivers}


def validate_rune_feature(compilers, native, native_feature="native-code"):
    """Require an actual Rune compilation with the declared native feature state."""
    if type(native) is not bool:
        raise ValueError('native feature state must be boolean')
    if native_feature not in ('native-code', 'fresh-core-native'):
        raise ValueError('unsupported native feature name')
    rune = []
    for row in compilers:
        argv = row['argv']
        for index, value in enumerate(argv[:-1]):
            if value == '--crate-name' and argv[index + 1] == 'rune':
                rune.append(argv)
                break
    if not rune:
        raise ValueError('actual Rune compilation is missing')
    for argv in rune:
        enabled = any(value == '--cfg' and argv[index + 1] == f'feature="{native_feature}"'
                      for index, value in enumerate(argv[:-1]))
        if enabled != native:
            raise ValueError('actual Rune native feature differs from the declared state')
    return len(rune)


def sysroot_identity(root, host):
    """Inventory the compiler and target libraries that can affect a build."""
    if not isinstance(host, str) or not re.fullmatch(r'[A-Za-z0-9_.-]+', host) or host in ('.', '..'):
        raise ValueError('invalid compiler host triple')
    root = Path(root).resolve(strict=True)
    library = root / 'lib' / 'rustlib' / host / 'lib'
    if not library.is_dir():
        raise ValueError('target standard libraries are missing')
    paths = {path.resolve(strict=True) for path in library.rglob('*') if path.is_file()}
    paths.update(path.resolve(strict=True) for path in (root / 'lib').glob('*.so*') if path.is_file())
    if not paths:
        raise ValueError('compiler and target libraries are missing')
    files = []
    for path in sorted(paths):
        digest = hashlib.sha256()
        with path.open('rb') as stream:
            for block in iter(lambda: stream.read(1 << 20), b''):
                digest.update(block)
        files.append({'path': str(path), 'sha256': digest.hexdigest(), 'bytes': path.stat().st_size})
    canonical = (json.dumps(files, sort_keys=True, indent=2, ensure_ascii=True, allow_nan=False) + '\n').encode()
    return {'root': str(root), 'files': files, 'sha256': hashlib.sha256(canonical).hexdigest()}


def elf_identity(text, expected_rustc, expected_linker):
    if not isinstance(expected_rustc, str) or not expected_rustc or not isinstance(expected_linker, str) or not expected_linker:
        raise ValueError('both expected tool identities are required')
    lines = re.findall(r'^\s*\[\s*[0-9a-f]+\]\s+(.*)$', text, re.M)
    rustc = [line.strip() for line in lines if line.strip().startswith('rustc version ')]
    linkers = [line.strip() for line in lines if line.strip().startswith(('Linker: ', 'mold ', 'GNU ld '))]
    if rustc != [expected_rustc]:
        raise ValueError('ELF compiler identity differs from the declared identity')
    if linkers != [expected_linker]:
        raise ValueError('ELF linker identity differs from the declared identity')
    return {'rustc': rustc[0], 'linker': linkers[0]}
