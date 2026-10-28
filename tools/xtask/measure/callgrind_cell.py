"""Analyze one pair of freshly captured callgrind profiles."""
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import callgrind
import input_closure


def declared(spec):
    if not isinstance(spec, dict) or set(spec) != {'tip', 'candidate', 'positions', 'symbol_map'}:
        raise ValueError('invalid callgrind cell specification')
    if spec['positions'] not in ('instr line', 'line'):
        raise ValueError('invalid callgrind positions')
    files = ['SPEC.json', spec['tip'], spec['candidate']]
    if spec['symbol_map'] is not None:
        if spec['symbol_map'] != 'SYMBOL-NAME-MAP.json':
            raise ValueError('invalid symbol mapping path')
        files.append(spec['symbol_map'])
    for name in files:
        input_closure._relative(name)
    if len(files) != len(set(files)):
        raise ValueError('callgrind input paths must be distinct')
    return sorted(files)


def analyze(root):
    root = Path(root)
    spec = json.loads((root / 'SPEC.json').read_text())
    declared(spec)
    tip = callgrind.parse(root / spec['tip'], spec['positions'])
    candidate = callgrind.parse(root / spec['candidate'], spec['positions'])
    if spec['symbol_map'] is not None:
        mapping = json.loads((root / spec['symbol_map']).read_text())
        if not isinstance(mapping, dict) or any(not isinstance(k, str) or not isinstance(v, str)
                                               for k, v in mapping.items()):
            raise ValueError('symbol map must contain string pairs')
        tip = callgrind.project_symbols(tip, mapping)
    return {'cost': callgrind.compare_cost(tip, candidate),
            'differences': callgrind.core_differences(tip, candidate)}


if __name__ == '__main__':
    print(json.dumps(analyze('/inputs'), sort_keys=True, allow_nan=False))
