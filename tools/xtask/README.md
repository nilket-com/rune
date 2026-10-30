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

The verifier runs on the machine where it is invoked. Pool defaults come from
`std::thread::available_parallelism()`: native jobs, Cargo jobs and native test
threads share one CPU budget/jobserver. Miri uses the reviewed general memory
policy `max(1, N / 4)`. Explicit `--workers`, `--jobs` and `--test-threads` are
upper bounds. A native library uses the available reserved slots as its actual
libtest thread count, so it can start alongside Miri without waiting for all
interpreters to finish. Explicit exclusive cases keep their full reservation.

Independent jobs run longest estimated first. The local job list splits the
98 Miri rows (the same 78 parent witnesses), contract cases/bundle, native
libraries/docs and all 60 AWFY observations. Failure/reuse sequences remain
inside their original row. Each Miri parent shares its original deadline,
starting only after its first CPU admission, and its rows use the same local
prepared state. Builds coalesce by complete inputs within this invocation.
`TASKS.json` and per-job receipts identify exactly what ran. Suite validators
still require all original selected/pass counts, comparisons and trace proof.
Miri overlaps native work; there is no first-pair or two-interpreter barrier.

Havlak-2 has an approved 1200-second CPU hang guard and a 3600-second wall
cleanup bound on every engine/control observation. Standalone measured CPU
costs were 250.94 seconds legacy and 257.53 fresh. These are hang guards, not
performance assertions. Other AWFY bounds and all comparisons are unchanged.
The large whole-machine AWFY isolation cohort has been removed.

`--cold` records explicit cold execution. There is no semantic pass cache:
every required job executes, and each output directory is new. Acceptance uses
one fresh clone with empty project targets and reports its wall time; there is
no under-30-minute gate. Cache work is deferred. Launch supervision, memory
ceilings and exit notifications belong to operator tooling outside Rune.
The runner blocks on child waits and condition variables, without progress
polling or progress watchers.

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
