"""Capture one local counter2 packet from six fresh commit-based builds.

This internal worker does not yet implement remote dispatch or the complete
measurement seal. Its output must be independently checked before recording.
"""
import hashlib
import argparse
import json
import os
from pathlib import Path
import shutil
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_package
import counter_cell
import counter_packet
import counter_schedule
import input_closure
import input_verification
import measurement_readiness
import measurement_runtime
import perf_parse
import role_evidence

IMPLEMENTATIONS = ('counter_controller.py', 'counter_cell.py', 'counter_packet.py', 'counter_schedule.py',
                   'counter_assess.py', 'input_closure.py', 'measurement_readiness.py', 'measurement_runtime.py',
                   'perf_parse.py', 'role_evidence.py', 'source_export.py', 'tool_identity.py', 'build_worker.py',
                   'build_capsule.py', 'build_recipe.py', 'analysis_package.py', 'analysis_sandbox.py',
                   'callgrind_packet.py', 'callgrind_trend.py', 'callgrind.py', 'callgrind_rules.py', 'qualification_policy.py',
                   'callgrind_guard.py', 'k1_analysis.py', 'k1_packet.py', 'k1.py', 'input_verification.py',
                   'gen_inputs.py', 'analysis_runtime.py', 'cargo_cache.py')


def _pin(path):
    path = Path(path).resolve(strict=True)
    record = input_closure._pin(path.parent, path.name)
    return {'path': str(path), 'sha256': record['sha256'], 'bytes': record['bytes']}


