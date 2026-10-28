"""Analyze complete fresh CGU1 callgrind controls and five-kernel phases."""
import json
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import callgrind
import callgrind_rules
import gen_inputs
import input_closure
import qualification_policy


def declared(spec):
    if (not isinstance(spec, dict) or set(spec) not in (
            {'positions', 'symbol_map', 'controls', 'phases'},
            {'positions', 'symbol_map', 'controls', 'phases', 'qualifications'})):
        raise ValueError('invalid callgrind packet specification')
    if spec['positions'] not in ('instr line', 'line'):
        raise ValueError('invalid callgrind positions')
    if 'qualifications' in spec:
        qualification_policy.validate_policy(spec['qualifications'])
    files = {'SPEC.json'}
    if spec['symbol_map'] is not None:
        if spec['symbol_map'] != 'SYMBOL-NAME-MAP.json':
            raise ValueError('invalid symbol mapping path')
        files.add(spec['symbol_map'])
    def add_profile(row):
        if not isinstance(row, dict) or set(row) != {'profile', 'stdout'}:
            raise ValueError('profile and stdout declarations required')
        for name in (row['profile'], row['stdout']):
            if not isinstance(name, str) or name in files:
                raise ValueError('callgrind capture files must be distinct')
            files.add(name)
    def add_guard(guard):
        if guard is not None:
            if not isinstance(guard, dict) or set(guard) != {'tip_source', 'candidate_source', 'tip_binary', 'candidate_binary'}:
                raise ValueError('invalid iterator guard declaration')
            files.update(guard.values())
    controls = spec['controls']
    if not isinstance(controls, dict) or set(controls) != {'profiles', 'guard'}:
        raise ValueError('invalid callgrind controls')
    if not isinstance(controls['profiles'], dict) or set(controls['profiles']) != {'is', 'branch'}:
        raise ValueError('both callgrind controls required')
    for profiles in controls['profiles'].values():
        if not isinstance(profiles, dict) or set(profiles) != {'tip0', 'tip100', 'candidate0', 'candidate100'}:
            raise ValueError('each control requires four profiles')
        for row in profiles.values():
            add_profile(row)
    add_guard(controls['guard'])
    phases = spec['phases']
    if not isinstance(phases, dict) or set(phases) != {'off', 'on'}:
        raise ValueError('both callgrind phases required')
    for phase in phases.values():
        if not isinstance(phase, dict) or set(phase) != {'cells', 'guard'}:
            raise ValueError('invalid callgrind phase')
        if not isinstance(phase['cells'], dict) or set(phase['cells']) != callgrind_rules.KERNELS:
            raise ValueError('five callgrind kernels required')
        add_guard(phase['guard'])
        for kernel, cell in phase['cells'].items():
            if not isinstance(cell, dict) or set(cell) != {'tip', 'candidate', 'reference', 'input_file'}:
                raise ValueError('invalid callgrind cell')
            for role in ('tip', 'candidate', 'reference'):
                add_profile(cell[role])
            if not isinstance(cell['input_file'], str):
                raise ValueError('callgrind input stream is required')
            files.add(cell['input_file'])
    for path in files:
        input_closure._relative(path)
    return sorted(files)


def analyze(root, proof=None):
    root = Path(root)
    spec = json.loads((root / 'SPEC.json').read_text())
    declared(spec)
    mapping = None
    if spec['symbol_map'] is not None:
        mapping = json.loads((root / spec['symbol_map']).read_text())
        if not isinstance(mapping, dict) or any(not isinstance(k, str) or not isinstance(v, str) for k, v in mapping.items()):
            raise ValueError('symbol map must contain string pairs')
    def profile(row, baseline):
        parsed = callgrind.parse(root / row['profile'], spec['positions'])
        if baseline and mapping is not None:
            parsed = callgrind.project_symbols(parsed, mapping)
        lines = (root / row['stdout']).read_text().splitlines()
        if not lines or not re.fullmatch(r'CHECKSUM -?\d+ invocations=1000', lines[0]):
            raise ValueError('callgrind checksum output differs')
        return parsed, lines[0]
    def guard(paths):
        return None if paths is None else {name: str(root / path) for name, path in paths.items()}
    control_profiles = {}
    for name, rows in spec['controls']['profiles'].items():
        observed = {key: profile(row, key.startswith('tip')) for key, row in rows.items()}
        if any(observed['tip' + k][1] != observed['candidate' + k][1] for k in ('0', '100')):
            raise ValueError('callgrind control checksums differ')
        for suffix in ('0', '100'):
            value = 0 if suffix == '0' else (300 if name == 'is' else 150)
            expected = f'CHECKSUM {gen_inputs.checksum([value] * 1000)} invocations=1000'
            if observed['tip' + suffix][1] != expected:
                raise ValueError('callgrind control checksum differs from its model')
        control_profiles[name] = {key: value[0] for key, value in observed.items()}
    result = {}
    for mode, phase in spec['phases'].items():
        cells = {}
        for kernel, cell in phase['cells'].items():
            observed = {role: profile(cell[role], role != 'candidate') for role in ('tip', 'candidate', 'reference')}
            if len({value[1] for value in observed.values()}) != 1:
                raise ValueError('callgrind role checksums differ')
            values = (root / cell['input_file']).read_text().splitlines()
            if len(values) < 1000:
                raise ValueError('callgrind input stream is incomplete')
            xs = [int(x) for x in values[:1000]]
            expected = f'CHECKSUM {gen_inputs.checksum([gen_inputs.model(kernel, x) for x in xs])} invocations=1000'
            if observed['tip'][1] != expected:
                raise ValueError('callgrind checksum differs from its input model')
            iterations = sum(xs) if kernel in ('while', 'numeric', 'calls') else None
            cells[kernel] = {'tip': observed['tip'][0], 'candidate': observed['candidate'][0],
                             'reference': observed['reference'][0], 'iterations': iterations,
                             'guard': guard(phase['guard'])}
        result[mode] = callgrind_rules.assess_phase_with_controls(
            cells, control_profiles, guard=guard(spec['controls']['guard']),
            qualifications=spec.get('qualifications'), proof=proof, mode=mode)
    return result


if __name__ == '__main__':
    if len(sys.argv) not in (1, 2):
        raise ValueError('invalid proof arguments')
    proof = json.loads(Path(sys.argv[1]).read_text()) if len(sys.argv) == 2 else None
    print(json.dumps(analyze('/inputs', proof=proof), sort_keys=True, allow_nan=False))
