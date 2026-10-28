"""Independently reparse a complete fresh counter2-v1 packet."""
import hashlib
import json
import math
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import counter_cell
import counter_schedule
import input_closure
import perf_parse

GROUP = '{cpu_core/instructions/u,cpu_core/cpu-cycles/u}'


def declared(spec):
    required = {'seed', 'cpu', 'pins', 'binaries', 'logical_session', 'logical_inputs', 'logical_kernels',
                'env_probe', 'environment', 'path_counts'}
    if not isinstance(spec, dict) or set(spec) != required:
        raise ValueError('invalid counter packet specification')
    if (not isinstance(spec['seed'], str) or not spec['seed']
            or any(c not in '0123456789abcdef' for c in spec['seed'])):
        raise ValueError('counter packet seed must be lowercase hexadecimal')
    if type(spec['cpu']) is not int or spec['cpu'] < 0 or not isinstance(spec['pins'], dict):
        raise ValueError('invalid counter packet host binding')
    if not isinstance(spec['binaries'], dict) or set(spec['binaries']) != set(counter_schedule.SUBJECTS):
        raise ValueError('counter packet needs all six build roles')
    for value in [*spec['binaries'].values(), spec['logical_session'], spec['logical_inputs'], spec['logical_kernels'], spec['env_probe']]:
        if not isinstance(value, str) or not Path(value).is_absolute():
            raise ValueError('counter packet logical paths must be absolute')
    environment = spec['environment']
    if (not isinstance(environment, dict) or set(environment) != {'PATH', 'HOME', 'LANG'}
            or any(not isinstance(value, str) for value in environment.values())):
        raise ValueError('invalid counter packet environment')
    files = ['SPEC.json', 'MANIFEST.json', 'SCHEDULE.json', 'session/records.jsonl']
    if spec['path_counts'] is not None:
        if spec['path_counts'] != 'PATH-COUNTS.json':
            raise ValueError('invalid counter path-count input')
        files.append(spec['path_counts'])
    prefixes = [f'probe-{i}' for i in range(3)]
    for subject in counter_schedule.SUBJECTS:
        prefixes.append('env-' + subject)
        prefixes.extend('admission-' + subject + '-' + kernel for kernel, _ in counter_schedule.KERNELS)
    prefixes.extend(f'row-{i:03}' for i in range(150))
    files.extend('session/' + prefix + suffix for prefix in prefixes for suffix in ('.perf', '.stdout', '.stderr'))
    for name in files:
        input_closure._relative(name)
    return sorted(files)


def _require(condition, message):
    if not condition:
        raise ValueError(message)


def _perf_argv(path, child):
    return ['/usr/bin/perf', 'stat', '--no-scale', '-x', ';', '-o', str(path), '-e', GROUP, '--', *child]


