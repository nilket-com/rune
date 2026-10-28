"""Fail-closed structural rules for fresh callgrind profile pairs.

Iterator growth requires a fresh source and binary guard proof. Control growth
also requires its four-profile proof. Residue requires all five phase cells.
"""
from fractions import Fraction
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import callgrind
import qualification_policy

ITER = '<rune::experimental_core::heap::Store>::iterator_next'
SWAP = frozenset({'<rune::experimental_core::heap::Store>::range_iterator_for_native',
                  '<rune::experimental_core::heap::Store>::commit_range_next_for_native',
                  'rune::experimental_core::region::commit::range_iterator'})
KERNELS = frozenset({'while', 'numeric', 'calls', 'fib', 'shared_rows'})
INSTALL_RESIDUE = ('<rune::compile::names::Names>::insert', 'rune::compile::names::Node',
                   '<rune::compile::context::Context>::install', '<rune::function::methods::State>::',
                   'rune::module::module::Name',
                   '<rune_alloc::hashbrown::map::HashMap<rune_core::hash::Hash, (), rune::hash::HashBuildHasher>>::try_insert',
                   '<rune_alloc::hashbrown::set::HashSet<rune_core::hash::Hash>>::try_insert',
                   '<rune_alloc::hashbrown::map::HashMap<rune_core::hash::Hash, ()>>::try_insert',
                   '<rune_alloc::btree::map::entry::VacantEntry<rune_core::hash::Hash, rune_alloc::btree::set_val::SetValZST>>::',
                   'rune_alloc::btree::map::infallible_cmp::<usize>',
                   '<rune::compile::meta::AssociatedKind as rune_alloc::clone::TryClone>::try_clone')
INSTALL_CALLS = ('<rune::compile::names::Names>::insert::<&rune_core::item::item_buf::ItemBuf>',
                 '<rune::compile::context::Context>::install_associated',
                 '<rune::compile::context::Context>::install_meta',
                 '<rune::compile::context::Context>::install::<rune::module::module::Module>')


def assess(tip, candidate, reference, *, invocations, iterations, controls, guard=None):
    return _assess(tip, candidate, reference, invocations=invocations, iterations=iterations,
                   controls=controls, guard=guard, allow_residue=False, allow_control_growth=False)


