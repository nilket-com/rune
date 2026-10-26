# Rune correctness verification

`cargo xtask list` lists the named suites. Run suites with a new output directory outside the source tree:

```sh
cargo xtask verify --suite native,contracts --fixtures-root /path/to/.reviews \
  --out /path/to/new-result --jobs 8 --test-threads 4 --workers 2
```

Use `--suite all` for every suite. `--check-manifests` checks the selected suites'
pinned inputs without compiling Rune. The default Rust toolchain is `1.98.1`;
Miri uses the nightly pinned in its manifest. `--repo` selects the Git checkout
whose source will be built. The runner includes ignored `Cargo.lock` in its source
identity. Prepare the lockfile before starting a verification run; builds use
`--locked`. Keep that checkout unchanged until completion.

Each child has immutable stdout, stderr and a JSON process receipt with arguments,
working directory, effective environment, hashes, exit status and elapsed time.
`SOURCE.json` pins the source files and revision; build receipts pin copied test
executables and verify the Rune compilation units' source directories in DWARF.
Suite and overall `RESULT.json` files report acceptance or retain the failure.
A failed output directory is evidence; use a new directory for another attempt.

`--workers` bounds independent suites. Its product with the larger of `--jobs`
and `--test-threads` must fit the process's available CPUs, capped at 30. Miri
starts only after all selected native suites finish and runs at most two actual
interpreters. The runner blocks on process completion; it does not poll process
status. Existing host scheduling and Nano measurement locks still apply.

Native libraries, AWFY, Clippy, layouts and Miri run from the repository alone;
`--fixtures-root` is optional for those suites. Their small frozen inputs are
byte-identical repository copies. Absolute names retained in historical
observations are logical source names, never filesystem read paths.

Contracts require the frozen Rnx corpus (65 files, 158,864 bytes). Each preservation
suite currently validates the shared corpus (12,207 files, 23,011,816 bytes).
`fixtures/EXTERNAL.json` records per-suite requirements and the fixture bundle's
SHA256. Obtain `0185-v1a-fixtures-v1.tar.gz` from the S7b2/V1a record artifacts
(`0185-v1a-fixture-bundle1`) through the record maintainer, verify its digest,
unpack it into a directory, and pass that directory as `--fixtures-root`.
The bundle contains all 12,272 required external files with unchanged bytes and
a per-file digest index. The named V1d followup, portable frozen correctness
inputs, owns content-addressed distribution and review of the remaining small
corpus data; Rnx ownership remains coordinated with V1b.

The manifests version selectors, counts, policies and frozen input hashes.
Historical controllers and receipts remain available for comparison. V1a covers
correctness only; Rnx verification and gate orchestration belong to V1b and V1c.
The outer host gate must exclude other native jobs throughout Miri: V1a checks
its own process counts and a launch snapshot, while V1c owns host scheduling.
Cargo's user configuration remains active for correctness builds. V1c measurement
build inputs must isolate user Cargo configuration, including the host linker.
