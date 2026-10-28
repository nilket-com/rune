"""Frozen F1 regression-screen cells, schedule and arithmetic bounds."""
import json, random
KERNELS = [('numeric','loop_main'),('while','loop_main'),('calls','calls_main'),
           ('fib','fib_main'),('shared_rows','shared_rows_main')]
SUBJECTS = ['cand-on','cand-off','base-on','base-on-prime','base-off']
ROUNDS = 10
B = 10000
CHILD_TIMEOUT_S = 120
SESSION_BOUND_S = 10800
COUNTS = dict(rounds=240,blocks=480,timed_children=720,validation_children=50)
PROBE = 'rnx-0185-f1-regression-screen-v1'

def cells():
    result=[]
    for kernel,stream in KERNELS:
        for config,pair in [('on',['cand-on','base-on']),('off',['cand-off','base-off'])]:
            for endpoint in ['hot','cold']:
                result.append(dict(kernel=kernel,stream=stream,config=config,endpoint=endpoint,subjects=pair))
        if kernel in ('fib','shared_rows'):
            for endpoint in ['hot','cold']:
                result.append(dict(kernel=kernel,stream=stream,config='aa',endpoint=endpoint,
                                   subjects=['base-on','base-on-prime']))
    return result

def schedule(seed,manifest):
    rng=random.Random(seed)
    rounds=[dict(cell,round=r) for cell in cells() for r in range(ROUNDS)]
    rng.shuffle(rounds)
    for ri,row in enumerate(rounds):
        info=manifest['cells'][row['kernel']+'/'+row['stream']]
        subjects=list(row['subjects']);rng.shuffle(subjects)
        blocks=[]
        for subject in subjects:
            order=[1] if row['endpoint']=='cold' else [info['k1'],info['k2']]
            if len(order)==2 and rng.random()>=0.5:order.reverse()
            blocks.append(dict(subject=subject,order=order))
        row.update(round_index=ri,blocks=blocks)
    assert len(rounds)==COUNTS['rounds']
    assert sum(len(r['blocks']) for r in rounds)==COUNTS['blocks']
    assert sum(len(b['order']) for r in rounds for b in r['blocks'])==COUNTS['timed_children']
    return rounds

def expected_maps(subject,kernel):
    return 1 if subject in ('cand-on','base-on','base-on-prime') and kernel in ('numeric','while','calls') else 0
