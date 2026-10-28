"""Construct the filesystem and environment capsule for an offline build."""
from pathlib import Path


def prepare(out, source, harness, toolchain, cargo_cache):
    """Prepare an offline Cargo home and return bwrap arguments and child environment.

    The caller supplies and pins the actual source, harness, toolchain and cache.
    The returned command has fixed logical mount names; it does not choose a host
    or start a build.
    """
    out = Path(out).resolve(strict=True)
    source = Path(source).resolve(strict=True)
    harness = Path(harness).resolve(strict=True)
    toolchain = Path(toolchain).resolve(strict=True)
    cargo_cache = Path(cargo_cache).resolve(strict=True)
    for path in (source, toolchain, cargo_cache):
        if out == path or out.is_relative_to(path) or path.is_relative_to(out):
            raise ValueError('capsule output overlaps a protected input')
    if not harness.is_dir() or not source.is_dir() or not toolchain.is_dir() or not cargo_cache.is_dir():
        raise ValueError('capsule inputs must be directories')
    for name in ('config', 'config.toml'):
        config = harness / '.cargo' / name
        if config.exists() or config.is_symlink():
            raise ValueError('harness contains a Cargo configuration')
    cargo_home = out / 'cargo-home'
    cargo_home.mkdir()
    (out / 'home').mkdir()
    (out / 'target').mkdir()
    (cargo_home / 'config.toml').write_text('[net]\noffline = true\n')
    args = ['/usr/bin/bwrap', '--die-with-parent', '--unshare-net', '--ro-bind', '/', '/',
            '--tmpfs', '/home', '--tmpfs', '/tmp', '--dir', '/tmp/rune-build',
            '--bind', str(out), '/tmp/rune-build/out',
            '--ro-bind', str(source), '/tmp/rune-build/source',
            '--ro-bind', str(harness), '/tmp/rune-build/harness',
            '--ro-bind', str(toolchain), '/tmp/rune-build/toolchain',
            '--proc', '/proc', '--dev', '/dev', '--chdir', '/tmp/rune-build/harness']
    if harness.is_relative_to(out):
        args += ['--ro-bind', str(harness), '/tmp/rune-build/out/' + str(harness.relative_to(out))]
    if Path('/.cargo').exists():
        args += ['--tmpfs', '/.cargo']
    for name in ('registry', 'git'):
        cache = cargo_cache / name
        if cache.is_dir():
            (cargo_home / name).mkdir()
            args += ['--ro-bind', str(cache), '/tmp/rune-build/out/cargo-home/' + name]
    env = {'HOME': '/tmp/rune-build/out/home', 'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8',
           'LC_ALL': 'C.UTF-8', 'TZ': 'UTC', 'CARGO_HOME': '/tmp/rune-build/out/cargo-home',
           'CARGO_TARGET_DIR': '/tmp/rune-build/out/target', 'CARGO_INCREMENTAL': '0',
           'RUSTC': '/tmp/rune-build/toolchain/bin/rustc'}
    return args, env