def _capture(repo, host, roles, inputs, seed, out, *, host_config, path_counts=None, request_spec=None, created):
    """Run the frozen counter2 schedule; all machine limits are supplied by host."""
    if not isinstance(host, dict):
        raise ValueError('measurement host configuration required')
    host_config = Path(host_config).resolve(strict=True)
    host_data = host_config.read_bytes()
    configured = json.loads(host_data)
    if (not isinstance(configured, dict) or configured.get('schema') != 1
            or set(configured) - {'schema', 'correctness', 'measurement'}
            or configured.get('measurement') != host):
        raise ValueError('host configuration does not match measurement host')
    for name in ('measurement_cpu', 'measurement_lock', 'measurement_max_load', 'measurement_min_available_bytes'):
        if name not in host:
            raise ValueError(f'measurement host requires {name}')
    cpu, lock = host['measurement_cpu'], Path(host['measurement_lock'])
    if type(cpu) is not int or cpu < 0 or not lock.is_absolute() or lock.is_symlink():
        raise ValueError('invalid measurement CPU or lock')
    if not isinstance(seed, str) or not seed or any(c not in '0123456789abcdef' for c in seed):
        raise ValueError('seed must be lowercase hexadecimal')
    if not isinstance(roles, dict) or set(roles) != set(counter_schedule.SUBJECTS):
        raise ValueError('six explicit measurement roles are required')
    repo = Path(repo).resolve(strict=True)
    inputs = Path(inputs).resolve(strict=True)
    out = Path(out).resolve(strict=False)
    lock = lock.resolve(strict=False)
    if any(out == p or out.is_relative_to(p) or p.is_relative_to(out) for p in (repo, inputs)):
        raise ValueError('measurement output overlaps protected inputs')
    if lock == out or lock.is_relative_to(out):
        raise ValueError('measurement lock must be outside the output')
    if any(lock == path or lock.is_relative_to(path) for path in (repo, inputs)):
        raise ValueError('measurement lock must be outside protected inputs')
    checked = {}
    for subject, role in roles.items():
        identity, config = subject.rsplit('-', 1)
        if role.get('name') != subject or role.get('role') != identity or role.get('native') is not (config == 'on'):
            raise ValueError('measurement role and build configuration differ')
        checked[subject] = role_evidence.check(repo, role)
    for identity in ('base', 'tip', 'candidate'):
        if checked[identity + '-off']['commit'] != checked[identity + '-on']['commit']:
            raise ValueError('on and off builds use different commits')
    if len({item['profile'] for item in checked.values()}) != 1:
        raise ValueError('counter packet builds use different profiles')
    manifest_path = inputs / 'MANIFEST.json'
    input_proof = input_verification.verify(inputs)
    manifest_data = manifest_path.read_bytes()
    manifest = json.loads(manifest_data)
    schedule = counter_schedule.workload(int(seed, 16), manifest)
    admissions = counter_schedule.admissions(manifest)
    kernels = Path(__file__).resolve().parent / 'kernels'
    env_probe = Path(__file__).resolve().parent / 'env_probe.py'
    code_root = Path(__file__).resolve().parent
    paths = [manifest_path, host_config, env_probe, Path('/usr/bin/perf'), Path('/usr/bin/env'), Path('/usr/bin/python3')]
    if request_spec is not None:
        if _pin(request_spec['path']) != request_spec:
            raise ValueError('measurement specification changed before capture')
        paths.append(Path(request_spec['path']))
    paths.extend(code_root / name for name in IMPLEMENTATIONS)
    paths.extend(inputs / item['path'] for item in input_proof['closure']['files'])
    for kernel, stream in counter_schedule.KERNELS:
        paths.append(kernels / (kernel + '.rn'))
        name = manifest['streams'][stream]['path']
        input_closure._relative(name)
        path = inputs / name
        if _pin(path)['sha256'] != manifest['streams'][stream]['sha256']:
            raise ValueError('measurement input stream differs from manifest')
        paths.append(path)
    for role in roles.values():
        paths.extend((Path(role['build']['path']), Path(role['binary']['path'])))
    if path_counts is not None:
        paths.append(Path(path_counts))
    pinned_paths = [(path, _pin(path)) for path in paths]
    pins = {item['path']: item['sha256'] for _, item in pinned_paths}
    if pins[str(host_config)] != hashlib.sha256(host_data).hexdigest():
        raise ValueError('host configuration changed during preparation')
    if pins[str(manifest_path.resolve())] != hashlib.sha256(manifest_data).hexdigest():
        raise ValueError('input manifest changed during preparation')
    runtime_paths = [Path('/usr/bin/perf'), Path('/usr/bin/env'), *(Path(role['binary']['path']) for role in roles.values())]
    runtime_dependencies = measurement_runtime.dependency_snapshot(runtime_paths)
    def check_pins():
        input_closure.verify(inputs, input_proof['closure'])
        for path, expected in pinned_paths:
            if _pin(path) != expected:
                raise ValueError('measurement input changed during run')
        if measurement_runtime.dependency_snapshot(runtime_paths) != runtime_dependencies:
            raise ValueError('measurement runtime dependencies changed')
    check_pins()
    out.mkdir(parents=True, exist_ok=False)
    created.append(True)
    session = out / 'session'
    session.mkdir()
    home = out / 'home'
    home.mkdir()
    shutil.copyfile(manifest_path, out / 'MANIFEST.json')
    schedule_bytes = (json.dumps(schedule, indent=2) + '\n').encode()
    (out / 'SCHEDULE.json').write_bytes(schedule_bytes)
    if path_counts is not None:
        shutil.copyfile(path_counts, out / 'PATH-COUNTS.json')
    spec = {'seed': seed, 'cpu': cpu, 'pins': pins,
            'binaries': {name: role['binary']['path'] for name, role in roles.items()},
            'logical_session': str(session), 'logical_inputs': str(inputs), 'env_probe': str(env_probe),
            'logical_kernels': str(kernels), 'environment': {'PATH': '/usr/bin:/bin', 'HOME': str(home), 'LANG': 'C.UTF-8'},
            'path_counts': 'PATH-COUNTS.json' if path_counts is not None else None}
    (out / 'SPEC.json').write_text(json.dumps(spec, indent=2) + '\n')
    env = {'PATH': '/usr/bin:/bin', 'HOME': str(home), 'LANG': 'C.UTF-8'}
    started = time.monotonic()
    deadline = started + 4800
    records_path = session / 'records.jsonl'
    count = 0
    with records_path.open('x') as log:
        def emit(record):
            log.write(json.dumps(record, sort_keys=True, allow_nan=False) + '\n')
            log.flush()
        def run_perf(lease, prefix, child):
            path = session / (prefix + '.perf')
            argv = counter_packet._perf_argv(path, child)
            receipt = lease.execute(argv, cwd=kernels, env=env, stdout_path=session / (prefix + '.stdout'),
                                    stderr_path=session / (prefix + '.stderr'), timeout=120, deadline=deadline)
            if receipt['cpu'] != cpu:
                raise ValueError('measurement child CPU differs from lease')
            perf = path.read_bytes()
            stdout = (session / (prefix + '.stdout')).read_bytes()
            stderr = (session / (prefix + '.stderr')).read_bytes()
            return argv, receipt['status'], perf, stdout, stderr, receipt
        with measurement_runtime.lease(lock, cpu) as lease:
            perf_policy = measurement_runtime.check_perf_policy(home)
            ready = lease.ready(max_load=host['measurement_max_load'],
                                min_available_bytes=host['measurement_min_available_bytes'])
            emit({'record': 'header', 'group': counter_packet.GROUP, 'cpu': cpu, 'pins': pins,
                  'context': ready['observation'], 'processes': ready['observation']['busy'], 'perf_policy': perf_policy,
                  'schedule_sha256': hashlib.sha256(schedule_bytes).hexdigest(),
                  'expected_probes': 3, 'expected_environments': 6, 'expected_admissions': 30,
                  'expected_workloads': 150, 'child_timeout_s': 120, 'whole_run_bound_s': 4800})
            try:
                for i in range(3):
                    argv, code, perf, stdout, stderr, execution = run_perf(lease, f'probe-{i}', ['/usr/bin/env', '-u', 'PERF_BUILDID_DIR', '/bin/true'])
                    parsed = perf_parse.classify_probe(code, stdout, stderr, perf, 'B1', counter_cell.EVENTS, counter_cell.GROUPS)
                    emit({'record': 'probe', 'index': i, 'argv': argv, 'rc': code, 'result': parsed, 'execution': execution})
                    if not parsed['probe_ok']:
                        raise ValueError('counter probe refused')
                def subject(ident, prefix, kind, index):
                    nonlocal count
                    kernel, stream, k, name = (ident[x] for x in ('kernel', 'stream', 'k', 'subject'))
                    child = ['/usr/bin/env', '-u', 'PERF_BUILDID_DIR', roles[name]['binary']['path'],
                             kernel + '.rn', str(k), str(inputs / manifest['streams'][stream]['path']), '-']
                    before = measurement_readiness.snapshot(cpu)
                    argv, code, perf, stdout, stderr, execution = run_perf(lease, prefix, child)
                    after = measurement_readiness.snapshot(cpu)
                    parsed = perf_parse.parse(perf_parse.decode(perf), 'B1', counter_cell.EVENTS, counter_cell.GROUPS)
                    operable, issues = perf_parse.probe_ok(parsed)
                    maps = int(name.endswith('-on') and kernel in ('while', 'numeric', 'calls'))
                    checksum = manifest['cells'][kernel + '/' + stream]['expected_checksum'][str(k)]
                    expected = f'CHECKSUM {checksum} invocations={k}\nMAPS anon_exec_mappings={maps} anon_exec_kib={maps * 4}\n'.encode()
                    valid = code == 0 and not stderr and stdout == expected and operable
                    emit({'record': kind, 'index': index, 'identity': ident, 'argv': argv, 'rc': code,
                          'valid': valid, 'counters': parsed, 'issues': issues, 'ready': before, 'after': after,
                          'perf_sha256': hashlib.sha256(perf).hexdigest(), 'stdout_sha256': hashlib.sha256(stdout).hexdigest(),
                          'execution': execution})
                    if not valid:
                        raise ValueError('counter admission or workload failed')
                    if kind == 'row':
                        count += 1
                offset = 0
                for name in counter_schedule.SUBJECTS:
                    child = ['/usr/bin/env', '-u', 'PERF_BUILDID_DIR', '/usr/bin/python3', '-I', '-B', str(env_probe)]
                    argv, code, perf, stdout, stderr, execution = run_perf(lease, 'env-' + name, child)
                    parsed = perf_parse.parse(perf_parse.decode(perf), 'B1', counter_cell.EVENTS, counter_cell.GROUPS)
                    keys = json.loads(stdout)
                    if (code != 0 or stderr or not perf_parse.probe_ok(parsed)[0] or not isinstance(keys, dict)
                            or 'PERF_BUILDID_DIR' in keys or any(not isinstance(k, str) or type(v) is not int or v < 0 for k, v in keys.items())):
                        raise ValueError('counter environment probe failed')
                    emit({'record': 'environment', 'subject': name, 'argv': argv, 'rc': code, 'keys_and_lengths': keys,
                          'perf': parsed, 'execution': execution})
                    for ident in admissions[offset:offset + 5]:
                        kernel = ident['kernel']
                        subject(ident, 'admission-' + name + '-' + kernel, 'admission', name + '/' + kernel)
                    offset += 5
                for i, ident in enumerate(schedule):
                    subject(ident, f'row-{i:03}', 'row', i)
                check_pins()
                emit({'record': 'footer', 'complete': count == 150, 'workloads': count, 'stopped': None,
                      'traceback': None, 'elapsed_s': time.monotonic() - started, 'context': measurement_readiness.snapshot(cpu),
                      'processes': []})
            except BaseException as error:
                emit({'record': 'footer', 'complete': False, 'workloads': count,
                      'stopped': f'{type(error).__name__}: {error}', 'traceback': 'see worker error',
                      'elapsed_s': time.monotonic() - started})
                raise
            analysis_manifest = analysis_package.inspect(out, 'counter2-v1')
            analysis = analysis_package.analyze(out, analysis_manifest)
            check_pins()
    rejected = any(cell['instruction_rule']['rejected'] for cell in analysis['cells'])
    cycle_flags = [{'config': cell['config'], 'kernel': cell['kernel']}
                   for cell in analysis['cells'] if cell['cycles_report']['flagged']]
    result = {'schema': 1, 'status': 'REJECT' if rejected else 'PASS', 'protocol': 'counter2-v1', 'out': str(out),
              'inputs': str(inputs), 'code_root': str(code_root), 'request_cwd': str(Path.cwd().resolve()),
              'roles': checked, 'host': host, 'host_config': {'path': str(host_config),
              'sha256': hashlib.sha256(host_data).hexdigest(), 'contents': host_data.decode('utf-8')},
              'request_spec': request_spec, 'input_proof': input_proof, 'runtime_dependencies': runtime_dependencies,
              'analysis_manifest': analysis_manifest, 'analysis': analysis, 'cycle_flags': cycle_flags}
    (out / 'RESULT.json').write_text(json.dumps(result, indent=2, sort_keys=True, allow_nan=False) + '\n')
    return result


