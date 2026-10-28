"""Independently reparse a fresh interleaved K1 measurement packet."""
import json
import math
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import counter_cell
import counter_packet
import input_closure
import k1
import perf_parse


def subjects(binaries):
    required = {f'{role}-{mode}' for role in ('candidate', 'tip') for mode in ('off', 'on')}
    milestone = {f'milestone-{mode}' for mode in ('off', 'on')}
    if not isinstance(binaries, dict) or not required <= binaries.keys() or not binaries.keys() <= required | milestone:
        raise ValueError('K1 role set differs')
    if binaries.keys() & milestone and not milestone <= binaries.keys():
        raise ValueError('both milestone configurations are required')
    roles = ('candidate', 'milestone', 'tip') if milestone <= binaries.keys() else ('candidate', 'tip')
    return [f'{role}-{mode}' for role in roles for mode in ('off', 'on')]


def declared(spec):
    fields = {'profile', 'cpu', 'pins', 'binaries', 'logical_session', 'logical_program', 'logical_input', 'environment'}
    if not isinstance(spec, dict) or set(spec) != fields or spec['profile'] not in ('cgu1', 'cgu16'):
        raise ValueError('invalid K1 packet specification')
    names = subjects(spec['binaries'])
    if type(spec['cpu']) is not int or spec['cpu'] < 0:
        raise ValueError('invalid K1 CPU')
    if not isinstance(spec['pins'], dict):
        raise ValueError('invalid K1 input pins')
    for path in [*spec['binaries'].values(), spec['logical_session'], spec['logical_program'], spec['logical_input']]:
        if not isinstance(path, str) or not Path(path).is_absolute():
            raise ValueError('K1 logical paths must be absolute')
    env = spec['environment']
    if not isinstance(env, dict) or set(env) != {'PATH', 'HOME', 'LANG'} or any(not isinstance(v, str) for v in env.values()):
        raise ValueError('invalid K1 environment')
    files = ['SPEC.json', 'records.jsonl']
    for repeat in range(5):
        for name in names:
            files.extend(f'{name}-stat-{repeat}{suffix}' for suffix in ('.perf', '.stdout', '.stderr'))
    for path in files:
        input_closure._relative(path)
    return sorted(files)


def analyze(root):
    root = Path(root)
    spec = json.loads((root / 'SPEC.json').read_text())
    declared(spec)
    names = subjects(spec['binaries'])
    records = [json.loads(line) for line in (root / 'records.jsonl').read_text().splitlines()]
    if len(records) != 5 * len(names) + 2:
        raise ValueError('K1 record count differs')
    header, footer = records[0], records[-1]
    if (header.get('record') != 'header' or header.get('cpu') != spec['cpu'] or header.get('profile') != spec['profile']
            or header.get('pins') != spec['pins']
            or header.get('perf_policy') != {'perfconfig_absent': True, 'perf_event_paranoid': '2'}
            or footer.get('record') != 'footer' or footer.get('complete') is not True):
        raise ValueError('K1 header or footer differs')
    series = {name: {'instructions': [], 'cycles': []} for name in names}
    position = 1
    session = Path(spec['logical_session'])
    for repeat in range(5):
        for name in names:
            prefix = f'{name}-stat-{repeat}'
            rec = records[position]
            position += 1
            child = ['/usr/bin/env', '-u', 'PERF_BUILDID_DIR', spec['binaries'][name], spec['logical_program'],
                     '1', spec['logical_input'], '-']
            argv = counter_packet._perf_argv(session / (prefix + '.perf'), child)
            if rec.get('record') != 'sample' or rec.get('name') != name or rec.get('repeat') != repeat or rec.get('argv') != argv:
                raise ValueError('K1 sample order or argv differs')
            execution = rec.get('execution')
            fields = {'argv', 'cwd', 'env', 'cpu', 'status', 'seconds', 'timeout_s', 'stdout', 'stderr'}
            if (not isinstance(execution, dict) or set(execution) != fields or execution['argv'] != argv
                    or execution['cwd'] != str(session) or execution['env'] != spec['environment']
                    or execution['cpu'] != spec['cpu'] or execution['status'] != 0 or execution['timeout_s'] != 120
                    or execution['stdout'] != str(session / (prefix + '.stdout'))
                    or execution['stderr'] != str(session / (prefix + '.stderr'))):
                raise ValueError('K1 execution receipt differs')
            seconds = execution['seconds']
            if type(seconds) not in (int, float) or not math.isfinite(seconds) or seconds < 0:
                raise ValueError('invalid K1 child duration')
            if (root / (prefix + '.stdout')).read_bytes() != b'CHECKSUM 0 invocations=1\nMAPS anon_exec_mappings=0 anon_exec_kib=0\n':
                raise ValueError('K1 checksum or native admission differs')
            if (root / (prefix + '.stderr')).read_bytes():
                raise ValueError('K1 child stderr is nonempty')
            parsed = perf_parse.parse(perf_parse.decode((root / (prefix + '.perf')).read_bytes()),
                                      'B1', counter_cell.EVENTS, counter_cell.GROUPS)
            if not perf_parse.probe_ok(parsed)[0] or parsed != rec.get('counters'):
                raise ValueError('K1 counters differ')
            for event in ('instructions', 'cycles'):
                series[name][event].append(counter_cell.counter_assess.count(parsed['events'][event]['value']))
    return k1.summarize(series, tier1=spec['profile'] == 'cgu1')


if __name__ == '__main__':
    print(json.dumps(analyze('/inputs'), sort_keys=True, allow_nan=False))
