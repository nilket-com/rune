"""Run one isolated offline measurement build from a verified source commit.

This worker requires an explicit lock, CPU allocation and tool identities. It
performs no measurement and never uses historical output as a build input.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import time
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import build_capsule
import build_recipe
import cargo_cache as registry_cache
import source_export
import tool_identity

ALIAS = '/tmp/rune-build'


def attest_memory(cgroup_root, membership, budget):
    """Require a kernel cgroup limit no larger than the declared build budget."""
    if type(budget) is not int or budget <= 0:
        raise ValueError('memory budget must be a positive integer')
    root = Path(cgroup_root).resolve(strict=True)
    relative = Path(membership)
    if not relative.is_absolute() or '..' in relative.parts:
        raise ValueError('invalid cgroup membership')
    current = (root / str(relative).lstrip('/')).resolve(strict=True)
    if not current.is_relative_to(root):
        raise ValueError('cgroup membership escapes its mount')
    limits = []
    for directory in (current, *current.parents):
        if not directory.is_relative_to(root):
            break
        path = directory / 'memory.max'
        if path.is_file():
            value = path.read_text().strip()
            if value != 'max':
                limit = int(value)
                if limit <= 0:
                    raise ValueError('invalid cgroup memory limit')
                limits.append(limit)
    if not limits or min(limits) > budget:
        raise ValueError('kernel cgroup memory limit does not enforce the declared budget')
    return {'declared_bytes': budget, 'effective_max_bytes': min(limits), 'membership': membership}


def _current_memory(budget):
    memberships = [line.split(':', 2)[2] for line in Path('/proc/self/cgroup').read_text().splitlines()
                   if line.startswith('0::')]
    if len(memberships) != 1:
        raise ValueError('a unified cgroup membership is required')
    return attest_memory('/sys/fs/cgroup', memberships[0], budget)


def _pin(path):
    path = Path(path).resolve(strict=True)
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1 << 20), b''):
            digest.update(block)
    return {'path': str(path), 'sha256': digest.hexdigest(), 'bytes': path.stat().st_size}


def _implementation_pins():
    root = Path(__file__).resolve().parent
    paths = [root / name for name in ('build_worker.py', 'build_capsule.py',
             'build_recipe.py', 'source_export.py', 'tool_identity.py', 'cargo_cache.py', 'input_closure.py')]
    if any(path.is_symlink() or not path.is_file() for path in paths):
        raise ValueError('build worker implementation must be regular files')
    return [_pin(path) for path in paths]


def _capture(command, out, label, *, env, cwd=None, timeout=60, cpus=None, read_stdout=True):
    stdout, stderr = out / (label + '.stdout'), out / (label + '.stderr')
    start = time.monotonic()
    process = None
    failure = None
    code = -1
    with stdout.open('xb') as so, stderr.open('xb') as se:
        affinity = (lambda: os.sched_setaffinity(0, cpus)) if cpus is not None else None
        try:
            process = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                       stdout=so, stderr=se, start_new_session=True, preexec_fn=affinity)
            code = process.wait(timeout=timeout)
        except BaseException as error:
            failure = f'{type(error).__name__}: {error}'
            if process is not None:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                code = process.wait()
    receipt = {'argv': command, 'cwd': str(cwd) if cwd else None, 'environment': env,
               'status': code, 'failure': failure, 'seconds': time.monotonic() - start,
               'stdout': _pin(stdout), 'stderr': _pin(stderr)}
    with (out / (label + '.json')).open('x', encoding='utf-8') as stream:
        json.dump(receipt, stream, indent=2, sort_keys=True)
        stream.write('\n')
    if failure is not None or code != 0:
        raise ValueError(f'{label} failed with status {code}: {failure}; see {stderr}')
    return stdout.read_text() if read_stdout else None


def _map_path(path, run, source, toolchain):
    virtual = Path(path)
    for alias, physical in ((Path(ALIAS + '/out'), run), (Path(ALIAS + '/source'), source),
                            (Path(ALIAS + '/harness'), run / 'harness'),
                            (Path(ALIAS + '/toolchain'), toolchain)):
        if virtual.is_relative_to(alias):
            return (physical / virtual.relative_to(alias)).resolve(strict=True)
    return virtual.resolve(strict=True)


def _trace(run, source, toolchain, rustc, lld, wrapper):
    files = sorted(run.glob('exec.trace.*'))
    if not files:
        raise ValueError('build produced no execution traces')
    rows = []
    for path in files:
        pin = _pin(path)
        for line in path.read_text().splitlines():
            row = tool_identity.parse_exec_line(line)
            if row is None:
                continue
            row['resolved'] = str(_map_path(row['path'], run, source, toolchain))
            row['trace'] = pin
            rows.append(row)
    selected = tool_identity.validate_invocations(rows, str(rustc), str(lld), str(wrapper))
    executable_paths = {row['resolved'] for row in rows}
    selected['executables'] = [_pin(path) for path in sorted(executable_paths)]
    selected['trace_files'] = [_pin(path) for path in files]
    return selected


def build(repo, revision, profile, native, out, *, role, toolchain, cargo_cache, lock_path,
          cpus, jobs, timeout, expected_rustc, expected_linker, memory_bytes=None,
          host_config=None, host_sha256=None):
    """Build one role in a new output tree under an exclusive supplied lock."""
    if role not in ('candidate', 'tip', 'base', 'milestone') or profile not in ('cgu1', 'cgu16'):
        raise ValueError('unknown source role or build profile')
    if type(native) is not bool or type(jobs) is not int or jobs < 1:
        raise ValueError('native must be boolean and jobs must be positive')
    if type(timeout) is not int or not 1 <= timeout <= 7200:
        raise ValueError('build timeout is out of bounds')
    if not isinstance(cpus, (list, tuple)) or not cpus or any(type(cpu) is not int or cpu < 0 for cpu in cpus):
        raise ValueError('a nonempty CPU allocation is required')
    if len(cpus) != len(set(cpus)):
        raise ValueError('CPU allocation contains duplicates')
    cpus = set(cpus)
    if not cpus <= os.sched_getaffinity(0) or jobs > len(cpus):
        raise ValueError('CPU allocation or job count exceeds effective affinity')
    if not isinstance(expected_rustc, str) or not expected_rustc or not isinstance(expected_linker, str) or not expected_linker:
        raise ValueError('declared compiler and linker identities are required')
    memory = _current_memory(memory_bytes) if memory_bytes is not None else None
    if (host_config is None) != (host_sha256 is None):
        raise ValueError('host configuration and its digest must be supplied together')
    host_receipt = None
    if host_config is not None:
        host_path = Path(host_config).resolve(strict=True)
        raw = host_path.read_bytes()
        if hashlib.sha256(raw).hexdigest() != host_sha256:
            raise ValueError('host configuration digest differs')
        host_receipt = {'path': str(host_path), 'sha256': host_sha256, 'contents': raw.decode('utf-8')}
    repo, toolchain, cargo_cache = (Path(p).resolve(strict=True) for p in (repo, toolchain, cargo_cache))
    out = Path(out).resolve(strict=False)
    lock_path = Path(lock_path)
    if not lock_path.is_absolute():
        raise ValueError('measurement lock path must be absolute')
    lock_resolved = lock_path.resolve(strict=False)
    for path in (repo, toolchain, cargo_cache):
        if out == path or out.is_relative_to(path) or path.is_relative_to(out):
            raise ValueError('build output overlaps a protected input')
        if lock_resolved == path or lock_resolved.is_relative_to(path):
            raise ValueError('measurement lock is inside a protected input')
    if lock_resolved == out or lock_resolved.is_relative_to(out):
        raise ValueError('measurement lock is inside the build output')
    out.mkdir(parents=True, exist_ok=False)
    result = {'schema': 1, 'status': 'FAIL', 'profile': profile, 'native': native, 'role': role}
    try:
        implementation = _implementation_pins()
        descriptor = os.open(lock_path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, 'a+b') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            export = source_export.export(repo, revision, out / 'export')
            source = Path(export['source'])
            run = out / 'run'
            run.mkdir()
            recipe = build_recipe.prepare(source, export['commit'], profile, run / 'harness',
                                          source_alias=ALIAS + '/source')
            registry = registry_cache.verify(run / 'harness/Cargo.lock', cargo_cache)
            rustc = (toolchain / 'bin/rustc').resolve(strict=True)
            cargo = (toolchain / 'bin/cargo').resolve(strict=True)
            base_env = {'PATH': '/usr/bin:/bin', 'HOME': str(run), 'LANG': 'C.UTF-8', 'TZ': 'UTC'}
            version = _capture([str(rustc), '-vV'], run, 'rustc-version', env=base_env)
            if version.splitlines()[0] != expected_rustc.replace(' version', '', 1):
                raise ValueError('compiler version differs from the declared identity')
            hosts = re.findall(r'^host: (\S+)$', version, re.M)
            if len(hosts) != 1:
                raise ValueError('compiler host identity is missing or ambiguous')
            host = hosts[0]
            lld = (toolchain / 'lib/rustlib' / host / 'bin/rust-lld').resolve(strict=True)
            wrapper = (toolchain / 'lib/rustlib' / host / 'bin/gcc-ld/ld.lld').resolve(strict=True)
            tool_paths = [rustc, cargo, lld, wrapper, Path('/usr/bin/bwrap'), Path('/usr/bin/strace'),
                          Path('/usr/bin/readelf'), Path('/usr/bin/python3'), Path('/usr/bin/git')]
            pins = [_pin(path) for path in tool_paths]
            sysroot = tool_identity.sysroot_identity(toolchain, host)
            capsule, env = build_capsule.prepare(run, source, run / 'harness', toolchain, cargo_cache)
            probe = """import os\nfrom pathlib import Path\nassert sorted(os.sched_getaffinity(0)) == EXPECTED\nassert Path(os.environ['CARGO_HOME'], 'config.toml').read_text() == '[net]\\noffline = true\\n'\nfor p in [Path.cwd(), *Path.cwd().parents]:\n for n in ('config', 'config.toml'):\n  assert not (p / '.cargo' / n).exists()\n""".replace('EXPECTED', repr(sorted(cpus)))
            _capture(capsule + ['--', '/usr/bin/python3', '-I', '-B', '-c', probe], run,
                     'namespace-probe', env=env, timeout=120, cpus=cpus)
            command = ['/usr/bin/strace', '-ff', '-ttt', '-s', '1048576', '-e', 'trace=execve',
                       '-o', ALIAS + '/out/exec.trace', ALIAS + '/toolchain/bin/cargo', 'build',
                       '--offline', '--release', '--locked', '-j', str(jobs), '--message-format=json']
            if native:
                command += ['--features', 'native']
            _capture(capsule + ['--', *command], run, 'build', env=env, timeout=timeout, cpus=cpus, read_stdout=False)
            invocations = _trace(run, source, toolchain, rustc, lld, wrapper)
            rune_compilations = tool_identity.validate_rune_feature(invocations['compilers'], native, recipe['native_feature'])
            binary = run / 'target/release/p2-probe-harness'
            if not binary.is_file():
                raise ValueError('measurement harness binary is missing')
            comments = _capture(['/usr/bin/readelf', '-p', '.comment', str(binary)], run,
                                'elf-comment', env=base_env)
            identity = tool_identity.elf_identity(comments, expected_rustc, expected_linker)
            sections = _capture(['/usr/bin/readelf', '-S', '-W', str(binary)], run,
                                'elf-sections', env=base_env)
            if re.search(r'\s\.debug_\w+', sections):
                raise ValueError('measurement binary has debug sections')
            if source_export.verify(source, export['commit'])['files'] != export['files']:
                raise ValueError('source changed during build')
            for name, expected in recipe['files'].items():
                if _pin(run / 'harness' / name)['sha256'] != expected['sha256']:
                    raise ValueError(f'build recipe changed: {name}')
            if build_recipe._harness_inputs() != recipe['harness_inputs']:
                raise ValueError('harness inputs changed during build')
            if registry_cache.verify(run / 'harness/Cargo.lock', cargo_cache) != registry:
                raise ValueError('Cargo registry cache changed during build')
            if _implementation_pins() != implementation:
                raise ValueError('build worker implementation changed during build')
            if tool_identity.sysroot_identity(toolchain, host) != sysroot or [_pin(p) for p in tool_paths] != pins:
                raise ValueError('compiler, linker, or sysroot changed during build')
            dep_info = sorted((run / 'target/release').rglob('*.d'))
            if not dep_info:
                raise ValueError('build produced no dependency information')
            if host_receipt is not None and hashlib.sha256(Path(host_receipt['path']).read_bytes()).hexdigest() != host_sha256:
                raise ValueError('host configuration changed during build')
            result.update(status='PASS', source=export, recipe=recipe, host=host, cpus=sorted(cpus),
                          jobs=jobs, memory=memory, host_config=host_receipt, lock=str(lock_path), implementation=implementation, tool_pins=pins, sysroot=sysroot, registry=registry,
                          invocations=invocations, rune_compilations=rune_compilations,
                          dep_info=[_pin(path) for path in dep_info], elf_identity=identity, binary=_pin(binary))
    except BaseException as error:
        result['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        with (out / 'RESULT.json').open('x', encoding='utf-8') as stream:
            json.dump(result, stream, indent=2, sort_keys=True)
            stream.write('\n')
    return result


def main():
    parser = argparse.ArgumentParser(description='Run one isolated commit-based measurement build')
    parser.add_argument('--repo', required=True)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--profile', required=True, choices=('cgu1', 'cgu16'))
    parser.add_argument('--role', required=True, choices=('candidate', 'tip', 'base', 'milestone'))
    parser.add_argument('--native', action='store_true')
    parser.add_argument('--out', required=True)
    parser.add_argument('--toolchain', required=True)
    parser.add_argument('--cargo-cache', required=True)
    parser.add_argument('--lock', required=True)
    parser.add_argument('--cpu', type=int, action='append', required=True)
    parser.add_argument('--jobs', type=int, required=True)
    parser.add_argument('--timeout', type=int, required=True)
    parser.add_argument('--rustc-identity', required=True)
    parser.add_argument('--linker-identity', required=True)
    parser.add_argument('--memory-bytes', type=int)
    parser.add_argument('--host-config')
    parser.add_argument('--host-sha256')
    args = parser.parse_args()
    build(args.repo, args.revision, args.profile, args.native, args.out, role=args.role,
          toolchain=args.toolchain, cargo_cache=args.cargo_cache, lock_path=args.lock,
          cpus=args.cpu, jobs=args.jobs, timeout=args.timeout,
          expected_rustc=args.rustc_identity, expected_linker=args.linker_identity,
          memory_bytes=args.memory_bytes, host_config=args.host_config, host_sha256=args.host_sha256)
    print('build PASS:', Path(args.out).resolve())


if __name__ == '__main__':
    main()
