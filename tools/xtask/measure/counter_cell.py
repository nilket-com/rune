"""Analyze one candidate-versus-tip cell of fresh B1 counter profiles."""
import json
from pathlib import Path
from statistics import median
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import counter_assess
import input_closure
import perf_parse

EVENTS = {
    'instructions': ('cpu_core/instructions/u', ('cpu_core/instructions/u', 'instructions', 'instructions:u')),
    'cycles': ('cpu_core/cpu-cycles/u', ('cpu_core/cpu-cycles/u', 'cpu-cycles', 'cpu-cycles:u')),
}
GROUPS = {'B1': ('instructions', 'cycles')}


def declared(spec):
    if not isinstance(spec, dict) or set(spec) != {'tip', 'candidate', 'config', 'path_counts'}:
        raise ValueError('invalid counter cell specification')
    if spec['config'] not in ('on', 'off'):
        raise ValueError('invalid counter configuration')
    files = ['SPEC.json']
    for role in ('tip', 'candidate'):
        if not isinstance(spec[role], list) or len(spec[role]) != 5:
            raise ValueError('counter cells require five profiles per role')
        files.extend(spec[role])
    if spec['path_counts'] is not None:
        if spec['path_counts'] != 'PATH-COUNTS.json':
            raise ValueError('invalid path-count input')
        files.append(spec['path_counts'])
    for name in files:
        input_closure._relative(name)
    if len(files) != len(set(files)):
        raise ValueError('counter input paths must be distinct')
    return sorted(files)


def analyze(root):
    root = Path(root)
    spec = json.loads((root / 'SPEC.json').read_text())
    declared(spec)
    values = {}
    for role in ('tip', 'candidate'):
        values[role] = {'instructions': [], 'cycles': []}
        for name in spec[role]:
            parsed = perf_parse.parse(perf_parse.decode((root / name).read_bytes()), 'B1', EVENTS, GROUPS)
            ok, why = perf_parse.probe_ok(parsed)
            if not ok:
                raise ValueError(f'counter profile unavailable: {why}')
            for event in values[role]:
                values[role][event].append(counter_assess.count(parsed['events'][event]['value']))
    result = {'instruction_rule': counter_assess.assess(median(values['candidate']['instructions']),
                                                         median(values['tip']['instructions'])),
              'cycles': counter_assess.assess_cycles(values['candidate']['cycles'], values['tip']['cycles'])}
    if spec['path_counts'] is not None:
        paths = json.loads((root / spec['path_counts']).read_text())
        if not isinstance(paths, dict) or spec['config'] not in paths:
            raise ValueError('missing representative path counts')
        result['representative_path_counts'] = paths[spec['config']]
    return result


if __name__ == '__main__':
    print(json.dumps(analyze('/inputs'), sort_keys=True, allow_nan=False))
