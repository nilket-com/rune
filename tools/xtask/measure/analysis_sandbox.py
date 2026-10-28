"""Run an analyzer with only pinned package and code files mounted.

This primitive does not declare protocol inputs; each completed analyzer must
supply its own complete manifest and prove its branches with this sandbox.
"""
from pathlib import Path
import json
import subprocess
import tempfile

import input_closure
import analysis_runtime


def _mount(command, root, manifest, destination):
    directories = set()
    for item in manifest['files']:
        path = Path(item['path'])
        parent = path.parent
        while str(parent) != '.':
            directories.add(parent.as_posix())
            parent = parent.parent
    command += ['--dir', destination]
    for directory in sorted(directories, key=lambda name: (name.count('/'), name)):
        command += ['--dir', destination + '/' + directory]
    for item in manifest['files']:
        command += ['--ro-bind', str(root / item['path']), destination + '/' + item['path']]


def run(package_root, package_manifest, code_root, code_manifest, entry, *, timeout=60, runtime=None,
        verified_facts=None):
    """Run a pinned Python analyzer; return stdout only on an isolated success."""
    package = Path(package_root).resolve(strict=True)
    code = Path(code_root).resolve(strict=True)
    if package == code or package.is_relative_to(code) or code.is_relative_to(package):
        raise ValueError('code and package roots must be disjoint')
    if any(root == Path('/usr') or root.is_relative_to('/usr') for root in (package, code)):
        raise ValueError('package and code must be outside system tools')
    if type(timeout) is not int or not 1 <= timeout <= 1800:
        raise ValueError('invalid analyzer timeout')
    input_closure.verify(package, package_manifest)
    input_closure.verify(code, code_manifest)
    if runtime is None:
        runtime = analysis_runtime.discover()
    elif analysis_runtime.discover() != runtime:
        raise ValueError('analyzer runtime differs from its manifest')
    input_closure._relative(entry)
    if entry not in {item['path'] for item in code_manifest['files']}:
        raise ValueError('analyzer entry is not pinned')
    command = ['/usr/bin/bwrap', '--unshare-all', '--new-session', '--die-with-parent', '--tmpfs', '/']
    targets = [(pin['path'], target) for pin, target in zip(runtime['executables'],
               ('/usr/bin/python3', '/usr/bin/nm', '/usr/bin/objdump'))]
    for pin in runtime['dependencies']:
        target = pin['target']
        if target.startswith('/lib/'):
            target = '/usr' + target
        targets.append((pin['path'], target))
    stdlib = runtime['stdlib']['root']
    directories = {'/usr', '/usr/bin', '/usr/lib', '/tmp', '/lib64'}
    for target in [stdlib, *(target for _, target in targets)]:
        parent = Path(target).parent
        while str(parent) != '/':
            directories.add(str(parent))
            parent = parent.parent
    for directory in sorted(directories, key=lambda name: (name.count('/'), name)):
        command += ['--dir', directory]
    command += ['--symlink', 'usr/lib', '/lib', '--ro-bind', stdlib, stdlib]
    for source, target in targets:
        if not Path(target).is_relative_to(stdlib):
            command += ['--ro-bind', source, target]
    _mount(command, package, package_manifest, '/inputs')
    _mount(command, code, code_manifest, '/tool')
    with tempfile.TemporaryDirectory(prefix='rune-analysis-proof-') as temp:
        proof_path = None
        proof_bytes = None
        if verified_facts is not None:
            proof_path = Path(temp) / 'proof.json'
            proof_bytes = json.dumps(verified_facts, sort_keys=True, allow_nan=False).encode()
            proof_path.write_bytes(proof_bytes)
            command += ['--ro-bind', str(proof_path), '/verified-proof.json']
        command += ['--chdir', '/inputs', '--clearenv', '--setenv', 'LANG', 'C.UTF-8',
                    '--setenv', 'HOME', '/tmp', '--', '/usr/bin/python3', '-I', '-B', '-S', '/tool/' + entry]
        if proof_path is not None:
            command += ['/verified-proof.json']
        result = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True,
                                timeout=timeout, check=False)
        if proof_path is not None and proof_path.read_bytes() != proof_bytes:
            raise ValueError('fresh qualification proof changed during analysis')
    input_closure.verify(package, package_manifest)
    input_closure.verify(code, code_manifest)
    if analysis_runtime.discover() != runtime:
        raise ValueError('analyzer runtime changed during execution')
    if result.returncode:
        raise ValueError(f'analyzer failed in isolated package: {result.returncode}: '
                         + result.stderr.decode(errors='replace')[:1000])
    return result.stdout
