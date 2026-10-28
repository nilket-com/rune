"""Require a coherent set of fresh sealed measurements for a ledger append."""


def validate(proofs, commit):
    if not isinstance(proofs, list) or len(proofs) != 5:
        raise ValueError('record requires five fresh protocol seals')
    coverage, tips, milestones, flags = set(), set(), set(), []
    for proof in proofs:
        if (not isinstance(proof, dict) or proof.get('status') != 'PASS'
                or proof.get('candidate_commit') != commit or not isinstance(proof.get('roles'), dict)):
            raise ValueError('seal does not bind the record candidate')
        protocol = proof.get('protocol')
        roles = list(proof['roles'].values())
        if not roles or any(not isinstance(role, dict) for role in roles):
            raise ValueError('seal role evidence is missing')
        for role in roles:
            if role.get('role') == 'candidate' and role.get('commit') != commit:
                raise ValueError('candidate role differs from record commit')
            if role.get('role') == 'tip':
                tips.add(role.get('commit'))
            if role.get('role') == 'milestone':
                milestones.add(role.get('commit'))
        if protocol == 'callgrind-v1':
            expected = {(role, profile, native) for profile, names in
                        (('cgu1', ('base', 'tip', 'candidate')), ('cgu16', ('milestone', 'tip', 'candidate')))
                        for role in names for native in (False, True)}
            key = (protocol, 'both')
        elif protocol in ('counter2-v1', 'k1-v1'):
            profiles = {role.get('profile') for role in roles}
            if len(profiles) != 1 or not profiles <= {'cgu1', 'cgu16'}:
                raise ValueError('seal profile is ambiguous')
            profile = next(iter(profiles))
            names = ('base', 'tip', 'candidate') if protocol == 'counter2-v1' else ('tip', 'candidate')
            if protocol == 'k1-v1' and profile == 'cgu16':
                names = (*names, 'milestone')
            elif protocol == 'k1-v1' and any(role.get('role') == 'milestone' for role in roles):
                names = (*names, 'milestone')
            expected = {(role, profile, native) for role in names for native in (False, True)}
            key = (protocol, profile)
        else:
            raise ValueError('unknown sealed protocol')
        actual = {(role.get('role'), role.get('profile'), role.get('native')) for role in roles}
        if len(actual) != len(roles) or actual != expected or key in coverage:
            raise ValueError('seal role coverage differs or is duplicated')
        coverage.add(key)
        if protocol == 'counter2-v1':
            flags.extend(dict(flag, profile=profile) for flag in proof.get('assessment', {}).get('cycle_flags', []))
    required = {('callgrind-v1', 'both'), ('counter2-v1', 'cgu1'), ('counter2-v1', 'cgu16'),
                ('k1-v1', 'cgu1'), ('k1-v1', 'cgu16')}
    if coverage != required or len(tips) != 1 or None in tips or len(milestones) != 1 or None in milestones:
        raise ValueError('seal protocol, tip or milestone coverage differs')
    return {'protocols': sorted([list(item) for item in coverage]), 'tip_commit': next(iter(tips)),
            'milestone_commit': next(iter(milestones)), 'cycle_flags': flags}