def run(repo, host, roles, inputs, seed, out, *, host_config, path_counts=None, request_spec=None):
    """Capture into a new output and preserve a typed failure if capture starts."""
    output = Path(out).resolve(strict=False)
    created = []
    started = time.monotonic()
    try:
        return _capture(repo, host, roles, inputs, seed, output,
                        host_config=host_config, path_counts=path_counts, request_spec=request_spec, created=created)
    except BaseException as error:
        if created and output.is_dir():
            failure = {'schema': 1, 'status': 'FAIL', 'protocol': 'counter2-v1', 'out': str(output),
                       'error': f'{type(error).__name__}: {error}', 'seconds': time.monotonic() - started}
            with (output / 'FAILURE.json').open('x', encoding='utf-8') as stream:
                json.dump(failure, stream, indent=2, sort_keys=True, allow_nan=False)
                stream.write('\n')
        raise


def main():
    parser = argparse.ArgumentParser(description='Capture a local counter2-v1 packet')
    parser.add_argument('--repo', required=True)
    parser.add_argument('--spec', required=True)
    parser.add_argument('--out', required=True)
    parser.add_argument('--hosts', required=True)
    parser.add_argument('--hosts-sha256', required=True)
    args = parser.parse_args()
    spec_path = Path(args.spec).resolve(strict=True)
    spec_pin = _pin(spec_path)
    spec_data = spec_path.read_bytes()
    if hashlib.sha256(spec_data).hexdigest() != spec_pin['sha256']:
        raise ValueError('measurement specification changed while reading')
    spec = json.loads(spec_data)
    if (not isinstance(spec, dict) or set(spec) != {'schema', 'roles', 'inputs', 'seed', 'path_counts'}
            or spec['schema'] != 1 or not isinstance(spec['inputs'], str)
            or (spec['path_counts'] is not None and not isinstance(spec['path_counts'], str))):
        raise ValueError('invalid counter2 measurement request')
    host_path = Path(args.hosts).resolve(strict=True)
    host_data = host_path.read_bytes()
    if hashlib.sha256(host_data).hexdigest() != args.hosts_sha256:
        raise ValueError('host configuration differs from the xtask receipt')
    host = json.loads(host_data).get('measurement')
    result = run(args.repo, host, spec['roles'], spec['inputs'], spec['seed'], args.out,
                 host_config=host_path, path_counts=spec['path_counts'], request_spec=spec_pin)
    print(f"counter2 {result['status']}: {result['out']}")
    if result['status'] != 'PASS':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
