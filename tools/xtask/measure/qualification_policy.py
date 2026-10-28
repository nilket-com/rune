"""Narrow, fail-closed per-record qualification of S8 callgrind findings."""
import re

CONTROL = '<rune::experimental_core::execute::Executor>::invoke_args::{closure#0}'
NATIVE_CONTROL = '<rune::experimental_core::execute::Executor>::invoke_args::<false>::{closure#0}'
INDEX = '<rune::experimental_core::heap::Store>::dynamic_get'
CELLS = [f'{mode}/{kernel}' for mode in ('off', 'on')
         for kernel in ('while', 'numeric', 'calls', 'fib', 'shared_rows')]
EXPECTED = [
    {'rule': 'invoke-entry-zero-controls', 'symbols': [CONTROL, NATIVE_CONTROL],
     'control_symbol': CONTROL, 'cells': CELLS, 'control_bound': 'nonpositive',
     'proof': 'live-control-bytecode-equal', 'entry_proof': 'live-invocation-source-equal'},
    {'rule': 'per-iteration-growth', 'symbols': [INDEX],
     'cells': ['off/shared_rows', 'on/shared_rows'], 'max_ir_per_call': 1,
     'proof': 'live-indexing-source-equal'},
]


def validate_policy(qualifications):
    if (not isinstance(qualifications, dict) or set(qualifications) != {'schema', 'record', 'baseline_milestone', 'candidate_commit', 'entries'}
            or qualifications['schema'] != 1 or qualifications['record'] != 'S8'
            or qualifications['baseline_milestone'] != 'M7'
            or not isinstance(qualifications['candidate_commit'], str)
            or not re.fullmatch('[0-9a-f]{40}', qualifications['candidate_commit'])
            or qualifications['entries'] != EXPECTED):
        raise ValueError('unreviewed qualification policy')
    return qualifications


def validate(qualifications, proof):
    if qualifications is None:
        if proof is not None:
            raise ValueError('unexpected qualification proof')
        return None
    validate_policy(qualifications)
    fields = {'schema', 'tip_commit', 'candidate_commit', 'control_bytecode_equal',
              'indexing_source_equal', 'entry_source_equal', 'control_bytecode_sha256'}
    if (not isinstance(proof, dict) or set(proof) != fields or proof['schema'] != 1
            or proof['control_bytecode_equal'] is not True or proof['indexing_source_equal'] is not True
            or proof['entry_source_equal'] is not True
            or any(not isinstance(proof[k], str) or not re.fullmatch('[0-9a-f]{40}', proof[k])
                   for k in ('tip_commit', 'candidate_commit'))
            or not isinstance(proof['control_bytecode_sha256'], str)
            or not re.fullmatch('[0-9a-f]{64}', proof['control_bytecode_sha256'])):
        raise ValueError('live qualification proof is missing or invalid')
    if proof['candidate_commit'] != qualifications['candidate_commit']:
        raise ValueError('live qualification proof differs from the record candidate')
    return qualifications


def controls_qualified(profiles, controls):
    """The reviewed finding permits only nonpositive control deltas in one symbol."""
    for name, samples in profiles.items():
        if controls[name]['core'] > 0 or controls[name]['whole'] > 0:
            return False
        names = set().union(*(sample['functions'] for sample in samples.values()))
        for symbol in names:
            if not ('experimental_core::' in symbol or 'experimental_native::' in symbol):
                continue
            def cost(key):
                return samples[key]['functions'].get(symbol, {}).get('self_Ir', 0)
            delta = cost('candidate100') - cost('candidate0') - cost('tip100') + cost('tip0')
            if delta and (symbol != CONTROL or delta > 0):
                return False
    return True


def bind_roles(proof, roles):
    if (not isinstance(proof, dict) or proof.get('tip_commit') != roles['tip-cgu1-off']['commit']
            or proof.get('candidate_commit') != roles['candidate-cgu1-off']['commit']):
        raise ValueError('live qualification proof differs from the measured roles')


def bind_record(qualifications, record, baseline_milestone, candidate_commit):
    validate_policy(qualifications)
    if (qualifications['record'] != record or qualifications['baseline_milestone'] != baseline_milestone
            or qualifications['candidate_commit'] != candidate_commit):
        raise ValueError('qualification differs from the measurement record, milestone or candidate')


def iteration_qualified(row, cell):
    return (cell in ('off/shared_rows', 'on/shared_rows') and row['function'] == INDEX
            and type(row['tip_calls']) is int and row['tip_calls'] > 0
            and row['candidate_calls'] == row['tip_calls']
            and 0 < row['delta'] <= row['tip_calls'])