def _assess(tip, candidate, reference, *, invocations, iterations, controls, guard, allow_residue, allow_control_growth,
            qualification=None, control_qualified=False, cell=None):
    if type(invocations) is not int or invocations <= 0:
        raise ValueError('positive invocation count required')
    if iterations is not None and (type(iterations) is not int or iterations < 0):
        raise ValueError('iterations must be a nonnegative integer or absent')
    if not isinstance(controls, dict) or set(controls) != {'is', 'branch'}:
        raise ValueError('both control deltas are required')
    for item in controls.values():
        if not isinstance(item, dict) or set(item) != {'core', 'whole'} or any(type(v) is not int for v in item.values()):
            raise ValueError('invalid control deltas')
        if item['core'] > 0 and not allow_control_growth:
            raise ValueError('control growth requires an unported guard proof')
    tip_core, candidate_core, reference_core = (callgrind.core_total(x) for x in (tip, candidate, reference))
    if tip_core <= 0 or tip['total_Ir'] <= 0 or reference_core <= 0:
        raise ValueError('positive tip and reference costs are required')
    diff = callgrind.core_differences(tip, candidate)
    swap = [row for row in diff if row['function'] in SWAP]
    if sum(row['delta'] for row in swap) > 0:
        raise ValueError('range swap group grows')
    for row in swap:
        calls = row['calls']
        if calls is None or calls % invocations != 0 or calls > 2 * invocations:
            raise ValueError('range swap call count differs')
        if iterations is not None and calls * 100 >= iterations:
            raise ValueError('range swap scales with iterations')
    if any(row['delta'] > 0 and row['function'] not in SWAP
           and 'range' in row['function'].lower()
           and ('iterator' in row['function'].lower() or 'range_next' in row['function'].lower()) for row in diff):
        raise ValueError('hot range cost grows')
    other = [row for row in diff if row['function'] != ITER]
    for row in other:
        if (row['function'] not in SWAP and row['delta'] >= invocations
                and (row['calls'] is None or row['calls'] > invocations + 2)):
            if not (qualification is not None and qualification_policy.iteration_qualified(row, cell)):
                raise ValueError('per-iteration cost grows')
            if Fraction(candidate_core - tip_core, tip_core) > Fraction(1, 1000):
                raise ValueError('qualified per-iteration cell exceeds its core threshold')
    guard_result = None
    if any(row['function'] == ITER and row['delta'] > 0 for row in diff):
        if not isinstance(guard, dict) or set(guard) != {'tip_source', 'candidate_source', 'tip_binary', 'candidate_binary'}:
            raise ValueError('iterator growth requires a fresh guard proof')
        import callgrind_guard
        guard_result = callgrind_guard.prove(guard['tip_source'], guard['candidate_source'],
                                             guard['tip_binary'], guard['candidate_binary'], candidate)
    group = [row for row in other if row['calls'] is not None and row['calls'] <= invocations + 2]
    net = sum(row['delta'] for row in group)
    invoke_rows = [row for row in group if 'invoke_args' in row['function'] and row['delta']]
    if invoke_rows:
        if any(controls[name]['core'] != 0 for name in ('is', 'branch')):
            allowed = (qualification is not None and control_qualified and cell in qualification_policy.CELLS
                       and all(row['delta'] < 0 and row['function'] in (qualification_policy.CONTROL, qualification_policy.NATIVE_CONTROL)
                               for row in invoke_rows))
            if not allowed:
                raise ValueError('invoke argument change requires zero controls')
    bound = max(Fraction(tip_core * 1001, 1000), Fraction(reference_core))
    if net > 0:
        if net > 72 * invocations or candidate_core > bound:
            raise ValueError('positive per-invocation group exceeds its bound')
        rule = "(v')"
    else:
        if Fraction(candidate_core - tip_core, tip_core) > Fraction(1, 1000):
            raise ValueError('core growth exceeds its bound')
        rule = '(iv)' if any(row['delta'] >= invocations for row in other) else 'none'
    whole = Fraction(candidate['total_Ir'] - tip['total_Ir'], tip['total_Ir'])
    if whole > Fraction(1, 1000) and not allow_residue:
        raise ValueError('whole-process growth requires an unported residue proof')
    return {'rule': rule, 'net_Ir': net, 'core_bound': str(bound), 'guard': guard_result,
            'core_change': str(Fraction(candidate_core - tip_core, tip_core)),
            'whole_change': str(whole), 'differences': diff}


def assess_phase(cells, *, controls, invocations=1000):
    """Assess all five kernels and prove residue jointly when any cell needs it."""
    return _assess_phase(cells, controls=controls, invocations=invocations, allow_control_growth=False)


