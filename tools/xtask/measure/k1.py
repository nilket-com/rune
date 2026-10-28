"""K1 median and exact-ratio calculations for fresh counter samples."""
from fractions import Fraction
from statistics import median


def summarize(series, *, tier1=False):
    """Summarize five unscaled instruction and cycle samples per subject.

    Required subject names are candidate-on/off and tip-on/off. A milestone
    pair is optional for trend reporting. The tier-1 instruction limit is the
    exact ratio 201/200; cycle ratios are reported without a gate here.
    """
    required = {f'{role}-{config}' for role in ('candidate', 'tip') for config in ('on', 'off')}
    allowed = required | {f'milestone-{config}' for config in ('on', 'off')}
    if not required <= series.keys() or not series.keys() <= allowed:
        raise ValueError('K1 subject set differs from the declared roles')
    milestones = {f'milestone-{config}' for config in ('on', 'off')}
    if series.keys() & milestones and not milestones <= series.keys():
        raise ValueError('both milestone configurations are required')
    medians = {}
    for subject, events in series.items():
        if set(events) != {'instructions', 'cycles'}:
            raise ValueError(f'K1 event set differs for {subject}')
        medians[subject] = {}
        for event, values in events.items():
            if len(values) != 5 or any(type(v) is not int or v <= 0 for v in values):
                raise ValueError(f'K1 needs five positive integer samples for {subject}/{event}')
            medians[subject][event] = median(values)
    ratios = {}
    for config in ('off', 'on'):
        candidate = medians[f'candidate-{config}']
        comparisons = {}
        for role in ('tip', 'milestone'):
            subject = f'{role}-{config}'
            if subject in medians:
                comparisons[role] = {event: str(Fraction(candidate[event], medians[subject][event]))
                                     for event in ('instructions', 'cycles')}
        instruction_ratio = Fraction(comparisons['tip']['instructions'])
        ratios[config] = {'comparisons': comparisons,
                          'tier1_pass': instruction_ratio <= Fraction(201, 200) if tier1 else None}
    return {'medians': medians, 'ratios': ratios}
