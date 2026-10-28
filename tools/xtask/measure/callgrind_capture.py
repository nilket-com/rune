"""Recheck callgrind execution receipts against declared roles and raw files."""
import json
import math
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import callgrind
import callgrind_packet
import callgrind_trend
import input_closure

KERNELS = ('while', 'numeric', 'calls', 'fib', 'shared_rows')
STREAMS = {'while': 'loop_main', 'numeric': 'loop_main', 'calls': 'calls_main',
           'fib': 'fib_main', 'shared_rows': 'shared_rows_main'}


def _pin(path):
    path = Path(path).resolve(strict=True)
    record = input_closure._pin(path.parent, path.name)
    return {'path': str(path), 'sha256': record['sha256'], 'bytes': record['bytes']}


def verify(out, *, roles, host, inputs, code):
    """Validate every capture against its role, command, output and profile."""
    out, inputs, code = (Path(path).resolve(strict=True) for path in (out, inputs, code))
    specs = {profile: json.loads((out / profile / 'SPEC.json').read_text()) for profile in ('cgu1', 'cgu16')}
    callgrind_packet.declared(specs['cgu1'])
    callgrind_trend.declared(specs['cgu16'])
    manifest = json.loads((inputs / 'MANIFEST.json').read_text())
    for value in (0, 100):
        if (out / 'cgu1' / 'data' / f'control-{value}.txt').read_bytes() != (str(value) + '\n').encode() * 1000:
            raise ValueError('callgrind control input differs')
    for mode in ('off', 'on'):
        guard = specs['cgu1']['phases'][mode]['guard']
        if not isinstance(guard, dict):
            raise ValueError('callgrind guard inputs are missing')
        for role in ('tip', 'candidate'):
            entry = roles[f'{role}-cgu1-{mode}']
            build = json.loads(Path(entry['build']['path']).read_bytes())
            source = Path(build['source']['source']) / 'crates/rune/src/experimental_core/heap/hot.rs'
            for kind, original in (('source', source), ('binary', Path(entry['binary']['path']))):
                if _pin(out / 'cgu1' / guard[role + '_' + kind])['sha256'] != _pin(original)['sha256']:
                    raise ValueError('callgrind guard input differs from its build')
    if specs['cgu1']['controls']['guard'] != specs['cgu1']['phases']['off']['guard']:
        raise ValueError('control guard differs from the off build')
    for profile in ('cgu1', 'cgu16'):
        for mode in ('off', 'on'):
            for kernel in KERNELS:
                phase = specs[profile]['phases'][mode]
                cell = phase['cells'][kernel] if profile == 'cgu1' else phase[kernel]
                source = inputs / manifest['streams'][STREAMS[kernel]]['path']
                if _pin(out / profile / cell['input_file'])['sha256'] != _pin(source)['sha256']:
                    raise ValueError('callgrind analysis input differs from capture input')
    records = json.loads((out / 'RUNS.json').read_text())
    journal = [json.loads(line) for line in (out / 'RUNS.jsonl').read_text().splitlines()]
    if len(records) != 68 or len(journal) != 68:
        raise ValueError('callgrind capture inventory differs')
    expected = []
    for control in ('is', 'branch'):
        rows = specs['cgu1']['controls']['profiles'][control]
        for role in ('tip', 'candidate'):
            for n in (0, 100):
                expected.append(('cgu1', role + '-cgu1-off', rows[f'{role}{n}'],
                                 code / 'kernels' / ('control_' + control + '.rn'),
                                 out / 'cgu1' / 'data' / f'control-{n}.txt'))
    for mode in ('off', 'on'):
        for kernel in KERNELS:
            cell = specs['cgu1']['phases'][mode]['cells'][kernel]
            stream = inputs / manifest['streams'][STREAMS[kernel]]['path']
            for role, row in (('tip', 'tip'), ('candidate', 'candidate'), ('base', 'reference')):
                expected.append(('cgu1', f'{role}-cgu1-{mode}', cell[row], code / 'kernels' / (kernel + '.rn'), stream))
    for mode in ('off', 'on'):
        for kernel in KERNELS:
            cell = specs['cgu16']['phases'][mode][kernel]
            stream = inputs / manifest['streams'][STREAMS[kernel]]['path']
            for role in ('tip', 'candidate', 'milestone'):
                expected.append(('cgu16', f'{role}-cgu16-{mode}', cell[role], code / 'kernels' / (kernel + '.rn'), stream))
    env = {'PATH': '/usr/bin:/bin', 'HOME': str(out / 'home'), 'LANG': 'C.UTF-8'}
    for row, logged, (profile, role, files, program, stream) in zip(records, journal, expected):
        if not isinstance(row, dict) or set(row) != {'label', 'role', 'execution', 'profile', 'stdout', 'stderr', 'total_Ir'}:
            raise ValueError('invalid callgrind execution record')
        if {key: value for key, value in row.items() if key != 'total_Ir'} != logged or row['role'] != role:
            raise ValueError('callgrind journal or role differs')
        label = Path(files['profile']).stem
        package = out / profile
        if row['label'] != label or files['stdout'] != label + '.stdout':
            raise ValueError('callgrind profile label differs')
        for kind, path in (('profile', package / files['profile']), ('stdout', package / files['stdout']),
                           ('stderr', package / (label + '.stderr'))):
            if _pin(path) != row[kind]:
                raise ValueError('callgrind raw capture changed')
        argv = [host['valgrind_path'], '--tool=callgrind', '--dump-instr=yes', '--collect-jumps=yes',
                '--callgrind-out-file=' + str(package / files['profile']), roles[role]['binary']['path'],
                str(program), '1000', str(stream), '-']
        execution = row['execution']
        fields = {'argv', 'cwd', 'env', 'cpu', 'status', 'seconds', 'timeout_s', 'stdout', 'stderr'}
        if (not isinstance(execution, dict) or set(execution) != fields or execution['argv'] != argv
                or execution['cwd'] != str(code / 'kernels') or execution['env'] != env
                or execution['cpu'] != host['measurement_cpu'] or execution['status'] != 0 or execution['timeout_s'] != 180
                or execution['stdout'] != str(package / files['stdout']) or execution['stderr'] != str(package / (label + '.stderr'))):
            raise ValueError('callgrind child receipt differs')
        seconds = execution['seconds']
        if type(seconds) not in (int, float) or not math.isfinite(seconds) or seconds < 0:
            raise ValueError('invalid callgrind child duration')
        if callgrind.parse(package / files['profile'])['total_Ir'] != row['total_Ir']:
            raise ValueError('callgrind total differs from raw profile')
    return {'status': 'PASS', 'captures': len(records)}
