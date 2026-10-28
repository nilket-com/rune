"""Analyze fresh K1 B1 profiles for each declared role and configuration."""
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import counter_assess
import counter_cell
import input_closure
import k1
import perf_parse


def declared(spec):
    if not isinstance(spec, dict) or set(spec) != {'samples', 'tier1'} or type(spec['tier1']) is not bool:
        raise ValueError('invalid K1 specification')
    samples = spec['samples']
    if not isinstance(samples, dict):
        raise ValueError('K1 samples must be an object')
    required = {f'{role}-{config}' for role in ('candidate', 'tip') for config in ('on', 'off')}
    milestone = {f'milestone-{config}' for config in ('on', 'off')}
    if not required <= samples.keys() or not samples.keys() <= required | milestone:
        raise ValueError('invalid K1 role set')
    if samples.keys() & milestone and not milestone <= samples.keys():
        raise ValueError('both milestone configurations are required')
    files = ['SPEC.json']
    for paths in samples.values():
        if not isinstance(paths, list) or len(paths) != 5:
            raise ValueError('K1 requires five profiles per role')
        files.extend(paths)
    for name in files:
        input_closure._relative(name)
    if len(files) != len(set(files)):
        raise ValueError('K1 input paths must be distinct')
    return sorted(files)


def analyze(root):
    root = Path(root)
    spec = json.loads((root / 'SPEC.json').read_text())
    declared(spec)
    series = {}
    for role, paths in spec['samples'].items():
        series[role] = {'instructions': [], 'cycles': []}
        for name in paths:
            parsed = perf_parse.parse(perf_parse.decode((root / name).read_bytes()), 'B1',
                                      counter_cell.EVENTS, counter_cell.GROUPS)
            ok, why = perf_parse.probe_ok(parsed)
            if not ok:
                raise ValueError(f'K1 profile unavailable: {why}')
            for event in series[role]:
                series[role][event].append(counter_assess.count(parsed['events'][event]['value']))
    return k1.summarize(series, tier1=spec['tier1'])


if __name__ == '__main__':
    print(json.dumps(analyze('/inputs'), sort_keys=True, allow_nan=False))
