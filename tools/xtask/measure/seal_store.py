"""Create or recheck fresh local measurement evidence seals."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import role_evidence
import seal_evidence


def _write(path, value):
    with path.open('x', encoding='utf-8') as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())


def verify(spec_path, seal_path):
    """Recompute a seal without writing or relying on saved analysis values."""
    spec_path, seal_path = (Path(path).resolve(strict=True) for path in (spec_path, seal_path))
    spec_pin, seal_pin = role_evidence._pin(spec_path), role_evidence._pin(seal_path)
    raw, sealed_data = spec_path.read_bytes(), seal_path.read_bytes()
    if (hashlib.sha256(raw).hexdigest() != spec_pin['sha256']
            or hashlib.sha256(sealed_data).hexdigest() != seal_pin['sha256']):
        raise ValueError('seal or specification changed while reading')
    tool = role_evidence._pin(Path(__file__).resolve())
    evidence = seal_evidence.inspect(json.loads(raw))
    expected = {'schema': 1, 'spec': spec_pin, 'worker': tool, 'evidence': evidence}
    if (json.loads(sealed_data) != expected or role_evidence._pin(spec_path) != spec_pin
            or role_evidence._pin(seal_path) != seal_pin or role_evidence._pin(Path(__file__).resolve()) != tool):
        raise ValueError('sealed evidence differs from current inputs')
    return {'spec': spec_pin, 'seal': seal_pin, 'evidence': evidence}


def execute(operation, spec_path, out, *, seal_path=None):
    if operation not in ('create', 'check') or (operation == 'check') != (seal_path is not None):
        raise ValueError('invalid seal operation')
    spec_path = Path(spec_path).resolve(strict=True)
    spec_pin = role_evidence._pin(spec_path)
    raw = spec_path.read_bytes()
    if hashlib.sha256(raw).hexdigest() != spec_pin['sha256']:
        raise ValueError('seal specification changed while reading')
    spec = json.loads(raw)
    output = Path(out).resolve(strict=False)
    capture = Path(spec['capture']).resolve(strict=True)
    source = Path(spec['native_source']).resolve(strict=True)
    repo = Path(spec['signing_repo']).resolve(strict=True)
    for protected in (capture.parent, source, repo):
        if output == protected or output.is_relative_to(protected) or protected.is_relative_to(output):
            raise ValueError('seal output overlaps protected inputs')
    if output == spec_path or spec_path.is_relative_to(output):
        raise ValueError('seal output overlaps its specification')
    seal_pin = None
    if seal_path is not None:
        seal_path = Path(seal_path).resolve(strict=True)
        if output == seal_path or seal_path.is_relative_to(output):
            raise ValueError('seal output overlaps existing seal')
        seal_pin = role_evidence._pin(seal_path)
    tool = role_evidence._pin(Path(__file__).resolve())
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    try:
        evidence = seal_evidence.inspect(spec)
        if role_evidence._pin(spec_path) != spec_pin or role_evidence._pin(Path(__file__).resolve()) != tool:
            raise ValueError('seal specification or worker changed during check')
        manifest = {'schema': 1, 'spec': spec_pin, 'worker': tool, 'evidence': evidence}
        if operation == 'create':
            _write(output / 'SEAL.json', manifest)
            seal_pin = role_evidence._pin(output / 'SEAL.json')
        else:
            sealed = json.loads(seal_path.read_bytes())
            if role_evidence._pin(seal_path) != seal_pin or sealed != manifest:
                raise ValueError('sealed evidence differs from current inputs')
        result = {'schema': 1, 'status': 'PASS', 'operation': operation,
                  'spec': spec_pin, 'seal': seal_pin, 'candidate_commit': evidence['candidate_commit'],
                  'protocol': evidence['protocol'], 'assessment': evidence['assessment'],
                  'seconds': time.monotonic() - started}
        _write(output / 'RESULT.json', result)
        _write(output / 'COMPLETE.json', {'schema': 1, 'status': 'PASS', 'operation': operation,
                                         'result': role_evidence._pin(output / 'RESULT.json'), 'seal': seal_pin})
        return result
    except BaseException as error:
        _write(output / 'FAILURE.json', {'schema': 1, 'status': 'FAIL', 'operation': operation,
                                        'error': f'{type(error).__name__}: {error}',
                                        'seconds': time.monotonic() - started})
        raise


def main():
    parser = argparse.ArgumentParser(description='Create or recheck fresh measurement seals')
    parser.add_argument('operation', choices=('create', 'check'))
    parser.add_argument('--spec', required=True)
    parser.add_argument('--out', required=True)
    parser.add_argument('--seal')
    args = parser.parse_args()
    result = execute(args.operation, args.spec, args.out, seal_path=args.seal)
    print(f"seal {result['operation']} PASS: {Path(args.out).resolve()}")


if __name__ == '__main__':
    main()
