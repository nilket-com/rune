"""Recheck fresh local measurement evidence for a candidate source snapshot.

The command-level seal and ledger integration still require controller closure
coverage and remote-host receipts; this module exposes no record operation.
"""
import hashlib
import json
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_package
import callgrind_capture
import control_proof
import counter_controller
import counter_schedule
import input_verification
import k1_packet
import measurement_runtime
import measurement_readiness
import qualification_policy
import role_evidence
import source_export


def _pin(path):
    return role_evidence._pin(Path(path).resolve(strict=True))


def _absolute(value, field):
    if not isinstance(value, str) or not Path(value).is_absolute():
        raise ValueError(f'{field} must be an absolute path')
    return Path(value).resolve(strict=True)


def _check_input_pins(pins):
    if not isinstance(pins, dict):
        raise ValueError('capture input pins are missing')
    for path, digest in pins.items():
        if (not isinstance(path, str) or not Path(path).is_absolute() or not isinstance(digest, str)
                or not re.fullmatch('[0-9a-f]{64}', digest) or _pin(path)['path'] != path
                or _pin(path)['sha256'] != digest):
            raise ValueError('capture input pin differs')


def inspect(spec):
    seal_tool = _pin(Path(__file__).resolve())
    fields = {'schema', 'capture', 'signing_repo', 'commit', 'native_source', 'ignored_inputs'}
    if not isinstance(spec, dict) or set(spec) != fields or spec['schema'] != 1:
        raise ValueError('invalid seal evidence specification')
    repo = _absolute(spec['signing_repo'], 'signing_repo')
    source = _absolute(spec['native_source'], 'native_source')
    capture_path = _absolute(spec['capture'], 'capture')
    if capture_path.name != 'RESULT.json':
        raise ValueError('capture must reference RESULT.json')
    capture_pin = _pin(capture_path)
    raw = capture_path.read_bytes()
    if hashlib.sha256(raw).hexdigest() != capture_pin['sha256']:
        raise ValueError('capture changed while reading')
    capture = json.loads(raw)
    protocol = capture.get('protocol')
    if (capture.get('schema') != 1 or capture.get('status') != 'PASS'
            or protocol not in ('counter2-v1', 'k1-v1', 'callgrind-v1')
            or capture.get('out') != str(capture_path.parent)):
        raise ValueError('capture is not a passing fresh measurement')
    code_root = Path(__file__).resolve().parent
    if capture.get('code_root') != str(code_root):
        raise ValueError('capture was produced by a different tool checkout')
    request_pin = capture.get('request_spec')
    if not isinstance(request_pin, dict) or set(request_pin) != {'path', 'sha256', 'bytes'}:
        raise ValueError('capture has no pinned measurement request')
    if _pin(request_pin['path']) != request_pin:
        raise ValueError('measurement request changed')
    request_data = Path(request_pin['path']).read_bytes()
    if hashlib.sha256(request_data).hexdigest() != request_pin['sha256']:
        raise ValueError('measurement request changed while reading')
    request = json.loads(request_data)
    roles = request.get('roles') if isinstance(request, dict) else None
    if not isinstance(roles, dict) or not roles:
        raise ValueError('measurement request roles are missing')
    if protocol == 'counter2-v1':
        if set(request) != {'schema', 'roles', 'inputs', 'seed', 'path_counts'} or set(roles) != set(counter_schedule.SUBJECTS):
            raise ValueError('invalid counter measurement request')
    elif protocol == 'k1-v1':
        if set(request) != {'schema', 'roles', 'profile'} or request.get('profile') not in ('cgu1', 'cgu16'):
            raise ValueError('invalid K1 measurement request')
        k1_packet.subjects(roles)
    else:
        import callgrind_controller
        if (set(request) not in ({'schema', 'roles', 'inputs', 'symbol_map'},
                                 {'schema', 'roles', 'inputs', 'symbol_map', 'qualifications',
                                  'record', 'baseline_milestone'})
                or set(roles) != callgrind_controller.required_roles()):
            raise ValueError('invalid callgrind measurement request')
        if 'qualifications' in request and request['qualifications'] is None:
            raise ValueError('qualified request requires an explicit policy')
    if request.get('schema') != 1:
        raise ValueError('unsupported measurement request schema')
    for name, entry in roles.items():
        if not isinstance(entry, dict):
            raise ValueError('measurement role must be an object')
        parts = name.split('-')
        expected_role, mode = parts[0], parts[-1]
        profile = parts[1] if protocol == 'callgrind-v1' else request.get('profile') if protocol == 'k1-v1' else entry.get('profile')
        if (entry.get('name') != name or entry.get('role') != expected_role
                or mode not in ('on', 'off') or entry.get('native') is not (mode == 'on')
                or profile not in ('cgu1', 'cgu16') or entry.get('profile') != profile):
            raise ValueError('measurement role identity differs from request')
    if protocol == 'counter2-v1' and len({entry['profile'] for entry in roles.values()}) != 1:
        raise ValueError('counter builds use different profiles')
    for role in {entry['role'] for entry in roles.values()}:
        if len({entry['commit'] for entry in roles.values() if entry['role'] == role}) != 1:
            raise ValueError('measurement role builds use different commits')
    if not isinstance(spec['commit'], str) or not re.fullmatch('[0-9a-f]{40}', spec['commit']):
        raise ValueError('seal requires a full candidate commit')
    candidates = [entry for entry in roles.values() if isinstance(entry, dict) and entry.get('role') == 'candidate']
    if not candidates or any(entry.get('commit') != spec['commit'] for entry in candidates):
        raise ValueError('candidate role does not match the signed commit')
    if (source / '.git').exists():
        if spec['ignored_inputs']:
            raise ValueError('commit exports cannot declare ignored inputs')
        signed = source_export.verify_signed(repo, source, spec['commit'])
    else:
        signed = source_export.verify_signed_snapshot(repo, source, spec['commit'], spec['ignored_inputs'])
    checked = {name: role_evidence.check(repo, entry) for name, entry in roles.items()}
    if any(item['source_tree'] != signed['tree'] for item in checked.values() if item['role'] == 'candidate'):
        raise ValueError('candidate build source differs from signed snapshot')
    if checked != capture.get('roles'):
        raise ValueError('capture role evidence differs from current builds')
    host_pin = capture.get('host_config')
    if not isinstance(host_pin, dict) or set(host_pin) != {'path', 'sha256', 'contents'}:
        raise ValueError('capture host configuration is missing')
    host_file = _absolute(host_pin['path'], 'host_config')
    host_data = host_file.read_bytes()
    if (hashlib.sha256(host_data).hexdigest() != host_pin['sha256']
            or host_data.decode('utf-8') != host_pin['contents']):
        raise ValueError('capture host configuration changed')
    host = json.loads(host_data).get('measurement')
    if host != capture.get('host'):
        raise ValueError('capture host differs from configuration')
    if protocol == 'k1-v1':
        runtime_paths = [Path('/usr/bin/perf'), Path('/usr/bin/env'),
                         *(Path(roles[name]['binary']['path']) for name in k1_packet.subjects(roles))]
    elif protocol == 'counter2-v1':
        runtime_paths = [Path('/usr/bin/perf'), Path('/usr/bin/env'),
                         *(Path(entry['binary']['path']) for entry in roles.values())]
    else:
        runtime_paths = [Path(host['valgrind_path']), *(Path(entry['binary']['path']) for entry in roles.values())]
    runtime_dependencies = measurement_runtime.dependency_snapshot(runtime_paths)
    if runtime_dependencies != capture.get('runtime_dependencies'):
        raise ValueError('measurement runtime dependencies differ')
    root = capture_path.parent
    cwd = _absolute(capture.get('request_cwd'), 'request_cwd')
    def requested_path(value):
        path = Path(value)
        return (path if path.is_absolute() else cwd / path).resolve(strict=True)
    common = [host_file, Path(request_pin['path'])]
    for role in roles.values():
        common.extend((Path(role['build']['path']), Path(role['binary']['path'])))
    if protocol == 'counter2-v1':
        inputs = requested_path(request['inputs'])
        expected_paths = common + [inputs / 'MANIFEST.json', code_root / 'env_probe.py',
                                   Path('/usr/bin/perf'), Path('/usr/bin/env'), Path('/usr/bin/python3')]
        expected_paths.extend(code_root / name for name in counter_controller.IMPLEMENTATIONS)
        expected_paths.extend(inputs / item['path'] for item in capture['input_proof']['closure']['files'])
        manifest = json.loads((inputs / 'MANIFEST.json').read_bytes())
        for kernel, stream in counter_schedule.KERNELS:
            expected_paths.extend((code_root / 'kernels' / (kernel + '.rn'), inputs / manifest['streams'][stream]['path']))
        if request['path_counts'] is not None:
            expected_paths.append(requested_path(request['path_counts']))
    elif protocol == 'k1-v1':
        import k1_controller
        expected_paths = common + [code_root / 'kernels/k1.rn', code_root / 'kernels/k1.inputs.txt',
                                   Path('/usr/bin/perf'), Path('/usr/bin/env'), Path('/usr/bin/python3')]
        expected_paths.extend(code_root / name for name in k1_controller.IMPLEMENTATIONS)
    else:
        inputs = requested_path(request['inputs'])
        expected_paths = common + [inputs / 'MANIFEST.json', Path(host['valgrind_path']),
                                   Path('/usr/bin/python3'), Path('/usr/bin/nm'), Path('/usr/bin/objdump')]
        expected_paths.extend(code_root / name for name in callgrind_controller.IMPLEMENTATIONS)
        expected_paths.extend(inputs / item['path'] for item in capture['input_proof']['closure']['files'])
        manifest = json.loads((inputs / 'MANIFEST.json').read_bytes())
        for kernel in callgrind_controller.KERNELS:
            expected_paths.extend((code_root / 'kernels' / (kernel + '.rn'),
                                   inputs / manifest['streams'][callgrind_controller.STREAMS[kernel]]['path']))
        expected_paths.extend(code_root / 'kernels' / ('control_' + name + '.rn') for name in ('is', 'branch'))
        if request['symbol_map'] is not None:
            expected_paths.append(requested_path(request['symbol_map']))
    controller_inputs = {pin['path']: pin for pin in (_pin(path) for path in expected_paths)}
    expected_input_pins = set(controller_inputs)
    analyses = {}
    journals = []
    assessment = {}
    if protocol in ('counter2-v1', 'k1-v1'):
        package = json.loads((root / 'SPEC.json').read_bytes())
        if package.get('binaries') != {name: role['binary']['path'] for name, role in roles.items()}:
            raise ValueError('packet binaries differ from requested fresh builds')
        if package.get('cpu') != host.get('measurement_cpu'):
            raise ValueError('packet CPU differs from host configuration')
        environment = {'PATH': '/usr/bin:/bin', 'HOME': str(root / 'home'), 'LANG': 'C.UTF-8'}
        if package.get('environment') != environment:
            raise ValueError('packet environment differs from the measurement run')
        _check_input_pins(package.get('pins'))
        if set(package['pins']) != expected_input_pins:
            raise ValueError('measurement controller input declaration differs')
        if protocol == 'counter2-v1':
            inputs = requested_path(request['inputs'])
            if capture.get('inputs') != str(inputs) or package.get('logical_inputs') != str(inputs):
                raise ValueError('counter input path differs from request')
            if (package.get('seed') != request.get('seed') or package.get('logical_session') != str(root / 'session')
                    or package.get('logical_kernels') != str(code_root / 'kernels')
                    or package.get('env_probe') != str(code_root / 'env_probe.py')):
                raise ValueError('counter packet paths or seed differ from request')
            if input_verification.verify(inputs) != capture.get('input_proof'):
                raise ValueError('counter input regeneration differs')
            expected_counts = request.get('path_counts')
            if expected_counts is None:
                if package.get('path_counts') is not None:
                    raise ValueError('unexpected counter path-count input')
            else:
                original = requested_path(expected_counts)
                if package.get('path_counts') != 'PATH-COUNTS.json' or _pin(original)['sha256'] != _pin(root / 'PATH-COUNTS.json')['sha256']:
                    raise ValueError('counter path-count input differs')
            analyzer = 'counter2-v1'
        else:
            if package.get('profile') != request.get('profile'):
                raise ValueError('K1 profile differs from request')
            if (package.get('logical_session') != str(root) or package.get('logical_program') != str(code_root / 'kernels/k1.rn')
                    or package.get('logical_input') != str(code_root / 'kernels/k1.inputs.txt')):
                raise ValueError('K1 program or input differs from the measurement run')
            analyzer = 'k1-packet-v1'
        manifest = analysis_package.inspect(root, analyzer)
        if manifest != capture.get('analysis_manifest'):
            raise ValueError('capture analysis closure differs')
        result = analysis_package.analyze(root, manifest)
        if protocol == 'counter2-v1' and any(cell['instruction_rule']['rejected'] for cell in result['cells']):
            raise ValueError('counter instruction gate rejects')
        if protocol == 'k1-v1' and request['profile'] == 'cgu1' and any(not row['tier1_pass'] for row in result['ratios'].values()):
            raise ValueError('K1 instruction gate rejects')
        if protocol == 'counter2-v1':
            assessment = {'cells': [{'kernel': cell['kernel'], 'config': cell['config'],
                                     'instruction_rule': cell['instruction_rule'], 'cycles_report': cell['cycles_report']}
                                    for cell in result['cells']],
                          'cycle_flags': [{'kernel': cell['kernel'], 'config': cell['config']}
                                          for cell in result['cells'] if cell['cycles_report']['flagged']]}
        else:
            assessment = {'ratios': result['ratios']}
        analyses[protocol] = manifest
    else:
        inputs = requested_path(request['inputs'])
        if capture.get('inputs') != str(inputs) or input_verification.verify(inputs) != capture.get('input_proof'):
            raise ValueError('callgrind input regeneration differs')
        pins = capture.get('input_pins')
        if not isinstance(pins, dict):
            raise ValueError('callgrind input pins are missing')
        if set(pins) != expected_input_pins:
            raise ValueError('callgrind controller input declaration differs')
        for path, pin in pins.items():
            if path != pin.get('path') or _pin(path) != pin:
                raise ValueError('callgrind input changed')
        generated = capture.get('generated_pins')
        if not isinstance(generated, dict):
            raise ValueError('callgrind generated pins are missing')
        for path, pin in generated.items():
            if path != pin.get('path') or not Path(path).is_relative_to(root) or _pin(path) != pin:
                raise ValueError('callgrind generated input changed')
        symbol = request.get('symbol_map')
        if symbol is not None:
            original = requested_path(symbol)
            for profile in ('cgu1', 'cgu16'):
                if _pin(original)['sha256'] != _pin(root / profile / 'SYMBOL-NAME-MAP.json')['sha256']:
                    raise ValueError('callgrind symbol map differs from request')
        else:
            for profile in ('cgu1', 'cgu16'):
                if json.loads((root / profile / 'SPEC.json').read_bytes()).get('symbol_map') is not None:
                    raise ValueError('unexpected callgrind symbol map')
        if host.get('valgrind_identity') != capture.get('valgrind_identity'):
            raise ValueError('callgrind tool identity differs from host configuration')
        valgrind_root = _absolute(host.get('valgrind_root'), 'valgrind_root')
        if (not _absolute(host.get('valgrind_path'), 'valgrind_path').is_relative_to(valgrind_root)
                or measurement_runtime.tool_tree(valgrind_root) != capture.get('valgrind_tree')):
            raise ValueError('valgrind installation differs')
        context_pin = _pin(root / 'CAPTURE-CONTEXT.json')
        if capture.get('capture_context') != context_pin:
            raise ValueError('callgrind capture context differs from its receipt')
        context = json.loads((root / 'CAPTURE-CONTEXT.json').read_bytes())
        if (context.get('schema') != 1 or context.get('status') != 'CAPTURED'
                or context.get('protocol') != 'callgrind-v1'
                or any(context.get(key) != capture.get(key) for key in (
                    'out', 'inputs', 'code_root', 'request_cwd', 'roles', 'host', 'host_config',
                    'request_spec', 'input_proof', 'valgrind_identity', 'valgrind_tree',
                    'runtime_dependencies', 'input_pins', 'generated_pins', 'readiness',
                    'runs', 'capture_receipt', 'analysis_manifests'))):
            raise ValueError('callgrind capture context differs from the result')
        ready = context.get('readiness')
        if (not isinstance(ready, dict) or ready != measurement_readiness.require_ready(
                ready.get('observation', {}), max_load=host['measurement_max_load'],
                min_available_bytes=host['measurement_min_available_bytes'])
                or ready['observation']['cpu'] != host['measurement_cpu']
                or context.get('after_capture', {}).get('cpu') != host['measurement_cpu']):
            raise ValueError('callgrind capture readiness differs from the declared host')
        journals = [_pin(root / 'RUNS.json'), _pin(root / 'RUNS.jsonl')]
        callgrind_capture.verify(root, roles=roles, host=host, inputs=inputs, code=code_root)
        qualifications = request.get('qualifications')
        packet_spec = json.loads((root / 'cgu1/SPEC.json').read_bytes())
        if packet_spec.get('qualifications') != qualifications:
            raise ValueError('callgrind qualifications differ from the request')
        if qualifications is not None:
            qualification_policy.bind_record(qualifications, request.get('record'), request.get('baseline_milestone'),
                                             roles['candidate-cgu1-off']['commit'])
            expected_policy = json.loads((code_root / 'qualifications/s8.json').read_bytes())
            expected_policy['candidate_commit'] = roles['candidate-cgu1-off']['commit']
            if qualifications != expected_policy:
                raise ValueError('qualifications differ from the checked-in S8 policy')
        facts = control_proof.prove(repo, roles, code_root / 'kernels') if qualifications is not None else None
        qualification_policy.validate(qualifications, facts)
        if qualifications is not None:
            qualification_policy.bind_roles(facts, roles)
        for profile in ('cgu1', 'cgu16'):
            manifest = analysis_package.inspect(root / profile, 'callgrind-' + profile + '-v1')
            if manifest != capture.get('analysis_manifests', {}).get(profile):
                raise ValueError('callgrind analysis closure differs')
            fresh = analysis_package.analyze(root / profile, manifest,
                                             verified_facts=facts if profile == 'cgu1' else None)
            if profile == 'cgu1':
                assessment['cgu1'] = {mode: {kernel: {'rule': cell['rule'], 'core_change': cell['core_change'],
                                                       'whole_change': cell['whole_change']}
                                                for kernel, cell in phase['cells'].items()}
                                      for mode, phase in fresh.items()}
                if qualifications is not None:
                    assessment['qualifications'] = qualifications
            else:
                assessment['cgu16'] = [{'cell': cell['cell'], 'ratios': cell['ratios']} for cell in fresh['cells']]
            analyses[profile] = manifest
    if _pin(capture_path) != capture_pin or _pin(request_pin['path']) != request_pin:
        raise ValueError('capture or request changed during seal check')
    if _pin(Path(__file__).resolve()) != seal_tool:
        raise ValueError('seal evidence implementation changed during check')
    if _pin(host_file)['sha256'] != host_pin['sha256']:
        raise ValueError('host configuration changed during seal check')
    if measurement_runtime.dependency_snapshot(runtime_paths) != runtime_dependencies:
        raise ValueError('measurement runtime changed during seal check')
    signed_after = (source_export.verify_signed(repo, source, spec['commit']) if (source / '.git').exists()
                    else source_export.verify_signed_snapshot(repo, source, spec['commit'], spec['ignored_inputs']))
    if signed_after != signed or {name: role_evidence.check(repo, entry) for name, entry in roles.items()} != checked:
        raise ValueError('source or role evidence changed during seal check')
    if protocol in ('counter2-v1', 'k1-v1'):
        _check_input_pins(package['pins'])
        analyzer = 'counter2-v1' if protocol == 'counter2-v1' else 'k1-packet-v1'
        if analysis_package.inspect(root, analyzer) != analyses[protocol]:
            raise ValueError('analysis inputs changed during seal check')
    else:
        if measurement_runtime.tool_tree(valgrind_root) != capture['valgrind_tree']:
            raise ValueError('valgrind installation changed during seal check')
        if [_pin(root / 'RUNS.json'), _pin(root / 'RUNS.jsonl')] != journals:
            raise ValueError('callgrind capture journal changed during seal check')
        if _pin(root / 'CAPTURE-CONTEXT.json') != context_pin:
            raise ValueError('callgrind capture context changed during seal check')
        for path, pin in {**pins, **generated}.items():
            if _pin(path) != pin:
                raise ValueError('callgrind inputs changed during seal check')
        for profile in ('cgu1', 'cgu16'):
            if analysis_package.inspect(root / profile, 'callgrind-' + profile + '-v1') != analyses[profile]:
                raise ValueError('callgrind analysis inputs changed during seal check')
    return {'schema': 1, 'status': 'PASS', 'protocol': protocol, 'capture': capture_pin,
            'request': request_pin, 'candidate_commit': spec['commit'], 'native_source': str(source),
            'ignored_inputs': spec['ignored_inputs'], 'signed': signed, 'roles': checked, 'analysis_closures': analyses,
            'capture_journals': journals, 'runtime_dependencies': runtime_dependencies,
            'controller_inputs': controller_inputs, 'valgrind_tree': capture.get('valgrind_tree'), 'assessment': assessment,
            'seal_evidence_tool': seal_tool,
            'host_config': {'path': str(host_file), 'sha256': host_pin['sha256']}}
