"""Capture a local K1 packet from fresh commit-based measurement builds."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_package
import counter_cell
import counter_controller
import counter_packet
import k1_packet
import measurement_readiness
import measurement_runtime
import perf_parse
import role_evidence

IMPLEMENTATIONS = ('k1_controller.py', 'k1_packet.py', 'k1.py', 'counter_controller.py', 'counter_cell.py',
                   'counter_packet.py', 'counter_schedule.py', 'counter_assess.py', 'input_closure.py',
                   'input_verification.py', 'gen_inputs.py', 'measurement_readiness.py', 'measurement_runtime.py',
                   'perf_parse.py', 'role_evidence.py', 'source_export.py', 'tool_identity.py', 'build_worker.py',
                   'build_capsule.py', 'build_recipe.py', 'analysis_package.py', 'analysis_sandbox.py',
                   'callgrind_packet.py', 'callgrind_trend.py', 'callgrind.py', 'callgrind_rules.py', 'qualification_policy.py',
                   'callgrind_guard.py', 'k1_analysis.py', 'analysis_runtime.py', 'cargo_cache.py')


def _capture(repo, host, roles, profile, out, *, host_config, request_spec, created):
    if not isinstance(host, dict):
        raise ValueError('measurement host configuration required')
    for name in ('measurement_cpu', 'measurement_lock', 'measurement_max_load', 'measurement_min_available_bytes'):
        if name not in host:
            raise ValueError(f'measurement host requires {name}')
    host_config = Path(host_config).resolve(strict=True)
    host_data = host_config.read_bytes()
    configured = json.loads(host_data)
    if (not isinstance(configured, dict) or configured.get('schema') != 1
            or set(configured) - {'schema', 'correctness', 'measurement'} or configured.get('measurement') != host):
        raise ValueError('host configuration does not match measurement host')
    cpu, lock = host['measurement_cpu'], Path(host['measurement_lock'])
    if type(cpu) is not int or cpu < 0 or not lock.is_absolute() or lock.is_symlink():
        raise ValueError('invalid measurement CPU or lock')
    if profile not in ('cgu1', 'cgu16'):
        raise ValueError('invalid K1 build profile')
    names = k1_packet.subjects(roles)
    repo = Path(repo).resolve(strict=True)
    out = Path(out).resolve(strict=False)
    lock = lock.resolve(strict=False)
    if out == repo or out.is_relative_to(repo) or repo.is_relative_to(out):
        raise ValueError('measurement output overlaps repository')
    if lock == out or lock.is_relative_to(out):
        raise ValueError('measurement lock must be outside the output')
    if lock == repo or lock.is_relative_to(repo):
        raise ValueError('measurement lock must be outside protected inputs')
    checked = {}
    for name in names:
        role, mode = name.rsplit('-', 1)
        entry = roles[name]
        if (not isinstance(entry, dict) or entry.get('name') != name or entry.get('role') != role
                or entry.get('native') is not (mode == 'on') or entry.get('profile') != profile):
            raise ValueError('K1 role and build configuration differ')
        checked[name] = role_evidence.check(repo, entry)
    for role in {name.rsplit('-', 1)[0] for name in names}:
        if checked[role + '-off']['commit'] != checked[role + '-on']['commit']:
            raise ValueError('on and off K1 builds use different commits')
    code = Path(__file__).resolve().parent
    program, input_path = code / 'kernels/k1.rn', code / 'kernels/k1.inputs.txt'
    paths = [host_config, program, input_path, Path('/usr/bin/perf'), Path('/usr/bin/env'), Path('/usr/bin/python3')]
    paths.extend(code / name for name in IMPLEMENTATIONS)
    if request_spec is not None:
        if counter_controller._pin(request_spec['path']) != request_spec:
            raise ValueError('measurement specification changed before capture')
        paths.append(Path(request_spec['path']))
    for entry in roles.values():
        paths.extend((Path(entry['build']['path']), Path(entry['binary']['path'])))
    pinned = [(path, counter_controller._pin(path)) for path in paths]
    pins = {pin['path']: pin['sha256'] for _, pin in pinned}
    runtime_paths = [Path('/usr/bin/perf'), Path('/usr/bin/env'), *(Path(roles[name]['binary']['path']) for name in names)]
    runtime_dependencies = measurement_runtime.dependency_snapshot(runtime_paths)
    if counter_controller._pin(host_config)['sha256'] != hashlib.sha256(host_data).hexdigest():
        raise ValueError('host configuration changed during preparation')
    def check_pins():
        if any(counter_controller._pin(path) != pin for path, pin in pinned):
            raise ValueError('K1 input changed during measurement')
        if measurement_runtime.dependency_snapshot(runtime_paths) != runtime_dependencies:
            raise ValueError('K1 runtime dependencies changed')
    check_pins()
    out.mkdir(parents=True, exist_ok=False)
    created.append(True)
    home = out / 'home'
    home.mkdir()
    env = {'PATH': '/usr/bin:/bin', 'HOME': str(home), 'LANG': 'C.UTF-8'}
    spec = {'profile': profile, 'cpu': cpu, 'pins': pins, 'binaries': {name: roles[name]['binary']['path'] for name in names},
            'logical_session': str(out), 'logical_program': str(program), 'logical_input': str(input_path), 'environment': env}
    (out / 'SPEC.json').write_text(json.dumps(spec, indent=2) + '\n')
    started = time.monotonic()
    deadline = started + 120 * (5 * len(names) + 1)
    count = 0
    with (out / 'records.jsonl').open('x') as log:
        def emit(row):
            log.write(json.dumps(row, sort_keys=True, allow_nan=False) + '\n')
            log.flush()
        with measurement_runtime.lease(lock, cpu) as lease:
            perf_policy = measurement_runtime.check_perf_policy(home)
            ready = lease.ready(max_load=host['measurement_max_load'],
                                min_available_bytes=host['measurement_min_available_bytes'])
            emit({'record': 'header', 'cpu': cpu, 'profile': profile, 'pins': pins,
                  'context': ready['observation'], 'perf_policy': perf_policy})
            try:
                for repeat in range(5):
                    for name in names:
                        prefix = f'{name}-stat-{repeat}'
                        child = ['/usr/bin/env', '-u', 'PERF_BUILDID_DIR', roles[name]['binary']['path'], str(program), '1', str(input_path), '-']
                        argv = counter_packet._perf_argv(out / (prefix + '.perf'), child)
                        execution = lease.execute(argv, cwd=out, env=env, stdout_path=out / (prefix + '.stdout'),
                                                  stderr_path=out / (prefix + '.stderr'), timeout=120, deadline=deadline)
                        perf = (out / (prefix + '.perf')).read_bytes()
                        stdout = (out / (prefix + '.stdout')).read_bytes()
                        stderr = (out / (prefix + '.stderr')).read_bytes()
                        parsed = perf_parse.parse(perf_parse.decode(perf), 'B1', counter_cell.EVENTS, counter_cell.GROUPS)
                        if (execution['cpu'] != cpu or execution['status'] != 0 or stderr
                                or stdout != b'CHECKSUM 0 invocations=1\nMAPS anon_exec_mappings=0 anon_exec_kib=0\n'
                                or not perf_parse.probe_ok(parsed)[0]):
                            raise ValueError('K1 sample failed')
                        emit({'record': 'sample', 'name': name, 'repeat': repeat, 'argv': argv,
                              'execution': execution, 'counters': parsed})
                        count += 1
                check_pins()
                emit({'record': 'footer', 'complete': count == 5 * len(names), 'samples': count,
                      'context': measurement_readiness.snapshot(cpu), 'elapsed_s': time.monotonic() - started})
            except BaseException as error:
                emit({'record': 'footer', 'complete': False, 'samples': count, 'error': f'{type(error).__name__}: {error}'})
                raise
            analysis_manifest = analysis_package.inspect(out, 'k1-packet-v1')
            analysis = analysis_package.analyze(out, analysis_manifest)
            check_pins()
    rejected = profile == 'cgu1' and any(not row['tier1_pass'] for row in analysis['ratios'].values())
    result = {'schema': 1, 'status': 'REJECT' if rejected else 'PASS', 'protocol': 'k1-v1', 'out': str(out),
              'code_root': str(code), 'request_cwd': str(Path.cwd().resolve()),
              'roles': checked, 'host': host, 'host_config': {'path': str(host_config),
              'sha256': hashlib.sha256(host_data).hexdigest(), 'contents': host_data.decode('utf-8')},
              'request_spec': request_spec, 'runtime_dependencies': runtime_dependencies,
              'analysis_manifest': analysis_manifest, 'analysis': analysis}
    (out / 'RESULT.json').write_text(json.dumps(result, indent=2, sort_keys=True, allow_nan=False) + '\n')
    return result


def run(repo, host, roles, profile, out, *, host_config, request_spec=None):
    output, created, started = Path(out).resolve(strict=False), [], time.monotonic()
    try:
        return _capture(repo, host, roles, profile, output, host_config=host_config,
                        request_spec=request_spec, created=created)
    except BaseException as error:
        if created and output.is_dir():
            failure = {'schema': 1, 'status': 'FAIL', 'protocol': 'k1-v1', 'out': str(output),
                       'error': f'{type(error).__name__}: {error}', 'seconds': time.monotonic() - started}
            with (output / 'FAILURE.json').open('x', encoding='utf-8') as stream:
                json.dump(failure, stream, indent=2, sort_keys=True, allow_nan=False)
                stream.write('\n')
        raise


def main():
    parser = argparse.ArgumentParser(description='Capture a local K1 packet')
    for flag in ('repo', 'spec', 'out', 'hosts', 'hosts-sha256'):
        parser.add_argument('--' + flag, required=True)
    args = parser.parse_args()
    spec_path = Path(args.spec).resolve(strict=True)
    spec_pin = counter_controller._pin(spec_path)
    spec_data = spec_path.read_bytes()
    if hashlib.sha256(spec_data).hexdigest() != spec_pin['sha256']:
        raise ValueError('measurement specification changed while reading')
    spec = json.loads(spec_data)
    if not isinstance(spec, dict) or set(spec) != {'schema', 'profile', 'roles'} or spec['schema'] != 1:
        raise ValueError('invalid K1 measurement request')
    host_path = Path(args.hosts).resolve(strict=True)
    host_data = host_path.read_bytes()
    if hashlib.sha256(host_data).hexdigest() != args.hosts_sha256:
        raise ValueError('host configuration differs from the xtask receipt')
    host = json.loads(host_data).get('measurement')
    result = run(args.repo, host, spec['roles'], spec['profile'], args.out,
                 host_config=host_path, request_spec=spec_pin)
    print(f"K1 {result['status']}: {result['out']}")
    if result['status'] != 'PASS':
        raise SystemExit(1)


if __name__ == '__main__':
    main()
