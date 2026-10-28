"""Counter2-v1 six-role, five-repeat admission and workload schedule."""
import random

SUBJECTS = ['base-off', 'tip-off', 'candidate-off', 'base-on', 'tip-on', 'candidate-on']
KERNELS = [('shared_rows', 'shared_rows_main'), ('calls', 'calls_main'),
           ('numeric', 'loop_main'), ('fib', 'fib_main'), ('while', 'loop_main')]


def workload(seed, manifest):
    if type(seed) is not int or seed < 0:
        raise ValueError('counter schedule seed must be a nonnegative integer')
    if not isinstance(manifest, dict) or not isinstance(manifest.get('cells'), dict):
        raise ValueError('input manifest cells are required')
    rows = []
    for subject in SUBJECTS:
        for kernel, stream in KERNELS:
            cell = manifest['cells'].get(kernel + '/' + stream)
            if not isinstance(cell, dict) or type(cell.get('k2')) is not int or cell['k2'] <= 0:
                raise ValueError('positive K2 invocation counts are required')
            for repeat in range(5):
                rows.append({'subject': subject, 'kernel': kernel, 'stream': stream,
                             'k': cell['k2'], 'repeat': repeat})
    random.Random(seed).shuffle(rows)
    return rows


def admissions(manifest):
    if not isinstance(manifest, dict) or not isinstance(manifest.get('cells'), dict):
        raise ValueError('input manifest cells are required')
    rows = []
    for subject in SUBJECTS:
        for kernel, stream in KERNELS:
            cell = manifest['cells'].get(kernel + '/' + stream)
            if not isinstance(cell, dict) or type(cell.get('k1')) is not int or cell['k1'] <= 0:
                raise ValueError('positive K1 invocation counts are required')
            rows.append({'subject': subject, 'kernel': kernel, 'stream': stream, 'k': cell['k1']})
    return rows
