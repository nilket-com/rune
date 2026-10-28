"""Complete callgrind packet controls over synthetic profiles."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import analysis_sandbox
import callgrind_packet
import callgrind_rules
import gen_inputs
import input_closure


class CallgrindPacketTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'package'
        self.root.mkdir()
        self.tools = Path(__file__).resolve().parent
        self.code = input_closure.create(self.tools, ['callgrind_packet.py', 'callgrind.py',
                                                     'callgrind_rules.py', 'qualification_policy.py', 'callgrind_guard.py', 'gen_inputs.py', 'input_closure.py'])
        self.spec = {'positions': 'line', 'symbol_map': None,
                     'controls': {'profiles': {}, 'guard': None}, 'phases': {}}
        for control in ('is', 'branch'):
            self.spec['controls']['profiles'][control] = {
                f'{role}{n}': self.write_profile(f'{role}-{control}-{n}',
                    gen_inputs.checksum([0 if n == 0 else (300 if control == 'is' else 150)] * 1000))
                for role in ('tip', 'candidate') for n in (0, 100)}
        (self.root / 'loop.inputs').write_text('10\n' * 1000)
        for mode in ('off', 'on'):
            cells = {}
            for kernel in callgrind_rules.KERNELS:
                checksum = gen_inputs.checksum([gen_inputs.model(kernel, 10)] * 1000)
                cell = {role: self.write_profile(f'{role}-{mode}-{kernel}', checksum)
                        for role in ('tip', 'candidate', 'reference')}
                cell['input_file'] = 'loop.inputs'
                cells[kernel] = cell
            self.spec['phases'][mode] = {'cells': cells, 'guard': None}

    def write_profile(self, name, checksum):
        profile = name + '.cg'
        stdout = name + '.stdout'
        (self.root / profile).write_text('positions: line\nevents: Ir\nfn=(1) experimental_core::work\n1 10000\nsummary: 10000\n')
        (self.root / stdout).write_text(f'CHECKSUM {checksum} invocations=1000\n')
        return {'profile': profile, 'stdout': stdout}

    def sandbox(self, proof=None):
        (self.root / 'SPEC.json').write_text(json.dumps(self.spec))
        pin = input_closure.create(self.root, callgrind_packet.declared(self.spec))
        return json.loads(analysis_sandbox.run(self.root, pin, self.tools, self.code, 'callgrind_packet.py',
                                               verified_facts=proof))

    def test_complete_phases_in_isolated_package(self):
        result = self.sandbox()
        self.assertEqual(set(result), {'on', 'off'})
        self.assertEqual(set(result['off']['cells']), callgrind_rules.KERNELS)
        self.assertEqual(result['on']['cells']['while']['rule'], 'none')
        self.assertEqual(result['off']['controls']['is']['core'], 0)

    def test_optional_symbol_map_and_changed_checksum(self):
        cell = self.spec['phases']['off']['cells']['while']
        original = cell['candidate']['profile']
        cell['candidate']['profile'] = cell['tip']['profile']
        with self.assertRaisesRegex(ValueError, 'capture files must be distinct'):
            callgrind_packet.declared(self.spec)
        cell['candidate']['profile'] = original
        self.spec['symbol_map'] = 'SYMBOL-NAME-MAP.json'
        with self.assertRaises(ValueError):
            self.sandbox()
        (self.root / 'SYMBOL-NAME-MAP.json').write_text('{}')
        self.sandbox()
        bad = self.spec['phases']['off']['cells']['while']['candidate']['stdout']
        (self.root / bad).write_text('CHECKSUM 5 invocations=1000\n')
        with self.assertRaisesRegex(ValueError, 'analyzer failed'):
            self.sandbox()

    def test_qualified_packet_requires_fresh_proof(self):
        self.spec['qualifications'] = json.loads((self.tools / 'qualifications/s8.json').read_text())
        self.spec['qualifications']['candidate_commit'] = 'b' * 40
        proof = {'schema': 1, 'tip_commit': 'a' * 40, 'candidate_commit': 'b' * 40,
                 'control_bytecode_equal': True, 'indexing_source_equal': True, 'entry_source_equal': True,
                 'control_bytecode_sha256': 'c' * 64}
        with self.assertRaisesRegex(ValueError, 'analyzer failed'):
            self.sandbox()
        result = self.sandbox(proof)
        self.assertEqual(result['off']['qualifications'], self.spec['qualifications']['entries'])
        with self.assertRaisesRegex(ValueError, 'analyzer failed'):
            self.sandbox(dict(proof, control_bytecode_equal=False))


if __name__ == '__main__':
    unittest.main()
