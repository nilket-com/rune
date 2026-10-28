"""Iterator guard proof controls over synthetic disassembly and profiles."""
from pathlib import Path
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import callgrind_guard
import analysis_sandbox
import input_closure


def assembly(instructions):
    return '\n'.join(f'{i:x}: {instruction}' for i, instruction in enumerate(instructions))


class GuardTests(unittest.TestCase):
    def test_exact_toggle_and_instruction_limit(self):
        tip = assembly(['cmpl   $0x0,0x44(%rax)', 'mov    %rbx,%rcx'] + ['nop'] * 355)
        candidate = assembly(['mov    0x44(%rax),%rcx', 'test   %rcx,%rcx'] + ['nop'] * 355)
        self.assertTrue(callgrind_guard.compare_instructions(tip, candidate))
        self.assertFalse(callgrind_guard.compare_instructions(tip, candidate.replace('0x44', '0x48')))
        self.assertFalse(callgrind_guard.compare_instructions(tip, candidate + '\n200: nop'))

    def test_source_and_per_advance_checks(self):
        with tempfile.TemporaryDirectory() as tmp:
            paths = [Path(tmp) / str(i) for i in range(4)]
            for path in paths:
                path.write_text('same')
            asm = assembly(['nop'] * 357)
            profile = {'functions': {callgrind_guard.ITER: {'calls': 10, 'self_Ir': 490}}}
            with patch.object(callgrind_guard, 'disassemble', return_value=asm):
                result = callgrind_guard.prove(*paths, profile)
                self.assertEqual(result['calls'], 10)
                profile['functions'][callgrind_guard.ITER]['self_Ir'] = 491
                with self.assertRaisesRegex(ValueError, 'per-advance'):
                    callgrind_guard.prove(*paths, profile)
                paths[1].write_text('changed')
                with self.assertRaisesRegex(ValueError, 'source differs'):
                    callgrind_guard.prove(*paths, profile)

    def test_full_binary_proof_in_restricted_runtime(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package, code = root / 'package', root / 'code'
            package.mkdir()
            code.mkdir()
            symbol = callgrind_guard.ITER
            assembly_file = root / 'guard.s'
            assembly_file.write_text(f'.globl "{symbol}"\n.type "{symbol}", @function\n"{symbol}":\n'
                                     f'.rept 357\nnop\n.endr\n.size "{symbol}", .-"{symbol}"\n')
            binary = package / 'tip-binary'
            subprocess.run(['/usr/bin/as', '-o', str(binary), str(assembly_file)], check=True, capture_output=True)
            shutil.copyfile(binary, package / 'candidate-binary')
            for name in ('tip-source', 'candidate-source'):
                (package / name).write_text('same source')
            for name in ('callgrind_guard.py', 'callgrind_rules.py', 'qualification_policy.py', 'callgrind.py'):
                shutil.copyfile(Path(__file__).resolve().parent / name, code / name)
            (code / 'entry.py').write_text(
                "import json,sys;sys.path.insert(0,'/tool');import callgrind_guard as g;"
                "p={'functions':{g.ITER:{'calls':10,'self_Ir':490}}};"
                "print(json.dumps(g.prove('/inputs/tip-source','/inputs/candidate-source',"
                "'/inputs/tip-binary','/inputs/candidate-binary',p)))\n")
            files = ['tip-source', 'candidate-source', 'tip-binary', 'candidate-binary']
            code_files = ['entry.py', 'callgrind_guard.py', 'callgrind_rules.py', 'qualification_policy.py', 'callgrind.py']
            result = analysis_sandbox.run(package, input_closure.create(package, files), code,
                                          input_closure.create(code, code_files), 'entry.py')
            self.assertEqual(json.loads(result)['calls'], 10)


if __name__ == '__main__':
    unittest.main()
