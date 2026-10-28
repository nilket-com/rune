"""Capture local callgrind controls, CGU1 gates and CGU16 trends."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_package
import callgrind
import callgrind_capture
import control_proof
import counter_controller
import gen_inputs
import input_closure
import input_verification
import measurement_readiness
import measurement_runtime
import qualification_policy
import role_evidence

KERNELS = ('while', 'numeric', 'calls', 'fib', 'shared_rows')
STREAMS = {'while': 'loop_main', 'numeric': 'loop_main', 'calls': 'calls_main',
           'fib': 'fib_main', 'shared_rows': 'shared_rows_main'}
IMPLEMENTATIONS = ('callgrind_controller.py', 'callgrind_capture.py', 'callgrind_packet.py', 'callgrind_trend.py', 'callgrind.py',
                   'callgrind_rules.py', 'qualification_policy.py', 'control_proof.py', 'qualifications/s8.json',
                   'milestone_registry.py', 'milestones.json',
                   'callgrind_guard.py', 'counter_controller.py', 'counter_packet.py',
                   'counter_cell.py', 'counter_schedule.py', 'counter_assess.py', 'input_closure.py',
                   'input_verification.py', 'gen_inputs.py', 'measurement_readiness.py', 'measurement_runtime.py',
                   'perf_parse.py', 'role_evidence.py', 'source_export.py', 'tool_identity.py', 'build_worker.py',
                   'build_capsule.py', 'build_recipe.py', 'analysis_package.py', 'analysis_sandbox.py',
                   'k1_analysis.py', 'k1_packet.py', 'k1.py', 'analysis_runtime.py', 'cargo_cache.py')


def required_roles():
    return {f'{role}-{profile}-{mode}' for profile, roles in
            (('cgu1', ('base', 'tip', 'candidate')), ('cgu16', ('milestone', 'tip', 'candidate')))
            for role in roles for mode in ('off', 'on')}


def valgrind_version(path):
    return subprocess.run([str(path), '--version'], stdin=subprocess.DEVNULL, capture_output=True,
                          text=True, env={'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}, timeout=30, check=False)


def _capture(repo, host, roles, inputs, out, *, host_config, request_spec, symbol_map, qualifications,
             record, baseline_milestone, created):
    if not isinstance(host, dict):
        raise ValueError('measurement host configuration required')
    for field in ('measurement_cpu', 'measurement_lock', 'measurement_max_load', 'measurement_min_available_bytes',
                  'valgrind_path', 'valgrind_root', 'valgrind_identity'):
        if field not in host:
            raise ValueError(f'measurement host requires {field}')
    host_config = Path(host_config).resolve(strict=True)
    host_data = host_config.read_bytes()
    config = json.loads(host_data)
    if (not isinstance(config, dict) or config.get('schema') != 1
            or set(config) - {'schema', 'correctness', 'measurement'} or config.get('measurement') != host):
        raise ValueError('host configuration does not match measurement host')
    cpu, lock, valgrind = host['measurement_cpu'], Path(host['measurement_lock']), Path(host['valgrind_path'])
    valgrind_root = Path(host['valgrind_root'])
    if (type(cpu) is not int or cpu < 0 or not lock.is_absolute() or lock.is_symlink()
            or not valgrind.is_absolute() or not valgrind_root.is_absolute()
            or not isinstance(host['valgrind_identity'], str) or not host['valgrind_identity']):
        raise ValueError('invalid callgrind host configuration')
    valgrind_root = valgrind_root.resolve(strict=True)
    if not valgrind.resolve(strict=True).is_relative_to(valgrind_root):
        raise ValueError('valgrind executable is outside its declared installation')
    if not isinstance(roles, dict) or set(roles) != required_roles():
        raise ValueError('callgrind requires twelve explicit build roles')
    repo, inputs = Path(repo).resolve(strict=True), Path(inputs).resolve(strict=True)
    out, lock = Path(out).resolve(strict=False), lock.resolve(strict=False)
    if any(out == path or out.is_relative_to(path) or path.is_relative_to(out) for path in (repo, inputs, valgrind_root)):
        raise ValueError('measurement output overlaps protected inputs')
    if lock == out or lock.is_relative_to(out):
        raise ValueError('measurement lock must be outside the output')
    if any(lock == path or lock.is_relative_to(path) for path in (repo, inputs, valgrind_root)):
        raise ValueError('measurement lock must be outside protected inputs')
    checked = {}
    for name, entry in roles.items():
        role, profile, mode = name.split('-')
        if (not isinstance(entry, dict) or entry.get('name') != name or entry.get('role') != role
                or entry.get('profile') != profile or entry.get('native') is not (mode == 'on')):
            raise ValueError('callgrind role and build configuration differ')
        checked[name] = role_evidence.check(repo, entry)
    for role in ('candidate', 'tip'):
        if len({checked[f'{role}-{profile}-{mode}']['commit'] for profile in ('cgu1', 'cgu16') for mode in ('off', 'on')}) != 1:
            raise ValueError('callgrind role builds use different commits')
    for role, profile in (('base', 'cgu1'), ('milestone', 'cgu16')):
        if checked[f'{role}-{profile}-off']['commit'] != checked[f'{role}-{profile}-on']['commit']:
            raise ValueError('callgrind baseline builds use different commits')
    input_proof = input_verification.verify(inputs)
    manifest_path = inputs / 'MANIFEST.json'
    manifest_data = manifest_path.read_bytes()
    manifest = json.loads(manifest_data)
    code = Path(__file__).resolve().parent
    if qualifications is not None:
        qualification_policy.bind_record(qualifications, record, baseline_milestone,
                                         roles['candidate-cgu1-off']['commit'])
        expected_policy = json.loads((code / 'qualifications/s8.json').read_text())
        expected_policy['candidate_commit'] = roles['candidate-cgu1-off']['commit']
        if qualifications != expected_policy:
            raise ValueError('qualifications differ from the checked-in S8 policy')
    elif record is not None or baseline_milestone is not None:
        raise ValueError('record binding requires explicit qualifications')
    paths = [host_config, valgrind, manifest_path, Path('/usr/bin/python3'), Path('/usr/bin/nm'), Path('/usr/bin/objdump')]
    paths.extend(code / name for name in IMPLEMENTATIONS)
    paths.extend(inputs / item['path'] for item in input_proof['closure']['files'])
    for stream in set(STREAMS.values()):
        name = manifest['streams'][stream]['path']
        input_closure._relative(name)
        path = inputs / name
        if counter_controller._pin(path)['sha256'] != manifest['streams'][stream]['sha256']:
            raise ValueError('callgrind stream differs from manifest')
        paths.append(path)
    paths.extend(code / 'kernels' / (name + '.rn') for name in (*KERNELS, 'control_is', 'control_branch'))
    for entry in roles.values():
        paths.extend((Path(entry['build']['path']), Path(entry['binary']['path'])))
    if request_spec is not None:
        if counter_controller._pin(request_spec['path']) != request_spec:
            raise ValueError('measurement request changed before capture')
        paths.append(Path(request_spec['path']))
    if symbol_map is not None:
        paths.append(Path(symbol_map))
    pinned = [(path, counter_controller._pin(path)) for path in paths]
    pin_map = {pin['path']: pin['sha256'] for _, pin in pinned}
    for stream in set(STREAMS.values()):
        path = (inputs / manifest['streams'][stream]['path']).resolve(strict=True)
        if pin_map[str(path)] != manifest['streams'][stream]['sha256']:
            raise ValueError('callgrind stream changed during preparation')
    if counter_controller._pin(host_config)['sha256'] != hashlib.sha256(host_data).hexdigest():
        raise ValueError('host configuration changed during preparation')
    if counter_controller._pin(manifest_path)['sha256'] != hashlib.sha256(manifest_data).hexdigest():
        raise ValueError('input manifest changed during preparation')
    valgrind_tree = measurement_runtime.tool_tree(valgrind_root)
    runtime_paths = [valgrind, *(Path(entry['binary']['path']) for entry in roles.values())]
    runtime_dependencies = measurement_runtime.dependency_snapshot(runtime_paths)
    def check_pins():
        input_closure.verify(inputs, input_proof['closure'])
        if any(counter_controller._pin(path) != pin for path, pin in pinned):
            raise ValueError('callgrind input changed during measurement')
        if (measurement_runtime.tool_tree(valgrind_root) != valgrind_tree
                or measurement_runtime.dependency_snapshot(runtime_paths) != runtime_dependencies):
            raise ValueError('callgrind runtime changed during measurement')
    check_pins()
    version = valgrind_version(valgrind)
    if version.returncode or version.stdout.strip() != host['valgrind_identity']:
        raise ValueError('valgrind identity differs from host configuration')
    out.mkdir(parents=True, exist_ok=False)
    created.append(True)
    (out / 'home').mkdir()
    for mode in ('cgu1', 'cgu16'):
        (out / mode / 'data').mkdir(parents=True)
    for stream in set(STREAMS.values()):
        source = inputs / manifest['streams'][stream]['path']
        for profile in ('cgu1', 'cgu16'):
            shutil.copyfile(source, out / profile / 'data' / (stream + '.txt'))
    for value in (0, 100):
        (out / 'cgu1' / 'data' / f'control-{value}.txt').write_text((str(value) + '\n') * 1000)
    if symbol_map is not None:
        for profile in ('cgu1', 'cgu16'):
            shutil.copyfile(symbol_map, out / profile / 'SYMBOL-NAME-MAP.json')
    env = {'PATH': '/usr/bin:/bin', 'HOME': str(out / 'home'), 'LANG': 'C.UTF-8'}
    started, runs = time.monotonic(), []
    deadline = started + 180 * 69
    with measurement_runtime.lease(lock, cpu) as lease:
        ready = lease.ready(max_load=host['measurement_max_load'], min_available_bytes=host['measurement_min_available_bytes'])
        def capture(profile, role, mode, label, program, stream, expected):
            package = out / profile
            result_name = label + '.callgrind'
            args = [str(valgrind), '--tool=callgrind', '--dump-instr=yes', '--collect-jumps=yes',
                    '--callgrind-out-file=' + str(package / result_name), roles[f'{role}-{profile}-{mode}']['binary']['path'],
                    str(program), '1000', str(stream), '-']
            execution = lease.execute(args, cwd=code / 'kernels', env=env, stdout_path=package / (label + '.stdout'),
                                      stderr_path=package / (label + '.stderr'), timeout=180, deadline=deadline)
            profile_path = package / result_name
            row = {'label': label, 'role': f'{role}-{profile}-{mode}', 'execution': execution,
                   'profile': counter_controller._pin(profile_path) if profile_path.is_file() else None,
                   'stdout': counter_controller._pin(package / (label + '.stdout')),
                   'stderr': counter_controller._pin(package / (label + '.stderr'))}
            with (out / 'RUNS.jsonl').open('a', encoding='utf-8') as log:
                log.write(json.dumps(row, sort_keys=True, allow_nan=False) + '\n')
                log.flush()
            if execution['cpu'] != cpu or execution['status'] != 0:
                raise ValueError('callgrind measurement process failed')
            if row['profile'] is None:
                raise ValueError('callgrind profile is missing')
            lines = (package / (label + '.stdout')).read_text().splitlines()
            if not lines or lines[0] != f'CHECKSUM {expected} invocations=1000':
                raise ValueError('callgrind checksum differs from its model')
            parsed = callgrind.parse(profile_path)
            runs.append(dict(row, total_Ir=parsed['total_Ir']))
            return {'profile': result_name, 'stdout': label + '.stdout'}
        def guard(mode):
            directory = out / 'cgu1' / 'guard' / mode
            directory.mkdir(parents=True)
            result = {}
            for role in ('tip', 'candidate'):
                entry = roles[f'{role}-cgu1-{mode}']
                build = json.loads(Path(entry['build']['path']).read_bytes())
                source = Path(build['source']['source']) / 'crates/rune/src/experimental_core/heap/hot.rs'
                for kind, src in (('source', source), ('binary', Path(entry['binary']['path']))):
                    dst = directory / (role + '-' + kind)
                    shutil.copyfile(src, dst)
                    if counter_controller._pin(src)['sha256'] != counter_controller._pin(dst)['sha256']:
                        raise ValueError('guard input copy differs')
                    result[role + '_' + kind] = str(dst.relative_to(out / 'cgu1'))
            return result
        cg1 = {'positions': 'instr line', 'symbol_map': 'SYMBOL-NAME-MAP.json' if symbol_map is not None else None,
               'controls': {'profiles': {}, 'guard': guard('off')}, 'phases': {}}
        if qualifications is not None:
            cg1['qualifications'] = qualifications
        on_guard = guard('on')
        generated = [(path, counter_controller._pin(path)) for package in (out / 'cgu1', out / 'cgu16')
                     for path in package.rglob('*') if path.is_file()]
        def check_generated():
            if any(counter_controller._pin(path) != pin for path, pin in generated):
                raise ValueError('callgrind generated input changed during measurement')
        check_generated()
        for name in ('is', 'branch'):
            controls = {}
            for role in ('tip', 'candidate'):
                for n in (0, 100):
                    model = 0 if n == 0 else (300 if name == 'is' else 150)
                    expected = gen_inputs.checksum([model] * 1000)
                    controls[f'{role}{n}'] = capture('cgu1', role, 'off', f'{role}-{name}-{n}',
                        code / 'kernels' / ('control_' + name + '.rn'), out / 'cgu1' / 'data' / f'control-{n}.txt', expected)
            cg1['controls']['profiles'][name] = controls
        for mode in ('off', 'on'):
            cells = {}
            for kernel in KERNELS:
                stream = inputs / manifest['streams'][STREAMS[kernel]]['path']
                xs = [int(value) for value in stream.read_text().splitlines()[:1000]]
                if len(xs) != 1000:
                    raise ValueError('callgrind input stream is incomplete')
                expected = gen_inputs.checksum([gen_inputs.model(kernel, value) for value in xs])
                cell = {}
                for role, output_role in (('tip', 'tip'), ('candidate', 'candidate'), ('base', 'reference')):
                    cell[output_role] = capture('cgu1', role, mode, f'{output_role}-{mode}-{kernel}',
                                                code / 'kernels' / (kernel + '.rn'), stream, expected)
                cell['input_file'] = 'data/' + STREAMS[kernel] + '.txt'
                cells[kernel] = cell
            cg1['phases'][mode] = {'cells': cells, 'guard': cg1['controls']['guard'] if mode == 'off' else on_guard}
        cg16 = {'positions': 'instr line', 'symbol_map': 'SYMBOL-NAME-MAP.json' if symbol_map is not None else None, 'phases': {}}
        for mode in ('off', 'on'):
            cells = {}
            for kernel in KERNELS:
                stream = inputs / manifest['streams'][STREAMS[kernel]]['path']
                xs = [int(value) for value in stream.read_text().splitlines()[:1000]]
                expected = gen_inputs.checksum([gen_inputs.model(kernel, value) for value in xs])
                cell = {role: capture('cgu16', role, mode, f'{role}-{mode}-{kernel}',
                                      code / 'kernels' / (kernel + '.rn'), stream, expected)
                        for role in ('tip', 'candidate', 'milestone')}
                cell['input_file'] = 'data/' + STREAMS[kernel] + '.txt'
                cells[kernel] = cell
            cg16['phases'][mode] = cells
        if len(runs) != 68:
            raise ValueError('callgrind capture count differs')
        for profile, spec in (('cgu1', cg1), ('cgu16', cg16)):
            (out / profile / 'SPEC.json').write_text(json.dumps(spec, indent=2) + '\n')
        (out / 'RUNS.json').write_text(json.dumps(runs, indent=2, sort_keys=True, allow_nan=False) + '\n')
        check_pins()
        check_generated()
        capture_receipt = callgrind_capture.verify(out, roles=roles, host=host, inputs=inputs, code=code)
        manifests = {profile: analysis_package.inspect(out / profile, 'callgrind-' + profile + '-v1')
                     for profile in ('cgu1', 'cgu16')}
        # Persist the capture context before analysis can fail.  A failed
        # analyzer must not erase the readiness observations or the binding
        # between the 68 raw runs and their verified input packages.
        capture_context = {
            'schema': 1, 'status': 'CAPTURED', 'protocol': 'callgrind-v1',
            'out': str(out), 'inputs': str(inputs), 'code_root': str(code),
            'request_cwd': str(Path.cwd().resolve()), 'roles': checked, 'host': host,
            'host_config': {'path': str(host_config), 'sha256': hashlib.sha256(host_data).hexdigest(),
                            'contents': host_data.decode('utf-8')},
            'request_spec': request_spec, 'input_proof': input_proof,
            'valgrind_identity': version.stdout.strip(), 'valgrind_tree': valgrind_tree,
            'runtime_dependencies': runtime_dependencies,
            'input_pins': {pin['path']: pin for _, pin in pinned},
            'generated_pins': {pin['path']: pin for _, pin in generated},
            'readiness': ready, 'after_capture': measurement_readiness.snapshot(cpu),
            'runs': len(runs), 'capture_receipt': capture_receipt, 'analysis_manifests': manifests,
        }
        with (out / 'CAPTURE-CONTEXT.json').open('x', encoding='utf-8') as stream:
            json.dump(capture_context, stream, indent=2, sort_keys=True, allow_nan=False)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
        capture_context_pin = counter_controller._pin(out / 'CAPTURE-CONTEXT.json')
        facts = control_proof.prove(repo, roles, code / 'kernels') if qualifications is not None else None
        qualification_policy.validate(qualifications, facts)
        if qualifications is not None:
            qualification_policy.bind_roles(facts, roles)
        analyses = {profile: analysis_package.analyze(out / profile, manifests[profile],
                    verified_facts=facts if profile == 'cgu1' else None) for profile in ('cgu1', 'cgu16')}
        check_pins()
        check_generated()
        if counter_controller._pin(out / 'CAPTURE-CONTEXT.json') != capture_context_pin:
            raise ValueError('callgrind capture context changed during analysis')
        final_context = measurement_readiness.snapshot(cpu)
    result = {'schema': 1, 'status': 'PASS', 'protocol': 'callgrind-v1', 'out': str(out),
              'inputs': str(inputs), 'code_root': str(code), 'request_cwd': str(Path.cwd().resolve()),
              'roles': checked, 'host': host, 'host_config': {'path': str(host_config),
              'sha256': hashlib.sha256(host_data).hexdigest(), 'contents': host_data.decode('utf-8')},
              'request_spec': request_spec, 'input_proof': input_proof, 'valgrind_identity': version.stdout.strip(),
              'valgrind_tree': valgrind_tree, 'runtime_dependencies': runtime_dependencies,
              'input_pins': {pin['path']: pin for _, pin in pinned},
              'generated_pins': {pin['path']: pin for _, pin in generated},
              'readiness': ready, 'after': final_context, 'runs': len(runs), 'capture_receipt': capture_receipt,
              'analysis_manifests': manifests, 'analysis': analyses, 'capture_context': capture_context_pin}
    (out / 'RESULT.json').write_text(json.dumps(result, indent=2, sort_keys=True, allow_nan=False) + '\n')
    return result


def run(repo, host, roles, inputs, out, *, host_config, request_spec=None, symbol_map=None, qualifications=None,
        record=None, baseline_milestone=None):
    output, created, started = Path(out).resolve(strict=False), [], time.monotonic()
    try:
        return _capture(repo, host, roles, inputs, output, host_config=host_config,
                        request_spec=request_spec, symbol_map=symbol_map, qualifications=qualifications,
                        record=record, baseline_milestone=baseline_milestone, created=created)
    except BaseException as error:
        if created and output.is_dir():
            failure = {'schema': 1, 'status': 'FAIL', 'protocol': 'callgrind-v1', 'out': str(output),
                       'error': f'{type(error).__name__}: {error}', 'seconds': time.monotonic() - started}
            with (output / 'FAILURE.json').open('x', encoding='utf-8') as stream:
                json.dump(failure, stream, indent=2, sort_keys=True, allow_nan=False)
                stream.write('\n')
        raise


def main():
    parser = argparse.ArgumentParser(description='Capture a local callgrind packet')
    for flag in ('repo', 'spec', 'out', 'hosts', 'hosts-sha256'):
        parser.add_argument('--' + flag, required=True)
    args = parser.parse_args()
    spec_path = Path(args.spec).resolve(strict=True)
    spec_pin = counter_controller._pin(spec_path)
    spec_data = spec_path.read_bytes()
    if hashlib.sha256(spec_data).hexdigest() != spec_pin['sha256']:
        raise ValueError('measurement specification changed while reading')
    spec = json.loads(spec_data)
    if (not isinstance(spec, dict) or set(spec) not in ({'schema', 'roles', 'inputs', 'symbol_map'},
                                                        {'schema', 'roles', 'inputs', 'symbol_map', 'qualifications',
                                                         'record', 'baseline_milestone'}) or spec['schema'] != 1
            or not isinstance(spec['inputs'], str) or (spec['symbol_map'] is not None and not isinstance(spec['symbol_map'], str))):
        raise ValueError('invalid callgrind measurement request')
    if 'qualifications' in spec and spec['qualifications'] is None:
        raise ValueError('qualified request requires an explicit policy')
    host_path = Path(args.hosts).resolve(strict=True)
    host_data = host_path.read_bytes()
    if hashlib.sha256(host_data).hexdigest() != args.hosts_sha256:
        raise ValueError('host configuration differs from the xtask receipt')
    host = json.loads(host_data).get('measurement')
    result = run(args.repo, host, spec['roles'], spec['inputs'], args.out, host_config=host_path,
                 request_spec=spec_pin, symbol_map=spec['symbol_map'], qualifications=spec.get('qualifications'),
                 record=spec.get('record'), baseline_milestone=spec.get('baseline_milestone'))
    print(f"callgrind {result['status']}: {result['out']}")


if __name__ == '__main__':
    main()
