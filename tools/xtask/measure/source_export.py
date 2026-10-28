"""Export and verify a Git commit for a fresh measurement build."""
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess


def _git(repo, *args):
    env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
    env.update(GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_SYSTEM='/dev/null',
               GIT_CONFIG_NOSYSTEM='1', GIT_OPTIONAL_LOCKS='0', GIT_NO_REPLACE_OBJECTS='1')
    command = ['/usr/bin/git', '-c', 'core.autocrlf=false', '-c', 'core.hooksPath=/dev/null',
               '-c', 'core.attributesFile=/dev/null', '-c', 'core.filemode=true',
               '-C', str(repo), *args]
    result = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True, env=env, check=False)
    if result.returncode:
        raise ValueError(f'Git failed for {args}: {result.stderr.decode(errors="replace").strip()}')
    return result.stdout


def _blob_oid(data):
    return hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()


def _tree(repo, commit):
    result = {}
    for item in _git(repo, 'ls-tree', '-r', '-z', '--full-tree', commit).split(b'\0'):
        if not item:
            continue
        metadata, raw_name = item.split(b'\t', 1)
        mode, kind, oid = metadata.decode('ascii').split()
        name = raw_name.decode('utf-8')
        parts = name.split('/')
        if (not name or name.startswith('/') or any(part in ('', '.', '..') for part in parts)
                or '\\' in name or kind != 'blob' or mode not in ('100644', '100755')):
            raise ValueError(f'unsupported Git source path or mode: {name}')
        result[name] = (mode, oid)
    return result


def verify(root, commit):
    """Verify every tracked byte and mode and reject untracked or linked inputs."""
    root = Path(root).resolve(strict=True)
    if not isinstance(commit, str) or not re.fullmatch('[0-9a-f]{40}', commit):
        raise ValueError('a full Git SHA-1 commit is required')
    tree = _tree(root, commit)
    if _git(root, 'rev-parse', 'HEAD').decode().strip() != commit:
        raise ValueError('checkout commit differs from the requested commit')
    if _git(root, 'status', '--porcelain', '--untracked-files=all'):
        raise ValueError('export checkout is dirty')
    present = set()
    for directory, dirs, names in os.walk(root, followlinks=False):
        if Path(directory) == root:
            dirs[:] = [name for name in dirs if name != '.git']
        for name in dirs + names:
            path = Path(directory) / name
            if path.is_symlink():
                raise ValueError(f'linked source input: {path}')
        for name in names:
            path = Path(directory) / name
            if not path.is_file():
                raise ValueError(f'nonregular source input: {path}')
            present.add(str(path.relative_to(root)))
    if present != set(tree):
        raise ValueError('export contains missing or undeclared files')
    files = []
    for name, (mode, oid) in sorted(tree.items()):
        path = root / name
        if path.is_symlink() or any(parent.is_symlink() for parent in path.parents if parent != root and root in parent.parents):
            raise ValueError(f'linked source input: {name}')
        if not path.is_file():
            raise ValueError(f'missing source input: {name}')
        actual_mode = '100755' if path.stat().st_mode & stat.S_IXUSR else '100644'
        data = path.read_bytes()
        if actual_mode != mode or _blob_oid(data) != oid:
            raise ValueError(f'source input differs from Git: {name}')
        files.append({'path': name, 'git_mode': mode, 'git_blob': oid,
                      'sha256': hashlib.sha256(data).hexdigest(), 'bytes': len(data)})
    return {'commit': commit, 'tree': _git(root, 'rev-parse', commit + '^{tree}').decode().strip(),
            'files': files}


def export(repo, revision, out):
    """Create a new commit checkout outside the source repository and return its inventory."""
    repo = Path(repo).resolve(strict=True)
    out = Path(out).resolve(strict=False)
    if out == repo or out.is_relative_to(repo) or repo.is_relative_to(out):
        raise ValueError('export output and source repository must be disjoint')
    if not isinstance(revision, str) or not revision or revision.startswith('-'):
        raise ValueError('invalid Git revision')
    if _git(repo, 'rev-parse', '--show-object-format').decode().strip() != 'sha1':
        raise ValueError('only Git SHA-1 repositories are supported')
    commit = _git(repo, 'rev-parse', '--verify', revision + '^{commit}').decode().strip()
    if not re.fullmatch('[0-9a-f]{40}', commit):
        raise ValueError('invalid resolved commit')
    out.mkdir(parents=True, exist_ok=False)
    source = out / 'source'
    _git(out, 'clone', '--no-local', '--no-hardlinks', '--no-checkout', '--quiet', str(repo), str(source))
    _git(source, 'checkout', '--detach', '--quiet', commit)
    inventory = verify(source, commit)
    receipt = {'schema': 1, 'status': 'PASS', 'requested_revision': revision,
               'source': str(source), **inventory}
    with (out / 'SOURCE.json').open('x', encoding='utf-8') as stream:
        json.dump(receipt, stream, sort_keys=True, indent=2, allow_nan=False)
        stream.write('\n')
    return receipt


