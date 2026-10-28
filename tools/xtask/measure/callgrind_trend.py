"""Compute CGU16 callgrind trend ratios from fresh role profiles."""
from fractions import Fraction
import json
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import callgrind
import callgrind_rules
import gen_inputs
import input_closure


def declared(spec):
    if not isinstance(spec, dict) or set(spec) != {'positions', 'symbol_map', 'phases'}:
        raise ValueError('invalid callgrind trend specification')
    if spec['positions'] not in ('instr line', 'line'):
        raise ValueError('invalid callgrind positions')
    files = {'SPEC.json'}
    if spec['symbol_map'] is not None:
        if spec['symbol_map'] != 'SYMBOL-NAME-MAP.json':
            raise ValueError('invalid symbol mapping path')
        files.add(spec['symbol_map'])
    phases = spec['phases']
    if not isinstance(phases, dict) or set(phases) != {'off', 'on'}:
        raise ValueError('both trend phases are required')
    for phase in phases.values():
        if not isinstance(phase, dict) or set(phase) != callgrind_rules.KERNELS:
            raise ValueError('five trend kernels are required')
        for cell in phase.values():
            if not isinstance(cell, dict) or set(cell) != {'tip', 'candidate', 'milestone', 'input_file'}:
                raise ValueError('trend requires all three roles')
            if not isinstance(cell['input_file'], str):
                raise ValueError('trend input stream is required')
            files.add(cell['input_file'])
            for role in ('tip', 'candidate', 'milestone'):
                profile = cell[role]
                if not isinstance(profile, dict) or set(profile) != {'profile', 'stdout'}:
                    raise ValueError('profile and stdout declarations required')
                for name in (profile['profile'], profile['stdout']):
                    if not isinstance(name, str) or name in files:
                        raise ValueError('trend capture files must be distinct')
                    files.add(name)
    for path in files:
        input_closure._relative(path)
    return sorted(files)


def analyze(root):
    root = Path(root)
    spec = json.loads((root / 'SPEC.json').read_text())
    declared(spec)
    mapping = None
    if spec['symbol_map'] is not None:
        mapping = json.loads((root / spec['symbol_map']).read_text())
        if not isinstance(mapping, dict) or any(not isinstance(k, str) or not isinstance(v, str) for k, v in mapping.items()):
            raise ValueError('symbol map must contain string pairs')
    results = []
    for mode in ('off', 'on'):
        for kernel in ('while', 'numeric', 'calls', 'fib', 'shared_rows'):
            cell = spec['phases'][mode][kernel]
            measurements, checksums = {}, []
            for role in ('tip', 'candidate', 'milestone'):
                row = cell[role]
                data = callgrind.parse(root / row['profile'], spec['positions'])
                if role != 'candidate' and mapping is not None:
                    data = callgrind.project_symbols(data, mapping)
                lines = (root / row['stdout']).read_text().splitlines()
                if not lines or not re.fullmatch(r'CHECKSUM -?\d+ invocations=1000', lines[0]):
                    raise ValueError('callgrind trend checksum output differs')
                checksums.append(lines[0])
                measurements[role] = {'core': callgrind.core_total(data), 'whole': data['total_Ir']}
            if len(set(checksums)) != 1:
                raise ValueError('callgrind trend role checksums differ')
            values = (root / cell['input_file']).read_text().splitlines()
            if len(values) < 1000:
                raise ValueError('callgrind trend input stream is incomplete')
            xs = [int(value) for value in values[:1000]]
            expected = f'CHECKSUM {gen_inputs.checksum([gen_inputs.model(kernel, x) for x in xs])} invocations=1000'
            if checksums[0] != expected:
                raise ValueError('callgrind trend checksum differs from its input model')
            ratios = {}
            for role in ('tip', 'milestone'):
                ratios[role] = {}
                for field in ('core', 'whole'):
                    base = measurements[role][field]
                    if base <= 0:
                        raise ValueError('positive trend baseline required')
                    ratios[role][field] = float(100 * (Fraction(measurements['candidate'][field], base) - 1))
            results.append({'cell': mode + '/' + kernel, 'measurements': measurements, 'ratios': ratios})
    return {'cells': results, 'status': 'reported'}


if __name__ == '__main__':
    print(json.dumps(analyze('/inputs'), sort_keys=True, allow_nan=False))
