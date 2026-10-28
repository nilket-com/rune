"""Prepare a measurement harness for a verified, explicitly supplied source commit."""
import hashlib
import json
from pathlib import Path
import shutil
import tomllib

import source_export

HARNESS = Path(__file__).resolve().parent / 'harness'


def _harness_inputs():
    result = {}
    for name in ('Cargo.toml', 'Cargo.lock', 'src/main.rs'):
        path = HARNESS / name
        if path.is_symlink() or not path.is_file():
            raise ValueError(f'invalid harness input: {name}')
        result[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    return result


def feature_names(source):
    features = tomllib.loads((Path(source) / 'crates/rune/Cargo.toml').read_text()).get('features', {})
    choices = [('execution', 'native-code'), ('fresh-core', 'fresh-core-native')]
    matches = [pair for pair in choices if all(name in features for name in pair)]
    if len(matches) != 1:
        raise ValueError('source must declare exactly one supported execution and native feature pair')
    return matches[0]


def prepare(source, commit, profile, out, *, source_alias=None):
    if profile not in ('cgu1', 'cgu16'):
        raise ValueError('profile must be cgu1 or cgu16')
    source = Path(source).resolve(strict=True)
    out = Path(out).resolve(strict=False)
    if source == out or source.is_relative_to(out) or out.is_relative_to(source):
        raise ValueError('recipe output and source must be disjoint')
    before = source_export.verify(source, commit)
    harness_before = _harness_inputs()
    rune = source / 'crates' / 'rune'
    if not (rune / 'Cargo.toml').is_file():
        raise ValueError('Rune crate is missing from the verified source')
    execution_feature, native_feature = feature_names(source)
    manifest = (HARNESS / 'Cargo.toml').read_text()
    if execution_feature == 'fresh-core':
        manifest = manifest.replace('features = ["execution"]', 'features = ["fresh-core"]')
        manifest = manifest.replace('rune/native-code', 'rune/fresh-core-native')
    original_path = 'path = "../../../../crates/rune"'
    original_units = 'codegen-units = 1'
    if manifest.count(original_path) != 1 or manifest.count(original_units) != 1:
        raise ValueError('measurement harness template differs from its expected shape')
    if source_alias is not None:
        alias = Path(source_alias)
        if (not alias.is_absolute() or str(alias) != str(source_alias)
                or any(part in ('.', '..') for part in str(source_alias).split('/'))):
            raise ValueError('source alias must be an absolute normalized path')
        manifest_rune = alias / 'crates' / 'rune'
    else:
        manifest_rune = rune
    manifest = manifest.replace(original_path, 'path = ' + json.dumps(str(manifest_rune), ensure_ascii=True))
    if profile == 'cgu16':
        manifest = manifest.replace(original_units, 'codegen-units = 16')
    out.mkdir(parents=True, exist_ok=False)
    (out / 'src').mkdir()
    (out / 'Cargo.toml').write_text(manifest)
    shutil.copyfile(HARNESS / 'Cargo.lock', out / 'Cargo.lock')
    shutil.copyfile(HARNESS / 'src' / 'main.rs', out / 'src' / 'main.rs')
    if source_export.verify(source, commit) != before or _harness_inputs() != harness_before:
        raise ValueError('source or harness changed while preparing the recipe')
    files = {}
    for name in ('Cargo.toml', 'Cargo.lock', 'src/main.rs'):
        data = (out / name).read_bytes()
        files[name] = {'sha256': hashlib.sha256(data).hexdigest(), 'bytes': len(data)}
    receipt = {'schema': 1, 'status': 'PASS', 'profile': profile, 'source': str(source),
               'source_alias': str(source_alias) if source_alias is not None else None,
               'commit': commit, 'tree': before['tree'], 'execution_feature': execution_feature,
               'native_feature': native_feature, 'harness_inputs': harness_before,
               'files': files}
    with (out / 'RECIPE.json').open('x', encoding='utf-8') as stream:
        json.dump(receipt, stream, indent=2, sort_keys=True)
        stream.write('\n')
    return receipt
