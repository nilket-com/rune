"""Parse instruction-only callgrind profiles from the current run."""
import re
from fractions import Fraction
from pathlib import Path


def parse(path, positions='instr line'):
    text = Path(path).read_text()
    names, costs, incoming = {}, {}, {}
    current = callee = None
    call_cost = False
    if positions not in ('instr line', 'line'):
        raise ValueError('unsupported callgrind position format')
    if not re.search(r'^positions: ' + re.escape(positions) + r'$', text, re.M) or not re.search(r'^events: Ir$', text, re.M):
        raise ValueError('callgrind positions or events differ from the protocol')
    for line in text.splitlines():
        match = re.match(r'(c?fn)=\((\d+)\)(?: (.*))?$', line)
        if match:
            tag, identifier, name = match.groups()
            if name:
                names[identifier] = name
            if tag == 'fn':
                current = identifier
                costs.setdefault(identifier, 0)
                call_cost = False
            else:
                callee = identifier
            continue
        if line.startswith('calls='):
            if callee is None or current is None:
                raise ValueError('call record without caller and callee')
            count = int(line[6:].split()[0])
            incoming[callee] = incoming.get(callee, 0) + count
            call_cost = True
            continue
        fields = line.split()
        if (current is not None and len(fields) >= 2
                and re.fullmatch(r'(?:0x[0-9a-fA-F]+|[+-]?\d+|\*)', fields[0])
                and re.fullmatch(r'[+-]?\d+|\*', fields[1])):
            cost = int(fields[1]) if positions == 'line' else (int(fields[2]) if len(fields) > 2 else 0)
            if call_cost:
                call_cost = False
            else:
                costs[current] += cost
    summary = re.search(r'^summary: (\d+)$', text, re.M)
    if summary is None or sum(costs.values()) != int(summary[1]):
        raise ValueError('callgrind self costs do not match the summary')
    functions = {}
    for identifier, cost in costs.items():
        name = names.get(identifier, 'ID' + identifier)
        record = functions.setdefault(name, dict(self_Ir=0, calls=0))
        record['self_Ir'] += cost
        record['calls'] += incoming.get(identifier, 0)
    return dict(total_Ir=int(summary[1]), functions=functions, self_sum_verified=True)


def project_symbols(profile, mapping):
    """Apply a supplied symbol-name mapping without losing cost or call counts."""
    functions = {}
    for name, record in profile['functions'].items():
        projected = re.sub(r'\b[A-Za-z_][A-Za-z_0-9]*\b',
                           lambda match: mapping.get(match[0], match[0]), name)
        out = functions.setdefault(projected, dict(self_Ir=0, calls=0))
        out['self_Ir'] += record['self_Ir']
        out['calls'] += record['calls']
    if sum(row['self_Ir'] for row in functions.values()) != profile['total_Ir']:
        raise ValueError('symbol projection changed the instruction total')
    return dict(profile, functions=functions)


def core_total(profile):
    return sum(row['self_Ir'] for name, row in profile['functions'].items()
               if 'experimental_core::' in name or 'experimental_native::' in name)


def core_differences(tip, candidate):
    """Report every changed core function; missing call counts stay unknown."""
    differences = []
    left, right = tip['functions'], candidate['functions']
    for name in sorted(left.keys() | right.keys()):
        before, after = left.get(name, {}), right.get(name, {})
        delta = after.get('self_Ir', 0) - before.get('self_Ir', 0)
        if delta and ('experimental_core::' in name or 'experimental_native::' in name):
            calls = [value for value in (before.get('calls'), after.get('calls')) if value is not None]
            differences.append(dict(function=name, tip=before.get('self_Ir', 0),
                                    candidate=after.get('self_Ir', 0), delta=delta,
                                    tip_calls=before.get('calls'), candidate_calls=after.get('calls'),
                                    calls=max(calls) if calls else None))
    return differences


def compare_cost(tip, candidate):
    """Calculate exact core and whole-process changes for two current profiles."""
    result = {}
    for name, before, after in (
        ('core', core_total(tip), core_total(candidate)),
        ('whole', tip['total_Ir'], candidate['total_Ir']),
    ):
        if type(before) is not int or type(after) is not int or before <= 0 or after < 0:
            raise ValueError(f'invalid {name} instruction counts')
        change = Fraction(after - before, before)
        result[name] = {'tip': before, 'candidate': after, 'change': str(change),
                        'percent': float(100 * change)}
    return result
