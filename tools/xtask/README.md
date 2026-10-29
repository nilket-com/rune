# Rune correctness verification

`cargo xtask list` lists the named suites. Output defaults to a new scratch directory under `target/xtask-verify`; an explicit output may also be outside the source tree. For example:

```sh
cargo xtask verify --suite native,contracts
```

Use `--suite all` for every suite. `--check-manifests` checks the selected suites'
pinned inputs without compiling Rune. The default Rust toolchain is `1.98.1`;
Miri uses the nightly pinned in its manifest. `--repo` selects the Git checkout
whose source will be built. The runner includes `Cargo.lock` in its source
identity. Builds use
`--locked`. Keep that checkout unchanged until completion.

The `measure-checks` suite runs synthetic controls for the measurement helper code. It does not collect measurements or read historical results.

Each child has immutable stdout, stderr and a JSON process receipt with arguments,
working directory, effective environment, hashes, exit status and elapsed time.
`SOURCE.json` pins the source files and revision; build receipts pin copied test
executables and verify the Rune compilation units' source directories in DWARF.
Suite and overall `RESULT.json` files report acceptance or retain the failure.
A failed output directory is evidence; use a new directory for another attempt.

`--workers` bounds independent suites. Its product with the larger of `--jobs`
and `--test-threads` must fit the process's available CPUs. Miri
starts only after all selected native suites finish and runs at most two actual
interpreters. The runner blocks on process completion; it does not poll process
status. Existing host scheduling and measurement locks still apply.

The L1 suite compares the checked-in programs on both engines, with native
code disabled and enabled. Both children
use the default Rune context, without filesystem or network
modules; the suite compares captured streams, return observations and full compile
warning multisets for admitted programs. Warning order is ignored; multiplicity,
source identity, spans, messages and auxiliary warning context are preserved. Runtime
errors whose typed details are not captured on either side are accepted only
with an exact message, and the report lists each such case explicitly.
An additional isolated work audit uses the execution core's permissive script
formatting policy. For each legacy completion it requires execution-core
completion with no greater work charge. A legacy budget halt followed by a
new completion is counted as `reaches-further/success`. Compile-only inputs and
refusals are reported separately and do not count as execution coverage.
Historical compiler emission byte checks and serialized Unit snapshots have been retired:
they detected changes in stored output rather than comparing behavior. The
corresponding L1 inputs remain in the live corpus. AWFY compares its benchmark
programs live and checks its declared refusals and resource limits. Complete self-contained verification also requires an isolated fresh-clone run
of the full suite.

An optional `--hosts PATH` selects a UTF-8 JSON host description; absent that
option the runner reads `rune-verify/hosts.json` under `XDG_CONFIG_HOME`, or
under the user's configuration directory if `XDG_CONFIG_HOME` is unset. If no
file exists, verification uses local process resources. Configuration uses
`schema: 1` and optional `correctness` and `measurement` objects. Each object
accepts only `endpoint` (SSH destination), `cpus` (distinct CPU numbers),
`memory_bytes`, `measurement_cpu`, `measurement_lock`, `measurement_max_load`,
`measurement_min_available_bytes`, `replay_root`,
`rustc_identity`, `linker_identity`, `valgrind_path`, `valgrind_root`, and `valgrind_identity`. Paths must be absolute. These fields
describe the host; the verification runner currently records the description
and its hash but does not use it to launch remote jobs or reserve resources.
Treat configuration receipts as sensitive if the local description contains
private information.

E1's isolated budget-edge probes cover `reaches-further/success`,
`reaches-further/error`, `budget-annotation`, and `both-halt/further` with
live larger-allowance legacy controls. They retain raw streams and ordered
host effects and require progress prefixes in both directions. Primary
errors and complete natural-outcome output must match. User-facing halt
wording, location policy and exit status belong to the real CLI routing
acceptance; these Rune probes observe typed outcomes and program streams.