def analyze(root):
    root = Path(root)
    spec = json.loads((root / 'SPEC.json').read_text())
    declared(spec)
    manifest = json.loads((root / 'MANIFEST.json').read_text())
    schedule_data = (root / 'SCHEDULE.json').read_bytes()
    schedule = json.loads(schedule_data)
    _require(schedule == counter_schedule.workload(int(spec['seed'], 16), manifest), 'counter schedule differs')
    records = [json.loads(line) for line in (root / 'session/records.jsonl').read_text().splitlines()]
    _require(len(records) == 191, 'counter packet record count differs')
    header, footer = records[0], records[-1]
    _require(header.get('record') == 'header' and header.get('pins') == spec['pins']
             and header.get('group') == GROUP and header.get('cpu') == spec['cpu'], 'counter header differs')
    _require(header.get('perf_policy') == {'perfconfig_absent': True, 'perf_event_paranoid': '2'},
             'counter perf policy differs')
    _require(header.get('schedule_sha256') == hashlib.sha256(schedule_data).hexdigest(), 'schedule digest differs')
    _require(tuple(header.get(k) for k in ('expected_probes', 'expected_environments', 'expected_admissions', 'expected_workloads'))
             == (3, 6, 30, 150), 'counter header counts differ')
    _require(header.get('child_timeout_s') == 120 and header.get('whole_run_bound_s') == 4800, 'counter bounds differ')
    _require(footer.get('record') == 'footer' and footer.get('complete') is True and footer.get('workloads') == 150
             and footer.get('stopped') is None and footer.get('traceback') is None, 'counter packet is incomplete')
    logical_session = Path(spec['logical_session'])
    logical_inputs = Path(spec['logical_inputs'])

    def raw(prefix):
        return tuple((root / 'session' / (prefix + suffix)).read_bytes() for suffix in ('.perf', '.stdout', '.stderr'))

    def execution(rec, prefix, argv):
        receipt = rec.get('execution')
        required = {'argv', 'cwd', 'env', 'cpu', 'status', 'seconds', 'timeout_s', 'stdout', 'stderr'}
        _require(isinstance(receipt, dict) and set(receipt) == required, 'child execution receipt missing')
        _require(receipt['argv'] == argv and receipt['cwd'] == spec['logical_kernels']
                 and receipt['env'] == spec['environment'] and receipt['cpu'] == spec['cpu']
                 and receipt['status'] == rec.get('rc')
                 and receipt['stdout'] == str(logical_session / (prefix + '.stdout'))
                 and receipt['stderr'] == str(logical_session / (prefix + '.stderr')),
                 'child execution receipt differs')
        elapsed = receipt['seconds']
        _require(type(elapsed) in (int, float) and math.isfinite(elapsed) and elapsed >= 0
                 and receipt['timeout_s'] == 120, 'invalid child duration or timeout')

    index = 1
    for i in range(3):
        rec = records[index]
        index += 1
        prefix = f'probe-{i}'
        perf, stdout, stderr = raw(prefix)
        expected_argv = _perf_argv(logical_session / (prefix + '.perf'), ['/usr/bin/env', '-u', 'PERF_BUILDID_DIR', '/bin/true'])
        _require(rec.get('record') == 'probe' and rec.get('index') == i and rec.get('argv') == expected_argv, 'probe identity differs')
        execution(rec, prefix, expected_argv)
        parsed = perf_parse.classify_probe(rec.get('rc'), stdout, stderr, perf, 'B1', counter_cell.EVENTS, counter_cell.GROUPS)
        _require(parsed == rec.get('result') and parsed['probe_ok'], 'probe evidence differs')

    cells = {}
    def subject(rec, ident, prefix, kind, key):
        _require(rec.get('record') == kind and rec.get('index') == key and rec.get('identity') == ident,
                 'counter subject identity differs')
        perf, stdout, stderr = raw(prefix)
        parsed = perf_parse.parse(perf_parse.decode(perf), 'B1', counter_cell.EVENTS, counter_cell.GROUPS)
        _require(parsed == rec.get('counters') and perf_parse.probe_ok(parsed)[0]
                 and rec.get('valid') is True and rec.get('rc') == 0 and not stderr, 'counter profile differs')
        _require(hashlib.sha256(perf).hexdigest() == rec.get('perf_sha256')
                 and hashlib.sha256(stdout).hexdigest() == rec.get('stdout_sha256'), 'counter raw digest differs')
        kernel, stream, k, role = ident['kernel'], ident['stream'], ident['k'], ident['subject']
        maps = 1 if role.endswith('-on') and kernel in ('numeric', 'while', 'calls') else 0
        checksum = manifest['cells'][kernel + '/' + stream]['expected_checksum'][str(k)]
        expected = f'CHECKSUM {checksum} invocations={k}\nMAPS anon_exec_mappings={maps} anon_exec_kib={maps * 4}\n'.encode()
        _require(stdout == expected, 'counter checksum or native admission differs')
        stream_path = logical_inputs / manifest['streams'][stream]['path']
        child = ['/usr/bin/env', '-u', 'PERF_BUILDID_DIR', spec['binaries'][role], kernel + '.rn', str(k), str(stream_path), '-']
        _require(rec.get('argv') == _perf_argv(logical_session / (prefix + '.perf'), child), 'counter argv differs')
        execution(rec, prefix, rec['argv'])
        values = {event: counter_cell.counter_assess.count(parsed['events'][event]['value']) for event in ('instructions', 'cycles')}
        if kind == 'row':
            cells.setdefault((kernel, role), []).append({'repeat': ident['repeat'], 'values': values})

    admissions = counter_schedule.admissions(manifest)
    admission_index = 0
    for name in counter_schedule.SUBJECTS:
        rec = records[index]
        index += 1
        prefix = 'env-' + name
        perf, stdout, stderr = raw(prefix)
        parsed = perf_parse.parse(perf_parse.decode(perf), 'B1', counter_cell.EVENTS, counter_cell.GROUPS)
        keys = json.loads(stdout)
        child = ['/usr/bin/env', '-u', 'PERF_BUILDID_DIR', '/usr/bin/python3', '-I', '-B', spec['env_probe']]
        _require(rec.get('record') == 'environment' and rec.get('subject') == name
                 and rec.get('argv') == _perf_argv(logical_session / (prefix + '.perf'), child), 'environment identity differs')
        execution(rec, prefix, rec['argv'])
        _require(rec.get('rc') == 0 and not stderr and perf_parse.probe_ok(parsed)[0] and parsed == rec.get('perf'),
                 'environment counter evidence differs')
        _require(keys == rec.get('keys_and_lengths') and 'PERF_BUILDID_DIR' not in keys
                 and isinstance(keys, dict) and all(isinstance(k, str) and type(v) is int and v >= 0 for k, v in keys.items()),
                 'environment contents differ')
        for kernel, _ in counter_schedule.KERNELS:
            ident = admissions[admission_index]
            admission_index += 1
            subject(records[index], ident, 'admission-' + name + '-' + kernel, 'admission', name + '/' + kernel)
            index += 1
    for i, ident in enumerate(schedule):
        subject(records[index], ident, f'row-{i:03}', 'row', i)
        index += 1
    _require(index == len(records) - 1 and len(cells) == 30, 'counter cell set is incomplete')
    path_counts = None if spec['path_counts'] is None else json.loads((root / spec['path_counts']).read_text())
    results = []
    for config in ('off', 'on'):
        for kernel, _ in counter_schedule.KERNELS:
            series = {}
            for role in ('base', 'tip', 'candidate'):
                entries = cells[(kernel, role + '-' + config)]
                _require(len(entries) == 5 and sorted(item['repeat'] for item in entries) == list(range(5)), 'counter repeats differ')
                series[role] = {event: [item['values'][event] for item in entries] for event in ('instructions', 'cycles')}
            from statistics import median
            medians = {role: {event: median(values) for event, values in events.items()} for role, events in series.items()}
            result = {'kernel': kernel, 'config': config, 'series': series, 'medians': medians,
                      'instruction_rule': counter_cell.counter_assess.assess(medians['candidate']['instructions'], medians['tip']['instructions']),
                      'cycles_report': counter_cell.counter_assess.assess_cycles(series['candidate']['cycles'], series['tip']['cycles'])}
            if path_counts is not None:
                _require(isinstance(path_counts, dict) and config in path_counts, 'representative path counts missing')
                result['representative_path_counts'] = path_counts[config]
            results.append(result)
    return {'cells': results, 'probes': 3, 'environments': 6, 'admissions': 30, 'workloads': 150}


if __name__ == '__main__':
    print(json.dumps(analyze('/inputs'), sort_keys=True, allow_nan=False))
