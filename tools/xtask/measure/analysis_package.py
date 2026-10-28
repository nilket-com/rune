"""Prepare and recheck isolated raw-profile analysis input packages.

This component is not a full measurement seal: it does not bind raw profiles
to controller receipts, role builds or signed source snapshots.
"""
import hashlib
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_sandbox
import analysis_runtime
import callgrind_packet
import callgrind_trend
import counter_packet
import input_closure
import k1_analysis
import k1_packet

CODE_ROOT = Path(__file__).resolve().parent
PROTOCOLS = {
    'callgrind-cgu1-v1': (callgrind_packet, 'callgrind_packet.py', ['callgrind_packet.py', 'callgrind.py', 'callgrind_rules.py', 'qualification_policy.py', 'callgrind_guard.py', 'gen_inputs.py', 'input_closure.py']),
    'callgrind-cgu16-v1': (callgrind_trend, 'callgrind_trend.py', ['callgrind_trend.py', 'callgrind.py', 'callgrind_rules.py', 'qualification_policy.py', 'gen_inputs.py', 'input_closure.py']),
    'counter2-v1': (counter_packet, 'counter_packet.py', ['counter_packet.py', 'counter_schedule.py', 'counter_cell.py', 'counter_assess.py', 'perf_parse.py', 'input_closure.py']),
    'k1-series-v1': (k1_analysis, 'k1_analysis.py', ['k1_analysis.py', 'k1.py', 'counter_cell.py', 'counter_assess.py', 'perf_parse.py', 'input_closure.py']),
    'k1-packet-v1': (k1_packet, 'k1_packet.py', ['k1_packet.py', 'k1.py', 'counter_cell.py', 'counter_packet.py', 'counter_schedule.py', 'counter_assess.py', 'perf_parse.py', 'input_closure.py']),
}


def inspect(package, protocol):
    if protocol not in PROTOCOLS:
        raise ValueError('unknown analysis protocol')
    root = Path(package).resolve(strict=True)
    module, entry, files = PROTOCOLS[protocol]
    spec_data = (root / 'SPEC.json').read_bytes()
    paths = module.declared(json.loads(spec_data))
    package_pin = input_closure.create(root, paths)
    spec_pin = next(item for item in package_pin['files'] if item['path'] == 'SPEC.json')
    if hashlib.sha256(spec_data).hexdigest() != spec_pin['sha256']:
        raise ValueError('analysis specification changed during declaration')
    code_pin = input_closure.create(CODE_ROOT, files)
    orchestrator_pin = input_closure.create(CODE_ROOT, ['analysis_package.py', 'analysis_sandbox.py', 'analysis_runtime.py', 'input_closure.py'])
    return {'schema': 1, 'protocol': protocol, 'entry': entry,
            'package': package_pin, 'code': code_pin, 'orchestrator': orchestrator_pin,
            'runtime': analysis_runtime.discover()}


def analyze(package, manifest, *, verified_facts=None):
    if not isinstance(manifest, dict) or set(manifest) != {'schema', 'protocol', 'entry', 'package', 'code', 'orchestrator', 'runtime'}:
        raise ValueError('invalid analysis package manifest')
    if manifest['schema'] != 1 or inspect(package, manifest['protocol']) != manifest:
        raise ValueError('analysis inputs or code differ from package manifest')
    root = Path(package).resolve(strict=True)
    if verified_facts is not None and manifest['protocol'] != 'callgrind-cgu1-v1':
        raise ValueError('qualification proof is not supported for this protocol')
    output = analysis_sandbox.run(root, manifest['package'], CODE_ROOT, manifest['code'], manifest['entry'], runtime=manifest['runtime'], verified_facts=verified_facts)
    if inspect(package, manifest['protocol']) != manifest:
        raise ValueError('analysis inputs changed during execution')
    return json.loads(output)
