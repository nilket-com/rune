"""Validate a role against an isolated, commit-based measurement build.

This is a component of seal validation, not a complete seal. Controllers and
all declared analysis inputs must also be checked before evidence is accepted.
"""
import json
import hashlib
from pathlib import Path
import re
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import input_closure
import build_worker
import build_recipe
import cargo_cache
import source_export
import tool_identity


def _pin(path):
    path = Path(path)
    if not path.is_absolute() or path.is_symlink():
        raise ValueError('evidence paths must be absolute regular files')
    record = input_closure._pin(path.parent, path.name)
    return {'path': str(path), 'sha256': record['sha256'], 'bytes': record['bytes']}


def check(repo, role):
    required = {'name', 'role', 'profile', 'native', 'commit', 'build', 'binary'}
    if not isinstance(role, dict) or set(role) != required:
        raise ValueError('invalid build role schema')
    if (not isinstance(role['name'], str) or not role['name']
            or role['role'] not in ('candidate', 'tip', 'base', 'milestone')
            or role['profile'] not in ('cgu1', 'cgu16') or type(role['native']) is not bool
            or not isinstance(role['commit'], str) or not re.fullmatch('[0-9a-f]{40}', role['commit'])):
        raise ValueError('invalid build role identity')
    for field in ('build', 'binary'):
        if not isinstance(role[field], dict) or set(role[field]) != {'path', 'sha256', 'bytes'}:
            raise ValueError(f'invalid {field} pin')
        if _pin(role[field]['path']) != role[field]:
            raise ValueError(f'{field} pin differs')
    if Path(role['build']['path']).name != 'RESULT.json':
        raise ValueError('build must reference its RESULT.json')
    result = json.loads(Path(role['build']['path']).read_bytes())
    if (result.get('schema') != 1 or result.get('status') != 'PASS'
            or any(result.get(field) != role[field] for field in ('role', 'profile', 'native'))
            or result.get('binary') != role['binary']):
        raise ValueError('build receipt differs from role')
    source = result.get('source')
    if not isinstance(source, dict) or source.get('status') != 'PASS' or source.get('commit') != role['commit']:
        raise ValueError('build source differs from role commit')
    observed = source_export.verify(source['source'], role['commit'])
    if observed['tree'] != source.get('tree') or observed['files'] != source.get('files'):
        raise ValueError('fresh source export differs from build receipt')
    signed = None
    if role['role'] == 'candidate':
        signed = source_export.verify_signed(repo, source['source'], role['commit'])
    recipe = result.get('recipe')
    if (not isinstance(recipe, dict) or recipe.get('status') != 'PASS'
            or recipe.get('commit') != role['commit'] or recipe.get('tree') != observed['tree']
            or recipe.get('profile') != role['profile']):
        raise ValueError('build recipe differs from source or profile')
    if (recipe.get('execution_feature'), recipe.get('native_feature')) != build_recipe.feature_names(source['source']):
        raise ValueError('build feature names differ from source')
    run = Path(role['build']['path']).parent / 'run'
    recipe_files = recipe.get('files')
    if not isinstance(recipe_files, dict) or set(recipe_files) != {'Cargo.toml', 'Cargo.lock', 'src/main.rs'}:
        raise ValueError('build recipe file pins are missing')
    for name, expected in recipe_files.items():
        actual = _pin(run / 'harness' / name)
        if not isinstance(expected, dict) or expected != {'sha256': actual['sha256'], 'bytes': actual['bytes']}:
            raise ValueError('build recipe input differs')
    if recipe.get('harness_inputs') != build_recipe._harness_inputs():
        raise ValueError('build harness inputs differ')
    registry = result.get('registry')
    if (not isinstance(registry, dict) or not isinstance(registry.get('root'), str)
            or not Path(registry['root']).is_absolute()
            or cargo_cache.verify(run / 'harness/Cargo.lock', registry['root']) != registry):
        raise ValueError('build registry inputs differ')
    if result.get('implementation') != build_worker._implementation_pins():
        raise ValueError('build worker implementation differs')
    invocations = result.get('invocations')
    if not isinstance(invocations, dict) or not all(isinstance(invocations.get(k), list)
                                                    for k in ('compilers', 'linkers', 'linker_drivers', 'trace_files')):
        raise ValueError('compiler invocation evidence is missing')
    tools = result.get('tool_pins')
    if not isinstance(tools, list) or len(tools) < 4:
        raise ValueError('compiler and linker pins are missing')
    for pin in tools:
        if _pin(pin['path']) != pin:
            raise ValueError('compiler or linker pin differs')
    recorded = invocations['compilers'] + invocations['linkers'] + invocations['linker_drivers']
    validated = tool_identity.validate_invocations(recorded, tools[0]['path'], tools[2]['path'], tools[3]['path'])
    if any(validated[key] != invocations[key] for key in ('compilers', 'linkers', 'linker_drivers')):
        raise ValueError('compiler or linker invocation classification differs')
    actual = set()
    for pin in invocations['trace_files']:
        if _pin(pin['path']) != pin:
            raise ValueError('execution trace changed')
        for line in Path(pin['path']).read_text().splitlines():
            row = tool_identity.parse_exec_line(line)
            if row:
                actual.add((pin['path'], row['path'], tuple(row['argv'])))
    for row in recorded:
        if (row['trace']['path'], row['path'], tuple(row['argv'])) not in actual:
            raise ValueError('recorded invocation is absent from execution traces')
    toolchain = Path(tools[0]['path']).parent.parent
    if tool_identity.sysroot_identity(toolchain, result.get('host')) != result.get('sysroot'):
        raise ValueError('compiler sysroot differs')
    observed_traces = build_worker._trace(run, Path(source['source']), toolchain,
                                          Path(tools[0]['path']), Path(tools[2]['path']),
                                          Path(tools[3]['path']))
    if observed_traces != invocations:
        raise ValueError('reconstructed execution traces differ from build receipt')
    if tool_identity.validate_rune_feature(invocations['compilers'], role['native'], recipe['native_feature']) != result.get('rune_compilations'):
        raise ValueError('compiler feature evidence differs')
    identity = result.get('elf_identity')
    if not isinstance(identity, dict) or set(identity) != {'rustc', 'linker'}:
        raise ValueError('ELF identity is missing')
    config = result.get('host_config')
    if not isinstance(config, dict) or set(config) != {'path', 'sha256', 'contents'}:
        raise ValueError('host configuration receipt is missing')
    contents = config['contents']
    if (not isinstance(contents, str) or hashlib.sha256(contents.encode()).hexdigest() != config['sha256']
            or _pin(config['path'])['sha256'] != config['sha256']):
        raise ValueError('host configuration differs from build receipt')
    host = json.loads(contents).get('measurement')
    if (not isinstance(host, dict) or host.get('rustc_identity') != identity['rustc']
            or host.get('linker_identity') != identity['linker']):
        raise ValueError('ELF identity differs from declared host tools')
    sections = subprocess.run(['/usr/bin/readelf', '-S', '-W', role['binary']['path']],
                              capture_output=True, text=True, check=False)
    if sections.returncode or re.search(r'\s\.debug_\w+', sections.stdout):
        raise ValueError('binary is not a release ELF')
    comment = subprocess.run(['/usr/bin/readelf', '-p', '.comment', role['binary']['path']],
                             capture_output=True, text=True, check=False)
    if comment.returncode or tool_identity.elf_identity(comment.stdout, identity['rustc'], identity['linker']) != identity:
        raise ValueError('ELF compiler or linker identity differs')
    if (source_export.verify(source['source'], role['commit']) != observed
            or build_worker._implementation_pins() != result['implementation']
            or build_recipe._harness_inputs() != recipe['harness_inputs']
            or cargo_cache.verify(run / 'harness/Cargo.lock', registry['root']) != registry
            or tool_identity.sysroot_identity(toolchain, result.get('host')) != result['sysroot']):
        raise ValueError('build source or tools changed during validation')
    for name, expected in recipe_files.items():
        actual = _pin(run / 'harness' / name)
        if expected != {'sha256': actual['sha256'], 'bytes': actual['bytes']}:
            raise ValueError('build recipe input changed during validation')
    for pin in tools + invocations['trace_files']:
        if _pin(pin['path']) != pin:
            raise ValueError('build tool or trace changed during validation')
    if _pin(config['path'])['sha256'] != config['sha256']:
        raise ValueError('host configuration changed during validation')
    if _pin(role['binary']['path']) != role['binary'] or _pin(role['build']['path']) != role['build']:
        raise ValueError('build evidence changed during validation')
    return {'name': role['name'], 'role': role['role'], 'commit': role['commit'],
            'profile': role['profile'], 'native': role['native'], 'binary': role['binary'],
            'build': role['build'], 'source_tree': observed['tree'], 'signed': signed}
