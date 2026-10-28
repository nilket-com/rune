"""Prove the reviewed iterator guard exception from fresh binaries and profiles."""
import difflib
import hashlib
from pathlib import Path
import re
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import callgrind_rules

ITER = callgrind_rules.ITER


def normalize(text):
    result = []
    for line in text.splitlines():
        match = re.match(r'\s*([0-9a-f]+):\s+(.*)', line)
        if not match:
            continue
        instruction = re.sub(r'\b[0-9a-f]+ <<rune::experimental_core::heap::Store>::iterator_next\+0x[0-9a-f]+>', '<SELF>', match[2])
        instruction = re.sub(r'\b[0-9a-f]+\s+<([^>]+)>', r'<\1>', instruction)
        instruction = re.sub(r'-?0x[0-9a-f]+\(%rip\)', 'RELOC(%rip)', instruction)
        instruction = re.sub(r'#.*$', '', instruction).rstrip()
        result.append(re.sub(r'%[a-z0-9]+', '%R', instruction))
    return result


def compare_instructions(tip, candidate):
    a, b = normalize(tip), normalize(candidate)
    removed, added = [], []
    for operation, i1, i2, j1, j2 in difflib.SequenceMatcher(None, a, b, autojunk=False).get_opcodes():
        if operation != 'equal':
            removed.extend(a[i1:i2])
            added.extend(b[j1:j2])
    toggle = (sorted(removed) == sorted(['cmpl   $0x0,0x44(%R)'] + [x for x in removed if x == 'mov    %R,%R'])
              and sorted(added) == sorted(['mov    0x44(%R),%R', 'test   %R,%R']
                                          + [x for x in added if x == 'mov    %R,%R']))
    calls = lambda rows: sorted(x for x in rows if x.startswith('call'))
    return len(a) == len(b) == 357 and ((toggle and calls(a) == calls(b)) or a == b)


def disassemble(binary):
    binary = str(Path(binary).resolve(strict=True))
    result = subprocess.run(['/usr/bin/nm', '-S', '--defined-only', '-C', binary],
                            capture_output=True, text=True, check=True)
    matches = [line.split(maxsplit=3) for line in result.stdout.splitlines()
               if len(line.split(maxsplit=3)) == 4 and line.split(maxsplit=3)[-1] == ITER]
    if len(matches) != 1:
        raise ValueError('iterator symbol is missing or ambiguous')
    start, size = (int(matches[0][i], 16) for i in (0, 1))
    return subprocess.run(['/usr/bin/objdump', '-Cd', '--no-show-raw-insn',
                           '--start-address=' + hex(start), '--stop-address=' + hex(start + size), binary],
                          capture_output=True, text=True, check=True).stdout


def _digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def prove(tip_source, candidate_source, tip_binary, candidate_binary, candidate_profile):
    """Check source identity, exact symbolized code change and per-advance cost."""
    paths = [Path(path).resolve(strict=True) for path in (tip_source, candidate_source, tip_binary, candidate_binary)]
    before = [_digest(path) for path in paths]
    if before[0] != before[1]:
        raise ValueError('iterator source differs')
    if not compare_instructions(disassemble(paths[2]), disassemble(paths[3])):
        raise ValueError('iterator disassembly differs from the reviewed guard toggle')
    iterator = candidate_profile['functions'].get(ITER, {})
    count, cost = iterator.get('calls', 0) or 0, iterator.get('self_Ir', 0) or 0
    if type(count) is not int or type(cost) is not int or count < 0 or cost < 0 or (count and cost > 49 * count):
        raise ValueError('iterator per-advance cost exceeds its limit')
    if [_digest(path) for path in paths] != before:
        raise ValueError('iterator proof inputs changed')
    return {'source_sha256': before[0], 'tip_binary_sha256': before[2],
            'candidate_binary_sha256': before[3], 'calls': count, 'self_Ir': cost}