def verify_signed(signing_repo, source, commit):
    """Reconcile a verified export with an authenticated commit and tree."""
    signing_repo = Path(signing_repo).resolve(strict=True)
    if not isinstance(commit, str) or not re.fullmatch('[0-9a-f]{40}', commit):
        raise ValueError('a full signed commit identity is required')
    if _git(signing_repo, 'rev-parse', '--verify', commit + '^{commit}').decode().strip() != commit:
        raise ValueError('signed commit does not resolve exactly')
    # Preserve the user's Git trust configuration, but reject environment overrides.
    env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
    env['GIT_NO_REPLACE_OBJECTS'] = '1'
    result = subprocess.run(['/usr/bin/git', '-C', str(signing_repo), 'verify-commit', commit],
                            stdin=subprocess.DEVNULL, capture_output=True, env=env, check=False)
    if result.returncode:
        raise ValueError('commit signature verification failed')
    if _git(signing_repo, 'log', '-1', '--format=%b', commit).strip():
        raise ValueError('signed commit must not have a body')
    actual = verify(source, commit)
    tree = _tree(signing_repo, commit)
    if actual['tree'] != _git(signing_repo, 'rev-parse', commit + '^{tree}').decode().strip():
        raise ValueError('signed tree differs from the export')
    if {item['path']: (item['git_mode'], item['git_blob']) for item in actual['files']} != tree:
        raise ValueError('signed source entries differ from the export')
    return {'status': 'PASS', 'commit': commit, 'tree': actual['tree'],
            'tracked_files': len(tree), 'signature_verified': True}


def verify_signed_snapshot(signing_repo, native_source, commit, ignored_inputs):
    """Reconcile a standalone source snapshot and declared extras with a signed tree."""
    import input_closure

    repo = Path(signing_repo).resolve(strict=True)
    root = Path(native_source).resolve(strict=True)
    if not isinstance(commit, str) or not re.fullmatch('[0-9a-f]{40}', commit):
        raise ValueError('a full signed commit identity is required')
    if _git(repo, 'rev-parse', '--verify', commit + '^{commit}').decode().strip() != commit:
        raise ValueError('signed commit does not resolve exactly')
    env = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
    env['GIT_NO_REPLACE_OBJECTS'] = '1'
    result = subprocess.run(['/usr/bin/git', '-C', str(repo), 'verify-commit', commit],
                            stdin=subprocess.DEVNULL, capture_output=True, env=env, check=False)
    if result.returncode or _git(repo, 'log', '-1', '--format=%b', commit).strip():
        raise ValueError('commit signature invalid or commit has a body')
    tree = _tree(repo, commit)
    if not isinstance(ignored_inputs, list):
        raise ValueError('ignored inputs must be a list')
    extras = {}
    for item in ignored_inputs:
        if not isinstance(item, dict) or set(item) != {'path', 'sha256', 'bytes', 'git_mode'}:
            raise ValueError('invalid ignored input pin')
        name = item['path']
        input_closure._relative(name)
        if (name in tree or name in extras or item['git_mode'] not in ('100644', '100755')
                or type(item['bytes']) is not int or item['bytes'] < 0
                or not isinstance(item['sha256'], str)
                or not re.fullmatch('[0-9a-f]{64}', item['sha256'])):
            raise ValueError('duplicate, tracked or invalid ignored input')
        extras[name] = item
    present = set()
    for directory, dirs, names in os.walk(root, followlinks=False):
        for name in dirs + names:
            if (Path(directory) / name).is_symlink():
                raise ValueError('linked source input')
        for name in names:
            path = Path(directory) / name
            if not path.is_file():
                raise ValueError('nonregular source input')
            present.add(str(path.relative_to(root)))
    if present != set(tree) | set(extras):
        raise ValueError('snapshot has missing or undeclared inputs')
    for name, (mode, oid) in tree.items():
        pin = input_closure._pin(root, name)
        path = root / name
        data = path.read_bytes()
        if (hashlib.sha256(data).hexdigest() != pin['sha256'] or _blob_oid(data) != oid
                or ('100755' if path.stat().st_mode & stat.S_IXUSR else '100644') != mode):
            raise ValueError(f'snapshot differs from signed tree: {name}')
    for name, item in extras.items():
        pin = input_closure._pin(root, name)
        if (pin != {key: item[key] for key in ('path', 'sha256', 'bytes')}
                or ('100755' if (root / name).stat().st_mode & stat.S_IXUSR else '100644') != item['git_mode']):
            raise ValueError(f'ignored input differs from declaration: {name}')
    return {'status': 'PASS', 'commit': commit,
            'tree': _git(repo, 'rev-parse', commit + '^{tree}').decode().strip(),
            'tracked_files': len(tree), 'ignored_files': len(extras), 'signature_verified': True}
