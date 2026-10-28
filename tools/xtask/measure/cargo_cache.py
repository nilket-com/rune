"""Reconcile extracted Cargo registry sources with locked crate archives."""
import hashlib
from pathlib import Path
import tarfile
import tomllib

import input_closure


def _hash(path):
    pin = input_closure._pin(Path(path).parent, Path(path).name)
    return {'path': str(path), 'sha256': pin['sha256'], 'bytes': pin['bytes']}


def verify(lock_path, cache_root):
    lock = Path(lock_path).resolve(strict=True)
    cache = Path(cache_root).resolve(strict=True)
    lock_pin = _hash(lock)
    packages = tomllib.loads(lock.read_text())['package']
    crates, absent = [], []
    for package in packages:
        source = package.get('source', '')
        if not source:
            continue
        if source != 'registry+https://github.com/rust-lang/crates.io-index':
            raise ValueError('unsupported Cargo dependency source')
        name, version, checksum = package['name'], package['version'], package.get('checksum')
        identifier = name + '-' + version
        if '/' in identifier or '\\' in identifier or not identifier or not isinstance(checksum, str):
            raise ValueError('invalid locked crate identity')
        archives = sorted((cache / 'registry/cache').glob('*/' + identifier + '.crate'))
        sources = sorted((cache / 'registry/src').glob('*/' + identifier))
        if not archives and not sources:
            absent.append(identifier)
            continue
        if len(archives) != 1 or len(sources) != 1 or archives[0].is_symlink() or sources[0].is_symlink():
            raise ValueError('locked crate archive or extracted source is missing or ambiguous')
        archive, root = archives[0], sources[0]
        archive_pin = _hash(archive)
        if archive_pin['sha256'] != checksum:
            raise ValueError(f'locked crate archive checksum differs: {identifier}')
        files, names = [], set()
        with tarfile.open(archive, 'r:gz') as tar:
            for member in tar:
                if member.isdir():
                    continue
                if not member.isfile() or not member.name.startswith(identifier + '/'):
                    raise ValueError(f'unsupported crate archive entry: {identifier}')
                name = member.name[len(identifier) + 1:]
                input_closure._relative(name)
                if name in names:
                    raise ValueError(f'duplicate crate archive entry: {identifier}')
                names.add(name)
                stream = tar.extractfile(member)
                if stream is None:
                    raise ValueError('crate archive file is unreadable')
                digest = hashlib.sha256()
                with stream:
                    for block in iter(lambda: stream.read(1 << 20), b''):
                        digest.update(block)
                pin = input_closure._pin(root, name)
                if pin['sha256'] != digest.hexdigest() or pin['bytes'] != member.size:
                    raise ValueError(f'extracted crate source differs: {identifier}/{name}')
                files.append(pin)
        present = set()
        for path in root.rglob('*'):
            if path.is_symlink():
                raise ValueError(f'linked crate source: {identifier}')
            if path.is_file():
                present.add(str(path.relative_to(root)))
            elif not path.is_dir():
                raise ValueError(f'nonregular crate source: {identifier}')
        markers = present - names
        if markers - {'.cargo-ok', '.cargo-checksum.json'} or names - present:
            raise ValueError(f'extracted crate source inventory differs: {identifier}')
        marker_pins = [input_closure._pin(root, name) for name in sorted(markers)]
        if _hash(archive) != archive_pin:
            raise ValueError(f'crate archive changed during verification: {identifier}')
        crates.append({'name': package['name'], 'version': version, 'archive': archive_pin,
                       'source': str(root.resolve()), 'files': sorted(files, key=lambda item: item['path']),
                       'markers': marker_pins})
    if _hash(lock) != lock_pin:
        raise ValueError('Cargo lock changed during cache verification')
    return {'schema': 1, 'root': str(cache), 'lock': lock_pin, 'crates': crates, 'absent': sorted(absent)}