def _assess_phase(cells, *, controls, invocations, allow_control_growth,
                  qualification=None, control_qualified=False, mode=None):
    if not isinstance(cells, dict) or set(cells) != KERNELS:
        raise ValueError('a phase must contain exactly the five kernels')
    required = {'tip', 'candidate', 'reference', 'iterations'}
    if any(not isinstance(cell, dict) or not required <= set(cell) or set(cell) - (required | {'guard'})
           for cell in cells.values()):
        raise ValueError('invalid callgrind phase cell')
    needs_residue = any(Fraction(c['candidate']['total_Ir'] - c['tip']['total_Ir'], c['tip']['total_Ir'])
                        > Fraction(1, 1000) for c in cells.values())
    if needs_residue:
        deltas = []
        for cell in cells.values():
            tip, candidate = cell['tip'], cell['candidate']
            before, after = tip['functions'], candidate['functions']
            core = callgrind.core_total(tip)
            if core <= 0 or Fraction(callgrind.core_total(candidate) - core, core) > Fraction(1, 10000):
                raise ValueError('residue core growth exceeds its bound')
            deltas.append(candidate['total_Ir'] - tip['total_Ir'])
            other = []
            for name in before.keys() | after.keys():
                if 'experimental_core::' in name or 'experimental_native::' in name:
                    continue
                left, right = before.get(name, {}), after.get(name, {})
                delta = right.get('self_Ir', 0) - left.get('self_Ir', 0)
                if delta and not any(part in name for part in INSTALL_RESIDUE):
                    if abs((right.get('calls') or 0) - (left.get('calls') or 0)) >= invocations:
                        raise ValueError('non-install residue scales with invocations')
                    other.append(delta)
            if sum(other) > 2000:
                raise ValueError('non-install residue exceeds its bound')
            for name in INSTALL_CALLS:
                if before.get(name, {}).get('calls') != after.get(name, {}).get('calls'):
                    raise ValueError('installation call counts differ')
        if max(deltas) - min(deltas) > 2000 or max(deltas) > 500000:
            raise ValueError('whole-process residue differs across the phase')
    results = {kernel: _assess(cell['tip'], cell['candidate'], cell['reference'],
                               invocations=invocations, iterations=cell['iterations'], controls=controls,
                               guard=cell.get('guard'),
                               allow_residue=needs_residue, allow_control_growth=allow_control_growth,
                               qualification=qualification, control_qualified=control_qualified,
                               cell=f'{mode}/{kernel}' if mode is not None else None)
               for kernel, cell in cells.items()}
    return {'cells': results, 'residue_proved': needs_residue}


def assess_controls(profiles, guard=None):
    """Compute four-profile double differences and prove any positive control."""
    if not isinstance(profiles, dict) or set(profiles) != {'is', 'branch'}:
        raise ValueError('both four-profile controls are required')
    names = ('tip0', 'tip100', 'candidate0', 'candidate100')
    result, proofs = {}, {}
    for control, data in profiles.items():
        if not isinstance(data, dict) or set(data) != set(names):
            raise ValueError('control needs four profiles')
        delta = lambda fn: fn(data['candidate100']) - fn(data['candidate0']) - fn(data['tip100']) + fn(data['tip0'])
        result[control] = {'core': delta(callgrind.core_total), 'whole': delta(lambda x: x['total_Ir'])}
        if result[control]['core'] > 0:
            if not isinstance(guard, dict) or set(guard) != {'tip_source', 'candidate_source', 'tip_binary', 'candidate_binary'}:
                raise ValueError('control growth requires a fresh guard proof')
            import callgrind_guard
            proofs[control] = callgrind_guard.prove(guard['tip_source'], guard['candidate_source'],
                                                    guard['tip_binary'], guard['candidate_binary'], data['candidate100'])
            functions = set().union(*(profile['functions'].keys() for profile in data.values()))
            other = {}
            for name in functions:
                if name == ITER or not ('experimental_core::' in name or 'experimental_native::' in name):
                    continue
                difference = delta(lambda x: x['functions'].get(name, {}).get('self_Ir', 0))
                if difference:
                    other[name] = difference
            if (control == 'is' and other) or (control == 'branch' and any(x > 0 for x in other.values())):
                raise ValueError('guard control has unpermitted other core changes')
    return result, proofs


def assess_phase_with_controls(cells, control_profiles, *, guard=None, invocations=1000,
                               qualifications=None, proof=None, mode=None):
    """Recompute controls, prove their exceptions, then assess the phase."""
    controls, proofs = assess_controls(control_profiles, guard=guard)
    qualification = qualification_policy.validate(qualifications, proof)
    if qualification is not None and mode not in ('off', 'on'):
        raise ValueError('qualified phase must name its mode')
    control_qualified = qualification is not None and qualification_policy.controls_qualified(control_profiles, controls)
    if qualification is not None and not control_qualified:
        raise ValueError('S8 control qualification is outside its reviewed bounds')
    result = _assess_phase(cells, controls=controls, invocations=invocations, allow_control_growth=True,
                           qualification=qualification, control_qualified=control_qualified, mode=mode)
    result = dict(result, controls=controls, control_guard_proofs=proofs)
    if qualification is not None:
        result['qualifications'] = qualification['entries']
        result['qualification_proof'] = proof
    return result
