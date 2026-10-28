"""Collect and enforce declared local measurement readiness limits."""
import math
import os
from pathlib import Path
import re

FORBIDDEN = frozenset({'cargo', 'rustc', 'perf', 'lua', 'luajit', 'rnx', 'miri'})


def snapshot(cpu, *, proc_root='/proc', sys_root='/sys'):
    if type(cpu) is not int or cpu < 0:
        raise ValueError('invalid measurement CPU')
    proc, system = Path(proc_root), Path(sys_root)
    load = float((proc / 'loadavg').read_text().split()[0])
    if not math.isfinite(load) or load < 0:
        raise ValueError('invalid load average')
    match = re.search(r'^MemAvailable:\s+(\d+)\s+kB$', (proc / 'meminfo').read_text(), re.M)
    if match is None:
        raise ValueError('available memory is missing')
    available = int(match[1]) * 1024
    throttle_text = (system / f'devices/system/cpu/cpu{cpu}/thermal_throttle/core_throttle_count').read_text().strip()
    if not throttle_text.isdecimal():
        raise ValueError('thermal throttle counter is invalid')
    ancestors = {os.getpid()}
    current = os.getpid()
    while current:
        try:
            status = (proc / str(current) / 'status').read_text()
        except FileNotFoundError:
            break
        parent = re.search(r'^PPid:\s+(\d+)$', status, re.M)
        if parent is None:
            raise ValueError('process parent is missing')
        current = int(parent[1])
        if current in ancestors:
            raise ValueError('process parent cycle')
        if current:
            ancestors.add(current)
    busy = []
    for path in proc.iterdir():
        if not path.name.isdecimal() or int(path.name) in ancestors:
            continue
        try:
            name = (path / 'comm').read_text().strip()
        except (FileNotFoundError, ProcessLookupError):
            continue
        if name in FORBIDDEN or name.startswith('p2-probe'):
            busy.append({'pid': int(path.name), 'name': name})
    return {'cpu': cpu, 'load': load, 'mem_available_bytes': available,
            'core_throttle_count': int(throttle_text), 'busy': sorted(busy, key=lambda item: item['pid'])}


def require_ready(observation, *, max_load, min_available_bytes):
    if (type(max_load) not in (int, float) or not math.isfinite(max_load) or max_load <= 0
            or type(min_available_bytes) is not int or min_available_bytes <= 0):
        raise ValueError('readiness limits must be declared and positive')
    if observation['busy'] or observation['load'] > max_load or observation['mem_available_bytes'] < min_available_bytes:
        raise ValueError('measurement host is not ready')
    return {'status': 'READY', 'observation': observation,
            'limits': {'max_load': max_load, 'min_available_bytes': min_available_bytes}}
