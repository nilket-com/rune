"""Rebuild the control bytecode from signed source; never read saved observations."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

import build_capsule
import build_recipe
import cargo_cache
import milestone_registry
import source_export

# This instrumentation is added only to disposable verified exports. It exposes
# no API in the committed Rune source or the measured binary.
_METHOD = r'''
impl Program {
    #[doc(hidden)]
    pub fn __xtask_dump_control_bytecode(&self) {
        std::println!("FUNCTIONS {}", self.code.functions());
        for id in 0..self.code.functions() as u32 {
            let function = self.code.function(id).unwrap();
            std::println!("FUNCTION {id}");
            for (pc, ins) in function.instructions().iter().enumerate() {
                std::println!("{pc:04} raw={ins:?} charged={} {:?}", ins.is_charged(), function.decode(*ins).unwrap());
            }
        }
    }
}
'''
_MAIN = r'''
use rune::experimental_core::frontend;
use rune::{Context, Diagnostics, Source, Sources};
fn main() {
    for name in ["control_is", "control_branch"] {
        let path = format!("/tmp/rune-build/out/{name}.rn");
        let text = std::fs::read_to_string(path).unwrap();
        let context = Context::with_default_modules().unwrap();
        let mut sources = Sources::new();
        sources.insert(Source::memory(&text).unwrap()).unwrap();
        let mut diagnostics = Diagnostics::new();
        let program = frontend::compile(&mut sources, &context, &mut diagnostics, 1).unwrap();
        println!("CONTROL {name}");
        program.__xtask_dump_control_bytecode();
    }
}
'''


def _pin(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def _role(repo, role, kernels, root):
    if role.get('role') not in ('tip', 'candidate') or role.get('profile') != 'cgu1' or role.get('native') is not False:
        raise ValueError('control proof requires the two CGU1 off roles')
    receipt_path = Path(role['build']['path']).resolve(strict=True)
    if _pin(receipt_path) != role['build']['sha256'] or receipt_path.stat().st_size != role['build']['bytes']:
        raise ValueError('control proof build receipt changed')
    receipt = json.loads(receipt_path.read_text())
    commit = role['commit']
    if (receipt.get('status') != 'PASS' or receipt.get('role') != role['role']
            or receipt.get('profile') != 'cgu1' or receipt.get('native') is not False
            or receipt.get('source', {}).get('commit') != commit or receipt.get('binary') != role['binary']):
        raise ValueError('control proof role differs from its build receipt')
    binary = Path(role['binary']['path']).resolve(strict=True)
    if _pin(binary) != role['binary']['sha256'] or binary.stat().st_size != role['binary']['bytes']:
        raise ValueError('control proof measured binary changed')
    source_export.verify_signed(repo, receipt['source']['source'], commit)
    exported = source_export.export(repo, commit, root / 'export')
    source = Path(exported['source'])
    source_export.verify_signed(repo, source, commit)
    run = root / 'run'
    run.mkdir()
    recipe = build_recipe.prepare(source, commit, 'cgu1', run / 'harness', source_alias='/tmp/rune-build/source')
    if recipe['files']['Cargo.lock'] != receipt['recipe']['files']['Cargo.lock']:
        raise ValueError('control proof lock differs from the measured build')
    rustc = next((Path(x['path']) for x in receipt['tool_pins'] if Path(x['path']).name == 'rustc'), None)
    if rustc is None:
        raise ValueError('measured compiler is missing')
    toolchain = rustc.parent.parent.resolve(strict=True)
    cargo = toolchain / 'bin/cargo'
    cargo_pin = next((x for x in receipt['tool_pins'] if Path(x['path']) == cargo), None)
    rustc_pin = next((x for x in receipt['tool_pins'] if Path(x['path']) == rustc), None)
    if not cargo_pin or not rustc_pin or _pin(cargo) != cargo_pin['sha256'] or _pin(rustc) != rustc_pin['sha256']:
        raise ValueError('control proof compiler differs from the measured build')
    cache = Path(receipt['registry']['root']).resolve(strict=True)
    cache_before = cargo_cache.verify(run / 'harness/Cargo.lock', cache)
    frontend = source / 'crates/rune/src/experimental_core/frontend.rs'
    original_frontend = frontend.read_bytes()
    with frontend.open('a', encoding='utf-8') as stream:
        stream.write(_METHOD)
    (run / 'harness/src/main.rs').write_text(_MAIN)
    kernel_pins = {}
    for name in ('control_is', 'control_branch'):
        original = Path(kernels) / (name + '.rn')
        data = original.read_bytes()
        kernel_pins[name] = hashlib.sha256(data).hexdigest()
        (run / (name + '.rn')).write_bytes(data)
    capsule, env = build_capsule.prepare(run, source, run / 'harness', toolchain, cache)
    cmd = capsule + ['--', '/tmp/rune-build/toolchain/bin/cargo', 'build', '--offline', '--release', '--locked', '-j', '2']
    result = subprocess.run(cmd, env=env, stdin=subprocess.DEVNULL, capture_output=True, timeout=1500, check=False)
    if result.returncode:
        raise ValueError('control proof build failed: ' + result.stderr.decode(errors='replace')[-2000:])
    result = subprocess.run(capsule + ['--', '/tmp/rune-build/out/target/release/p2-probe-harness'],
                            env=env, stdin=subprocess.DEVNULL, capture_output=True, timeout=120, check=False)
    if result.returncode:
        raise ValueError('control proof execution failed: ' + result.stderr.decode(errors='replace')[-2000:])
    if not result.stdout.startswith(b'CONTROL control_is\n') or b'CONTROL control_branch\n' not in result.stdout:
        raise ValueError('control proof output is incomplete')
    if cargo_cache.verify(run / 'harness/Cargo.lock', cache) != cache_before:
        raise ValueError('control proof dependency cache changed')
    if any(_pin(Path(kernels) / (name + '.rn')) != value for name, value in kernel_pins.items()):
        raise ValueError('control proof input changed')
    if frontend.read_bytes() != original_frontend + _METHOD.encode():
        raise ValueError('control proof instrumentation changed')
    frontend.write_bytes(original_frontend)
    source_export.verify_signed(repo, source, commit)
    source_export.verify_signed(repo, receipt['source']['source'], commit)
    return result.stdout


def prove(repo, roles, kernels):
    """Recompile both signed roles and compare their current emitted bytecode."""
    repo, kernels = Path(repo).resolve(strict=True), Path(kernels).resolve(strict=True)
    tip, candidate = roles['tip-cgu1-off'], roles['candidate-cgu1-off']
    registry = milestone_registry.load()
    baseline = [item['commit'] for item in registry['milestones'] if item['id'] == 'M7']
    if len(baseline) != 1 or tip['commit'] != baseline[0]:
        raise ValueError('S8 qualification requires the committed M7 baseline')
    before = {}
    for label, role in (('tip', tip), ('candidate', candidate)):
        receipt = json.loads(Path(role['build']['path']).read_text())
        source_export.verify_signed(repo, receipt['source']['source'], role['commit'])
        source = Path(receipt['source']['source']) / 'crates/rune/src/experimental_core'
        before[label] = {name: (source / name).read_bytes() for name in ('heap/indexing.rs', 'execute.rs')}
    if before['tip'] != before['candidate']:
        raise ValueError('qualified indexing or invocation source differs between signed commits')
    with tempfile.TemporaryDirectory(prefix='rune-control-proof-') as temp:
        root = Path(temp)
        tip_bytes = _role(repo, tip, kernels, root / 'tip')
        candidate_bytes = _role(repo, candidate, kernels, root / 'candidate')
    if tip_bytes != candidate_bytes:
        raise ValueError('qualified control bytecode differs between live compilers')
    for label, role in (('tip', tip), ('candidate', candidate)):
        receipt = json.loads(Path(role['build']['path']).read_text())
        source_export.verify_signed(repo, receipt['source']['source'], role['commit'])
        source = Path(receipt['source']['source']) / 'crates/rune/src/experimental_core'
        if any((source / name).read_bytes() != data for name, data in before[label].items()):
            raise ValueError('qualified source changed during proof')
    return {'schema': 1, 'tip_commit': tip['commit'], 'candidate_commit': candidate['commit'],
            'control_bytecode_equal': True, 'indexing_source_equal': True, 'entry_source_equal': True,
            'control_bytecode_sha256': hashlib.sha256(tip_bytes).hexdigest()}
