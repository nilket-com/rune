"""Pure ledger transition for an entry and its optional milestone."""
from copy import deepcopy
import re

ENTRY_FIELDS = frozenset('''K1 K1_follow_up backlog binary_identity candidate cells
classification commit continuation_scope controls correctness counter2
counts_toward_milestone_cadence cumulative_CGU16_K1_vs_M4 cycle_flag_record
cycle_follow_up cycle_ruling date evidence frontier joint_review kind limitations
milestone new_cycle_soft_flags next_record notes preservation probe probe_discarded
product_code_changed product_profile_changed profile_note publication
publication_review qualifications recommendation record scope sequencing_agreement
source source_profile source_scopes subject supersedes tier1 tier1_counter2
tier1_cumulative tier1_gate tier2_rule_flags tip validation variants vs_M0'''.split())


def _ledger(value):
    if not isinstance(value, dict) or set(value) != {'ledger', 'milestone_criterion', 'milestones', 'entries'}:
        raise ValueError('invalid ledger schema')
    if not isinstance(value['ledger'], str) or not value['ledger'] or not isinstance(value['milestone_criterion'], str) or not value['milestone_criterion']:
        raise ValueError('ledger identity and criterion are required')
    if not isinstance(value['entries'], list) or not isinstance(value['milestones'], list):
        raise ValueError('ledger entries and milestones must be lists')
    if any(not isinstance(item, dict) for item in value['entries'] + value['milestones']):
        raise ValueError('ledger entries and milestones must be objects')


def _entry(value):
    if not isinstance(value, dict):
        raise ValueError('entry must be an object')
    if set(value) - ENTRY_FIELDS:
        raise ValueError('entry contains unreviewed fields')
    for field in ('record', 'subject', 'date'):
        if not isinstance(value.get(field), str) or not value[field]:
            raise ValueError(f'entry {field} is required')
    if not isinstance(value.get('commit'), str) or not re.fullmatch('[0-9a-f]{40}', value['commit']):
        raise ValueError('entry requires a full commit identity')
    for field in ('product_code_changed', 'product_profile_changed', 'counts_toward_milestone_cadence'):
        if field in value and type(value[field]) is not bool:
            raise ValueError(f'entry {field} must be boolean')
    if value.get('kind') == 'tooling' and value.get('counts_toward_milestone_cadence') is not False:
        raise ValueError('tooling entries cannot count toward milestone cadence')


def transition(before, entry, milestone=None):
    """Return a new ledger; writing and sealed-evidence checks are separate steps."""
    _ledger(before)
    _entry(entry)
    declaration = entry.get('milestone')
    if declaration is not None and (not isinstance(declaration, dict) or type(declaration.get('due')) is not bool):
        raise ValueError('milestone declaration must have a boolean due field')
    due = isinstance(declaration, dict) and declaration.get('due') is True
    if due != (milestone is not None):
        raise ValueError('a due milestone and its registry entry must be supplied together')
    if milestone is not None and not isinstance(milestone, dict):
        raise ValueError('milestone must be an object')
    matches = [item for item in before['entries']
               if item.get('record') == entry['record'] and item.get('commit') == entry['commit']]
    if matches:
        if matches != [entry]:
            raise ValueError('conflicting entry for this record and commit')
        if milestone is not None:
            found = [item for item in before['milestones'] if item.get('id') == milestone.get('id')]
            if found != [milestone]:
                raise ValueError('existing entry has no matching milestone')
        return deepcopy(before), 'already-present'
    if milestone is not None:
        if not isinstance(milestone, dict) or milestone.get('commit') != entry['commit']:
            raise ValueError('milestone and entry commits differ')
        identifier = milestone.get('id')
        if not isinstance(identifier, str) or not re.fullmatch(r'M\d+', identifier):
            raise ValueError('invalid milestone identifier')
        if declaration.get('name') != identifier:
            raise ValueError('entry and milestone identifiers differ')
        existing = before['milestones']
        if not existing or not isinstance(existing[-1], dict):
            raise ValueError('a preceding milestone is required')
        previous = existing[-1].get('id')
        if not isinstance(previous, str) or not re.fullmatch(r'M\d+', previous):
            raise ValueError('invalid previous milestone identifier')
        if declaration.get('anchor') != previous or int(identifier[1:]) != int(previous[1:]) + 1:
            raise ValueError('milestone does not continue from the current anchor')
        if any(item.get('id') == identifier for item in existing):
            raise ValueError('milestone identifier already exists')
    after = deepcopy(before)
    after['entries'].append(deepcopy(entry))
    if milestone is not None:
        after['milestones'].append(deepcopy(milestone))
    return after, 'appended'
