"""Check a ledger and atomically append a record backed by fresh seals."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import stat
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ledger_store
import ledger_transition
import milestone_registry
import record_policy
import role_evidence
import seal_store


def _read(path):
    path = Path(path)
    if not path.is_absolute() or path.is_symlink():
        raise ValueError('input must be an absolute non-symlink path')
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), 'rb') as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise ValueError('input must be a regular file')
        return stream.read()


def _digest(data):
    return hashlib.sha256(data).hexdigest()


def execute(operation, spec_path, hosts_path, out):
    spec_path, hosts_path = Path(spec_path), Path(hosts_path)
    raw, host_raw = _read(spec_path), _read(hosts_path)
    registry_pin = role_evidence._pin(milestone_registry.REGISTRY)
    registry = milestone_registry.load()
    if role_evidence._pin(milestone_registry.REGISTRY) != registry_pin:
        raise ValueError('milestone registry changed while reading')
    spec, hosts = json.loads(raw), json.loads(host_raw)
    keys = {'schema', 'ledger', 'expected_entries'}
    if operation == 'append':
        keys |= {'entry', 'milestone', 'seals'}
    if operation not in ('check', 'summary', 'append') or set(spec) != keys or spec['schema'] != 1:
        raise ValueError('invalid record specification')
    ledger = spec['ledger']
    if not isinstance(ledger, dict) or set(ledger) != {'path', 'sha256', 'bytes'}:
        raise ValueError('invalid ledger pin')
    if type(spec['expected_entries']) is not int or spec['expected_entries'] < 0:
        raise ValueError('invalid expected entry count')
    lock = hosts.get('measurement', {}).get('measurement_lock')
    if not isinstance(lock, str) or not Path(lock).is_absolute():
        raise ValueError('measurement lock must be an absolute path')
    ledger_path = Path(ledger['path'])
    output = Path(out).absolute()
    protected = [spec_path.absolute(), hosts_path.absolute(), ledger_path.absolute(), Path(lock)]
    if any(p == output or p.is_relative_to(output) for p in protected):
        raise ValueError('output overlaps protected input')
    output.mkdir(parents=True, exist_ok=False)

    def verify_inputs():
        if (_read(spec_path) != raw or _read(hosts_path) != host_raw
                or role_evidence._pin(milestone_registry.REGISTRY) != registry_pin):
            raise ValueError('record specification or host configuration changed')

    def before():
        data = _read(ledger_path)
        if len(data) != ledger['bytes'] or _digest(data) != ledger['sha256']:
            raise ValueError('ledger differs from declared pin')
        value = json.loads(data)
        ledger_transition._ledger(value)
        if len(value['entries']) != spec['expected_entries']:
            raise ValueError('ledger entry count differs')
        anchor = registry['milestones'][-1]
        if (not value['milestones'] or value['milestones'][-1].get('id') != anchor['id']
                or value['milestones'][-1].get('commit') != anchor['commit']):
            raise ValueError('ledger milestone differs from committed registry')
        return value

    try:
        first = before()
        verification = None
        if operation == 'append':
            entry = spec['entry']
            allowed = {'record', 'subject', 'date', 'commit', 'kind', 'product_code_changed',
                       'product_profile_changed', 'counts_toward_milestone_cadence', 'milestone',
                       'notes', 'evidence'}
            if not isinstance(entry, dict) or set(entry) - allowed:
                raise ValueError('record fields outside the verified evidence are not supported')
            expected_after, _ = ledger_transition.transition(first, entry, spec['milestone'])
            seals = spec['seals']
            if not isinstance(seals, list) or len(seals) != 5 or any(not isinstance(x, dict) or set(x) != {'spec', 'seal'} for x in seals):
                raise ValueError('five seal specifications are required')
            def evidence_check():
                nonlocal verification
                verify_inputs()
                verified = [seal_store.verify(item['spec'], item['seal']) for item in seals]
                proofs = [item['evidence'] for item in verified]
                if any(proof.get('host_config', {}).get('sha256') != _digest(host_raw) for proof in proofs):
                    raise ValueError('sealed host configuration differs')
                verification = record_policy.validate(proofs, entry['commit'])
                anchor = registry['milestones'][-1]
                if (not first['milestones'] or first['milestones'][-1].get('id') != anchor['id']
                        or first['milestones'][-1].get('commit') != anchor['commit']
                        or anchor['commit'] != verification['milestone_commit']):
                    raise ValueError('sealed milestone differs from current ledger anchor')
                expected = {'seals': [{'spec': item['spec'], 'seal': item['seal'],
                                       'protocol': item['evidence']['protocol'],
                                       'assessment': item['evidence']['assessment']} for item in verified],
                            'coverage': verification}
                if entry.get('evidence') != expected:
                    raise ValueError('entry evidence must exactly reproduce verified fresh assessments')
                verify_inputs()
            status = ledger_store.append(ledger_path, lock, ledger['sha256'], entry, spec['milestone'], evidence_check)
            after = json.loads(_read(ledger_path))
            if after != expected_after:
                raise ValueError('ledger differs after atomic append')
        else:
            flags = os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK
            with os.fdopen(os.open(lock, flags, 0o600), 'r+b') as stream:
                if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
                    raise ValueError('lock must be a regular file')
                fcntl.flock(stream, fcntl.LOCK_EX)
                verify_inputs()
                first = before()
            after, status = first, 'checked'
        result = {'schema': 1, 'status': 'PASS', 'operation': operation, 'transition': status,
                  'ledger': role_evidence._pin(ledger_path), 'entries': len(after['entries']),
                  'milestones': len(after['milestones']), 'registry': registry_pin, 'coverage': verification}
        seal_store._write(output / 'RESULT.json', result)
        seal_store._write(output / 'COMPLETE.json', {'schema': 1, 'status': 'PASS',
                                                     'result': role_evidence._pin(output / 'RESULT.json')})
        return result
    except BaseException as error:
        seal_store._write(output / 'FAILURE.json', {'schema': 1, 'status': 'FAIL',
                                                    'error': f'{type(error).__name__}: {error}'})
        raise


def main():
    parser = argparse.ArgumentParser(description='Check or atomically update the measurement ledger')
    parser.add_argument('operation', choices=('check', 'summary', 'append'))
    parser.add_argument('--spec', required=True)
    parser.add_argument('--hosts', required=True)
    parser.add_argument('--out', required=True)
    args = parser.parse_args()
    result = execute(args.operation, args.spec, args.hosts, args.out)
    print(f"record {args.operation} {result['status']}: {Path(args.out).absolute()}")


if __name__ == '__main__':
    main()
