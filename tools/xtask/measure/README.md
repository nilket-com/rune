# Measurement inputs

`kernels/` contains the source programs shared by the native admission tests and the measurement harness. `gen_inputs.py` creates input streams and independent model outputs in a new scratch directory:

```sh
python3 tools/xtask/measure/gen_inputs.py /absolute/new-output 12345
```

The optional decimal seed is recorded in `MANIFEST.json`; when omitted, the generator selects and records a random seed. Generated inputs, model outputs and manifests are run artifacts and do not belong in Git. The xtask measurement controller is still being migrated.

`protocol.py` builds the fixed regression-screen schedule from a seed and the generated input manifest. Its controls can be run with `python3 -I -B tools/xtask/measure/test_protocol.py`.

`bootstrap.py` implements joint resampling of the current measurement rounds. Run its controls with `python3 -I -B tools/xtask/measure/test_bootstrap.py`.

`callgrind.py` parses a supplied instruction-only profile with instruction-and-line or line-only positions, checks that self costs sum to the reported total, and computes symbol projections and core differences. Run its controls with `python3 -I -B tools/xtask/measure/test_callgrind.py`.

`callgrind_rules.py` checks the structural rules ported so far, including the context-install residue rule across a complete five-kernel phase and four-profile controls. `callgrind_guard.py` derives the iterator exception from fresh source, binaries and profiles. Run their controls with `python3 -I -B tools/xtask/measure/test_callgrind_rules.py` and `python3 -I -B tools/xtask/measure/test_callgrind_guard.py`.

`perf_parse.py` parses supplied counter text using an explicit event table and group, retaining unavailable and multiplexed events as such. Run its controls with `python3 -I -B tools/xtask/measure/test_perf_parse.py`.

`k1.py` calculates K1 medians and exact ratios from supplied fresh samples. Run its controls with `python3 -I -B tools/xtask/measure/test_k1.py`.

`counter_assess.py` applies the five-repeat instruction rule and cycle soft-flag rule to supplied current measurements. Run its controls with `python3 -I -B tools/xtask/measure/test_counter_assess.py`.

`counter_schedule.py` constructs the seeded counter2 workload and admission order from the generated input manifest. Run its controls with `python3 -I -B tools/xtask/measure/test_counter_schedule.py`.

`measurement_runtime.py` supplies local locking, child CPU affinity and bounded subprocess execution for future controllers. `measurement_readiness.py` checks host load, available memory and competing processes against declared limits and records the thermal throttle counter; the controller must still check the protocol's before-and-after observations. Run their controls with `python3 -I -B tools/xtask/measure/test_measurement_runtime.py` and `python3 -I -B tools/xtask/measure/test_measurement_readiness.py`.

`source_export.py` exports a requested Git commit into a new directory and verifies its files against the commit; its signed reconciliation checks the configured Git trust store and compares either an export or a standalone snapshot and its declared extra inputs with the signed tree. Run its controls with `python3 -I -B tools/xtask/measure/test_source_export.py`.

`build_recipe.py` prepares the CGU1 or CGU16 harness against a verified source export; it does not yet run the isolated build. Run its controls with `python3 -I -B tools/xtask/measure/test_build_recipe.py`.

`build_capsule.py` prepares the offline build namespace and isolated Cargo environment. Run its controls with `python3 -I -B tools/xtask/measure/test_build_capsule.py`.

`cargo xtask build --help` describes the local commit-based build command. It requires a configured measurement CPU, lock, compiler identity and linker identity. Local CGU1 and CGU16 release builds have been validated with registry-cache checks. Remote execution is not supported. The worker's synthetic controls run with `python3 -I -B tools/xtask/measure/test_build_worker.py`.

`cargo_cache.py` checks the locked crate archives and their extracted registry source before and after a build. Its controls run with `python3 -I -B tools/xtask/measure/test_cargo_cache.py`.

`role_evidence.py` checks a role against a commit-based build, including the source tree, binary, compiler and linker pins, actual execution traces and candidate signature. It is a seal component and does not by itself establish a complete sealed measurement. Run its controls with `python3 -I -B tools/xtask/measure/test_role_evidence.py`.

`cargo xtask seal --help` describes creating and checking a local fresh-capture seal. The seal specification has exactly `schema: 1`, `capture` (the absolute `RESULT.json` path), `signing_repo`, `commit` (a full signed candidate commit), `native_source` and `ignored_inputs`. The worker rechecks the snapshot, build roles, controller inputs and analyzer closures and recalculates decisions from raw profiles rather than saved analysis values. Remote receipts are not supported. Run its synthetic controls with `python3 -I -B tools/xtask/measure/test_seal_evidence.py` and `python3 -I -B tools/xtask/measure/test_seal_store.py`.

`input_closure.py` pins and rechecks explicitly declared files; it is used by the analyzer sandbox and seal. Run its controls with `python3 -I -B tools/xtask/measure/test_input_closure.py`.

`analysis_sandbox.py` runs a pinned analyzer with declared package and code files, the pinned Python standard library, and explicitly pinned runtime binaries and shared libraries. It does not mount the rest of `/usr`. `analysis_runtime.py` inventories that runtime and rechecks it around analysis. The local protocol controllers and seals use these closures. Run their controls with `python3 -I -B tools/xtask/measure/test_analysis_sandbox.py` and `python3 -I -B tools/xtask/measure/test_analysis_runtime.py`.

`callgrind_cell.py`, `counter_cell.py`, and `k1_analysis.py` analyze fresh raw profiles within that boundary. Their sandbox controls cover symbol-map, path-count and milestone branches, including missing selected inputs. They are cell-level building blocks used by the local measurement controllers and seals.

`callgrind_packet.py` analyzes the CGU1 controls and both five-kernel phases with their reference profiles. `callgrind_trend.py` computes CGU16 trend ratios from fresh tip, candidate and milestone profiles. The controller binds captured profiles to their build roles. Run their sandbox controls with `python3 -I -B tools/xtask/measure/test_callgrind_packet.py` and `python3 -I -B tools/xtask/measure/test_callgrind_trend.py`.

`analysis_package.py` creates and rechecks the declarations for those analysis building blocks before running them. It does not bind profiles to measurement receipts or build roles and is not a complete measurement seal. Run its controls with `python3 -I -B tools/xtask/measure/test_analysis_package.py`.

`counter_packet.py` independently checks the 191-record counter2 packet, reparsing its raw profiles, schedule, admissions, checksums and environment evidence. Its synthetic complete-packet and optional-path controls run with `python3 -I -B tools/xtask/measure/test_counter_packet.py`. The seal also checks binding to the build roles.

The optional S8 Callgrind qualification is explicit per-record input: copy `qualifications/s8.json` into the request as `qualifications` and fill its `candidate_commit` with the full measured candidate commit; set the request `record` and `baseline_milestone` to match. Qualifications are per-record, never inherited or reused for a later record; each new qualification requires its own review. The verifier rebuilds the M7 and candidate control programs offline from signed commits and locked dependencies, compares their current emitted instructions, and compares the invocation and indexing source files directly. The emitted bytecode digest is evidence output, never a saved expected value. It only accepts the two named, bounded S8 findings; missing proof, other core-symbol control drift, positive core or whole control deltas, and larger or other-cell index growth fail closed. The seal repeats the live proof. Callgrind records `CAPTURE-CONTEXT.json` before analysis, preserving readiness, the capture receipt and analysis manifests even when analysis fails.

`cargo xtask measure --help` describes local counter2 capture. The request is JSON with exactly `schema: 1`, `roles`, `inputs`, hexadecimal `seed` and `path_counts` (a path or null). The six role names are `base-off`, `tip-off`, `candidate-off`, `base-on`, `tip-on` and `candidate-on`; each role specifies `name`, `role`, `profile`, `native`, `commit`, and `build` and `binary` file pins (`path`, `sha256`, `bytes`). It checks those fresh builds, regenerates the input manifest from its seed and model, captures under the configured measurement lock, and reparses in the analyzer sandbox. Local captures can be sealed and rechecked; remote execution is not supported. Its synthetic end-to-end controls run with `python3 -I -B tools/xtask/measure/test_counter_controller.py`; seed regeneration controls run with `python3 -I -B tools/xtask/measure/test_input_verification.py`.

For local `k1-v1`, the request has exactly `schema: 1`, `profile` (`cgu1` or `cgu16`) and `roles`. Candidate and tip on/off builds are required; milestone on/off builds can be added together. The worker captures five interleaved fresh samples per role, and the sandbox reparses the raw files and checks child receipts and expected output. Its controls run with `python3 -I -B tools/xtask/measure/test_k1_controller.py` and `python3 -I -B tools/xtask/measure/test_k1_packet.py`. Local captures can be sealed and rechecked.

For local `callgrind-v1`, the request has `schema: 1`, `roles`, `inputs` and `symbol_map` (a path or null), plus an optional reviewed `qualifications` object and its `record` and `baseline_milestone` bindings. It requires on/off `base`, `tip` and `candidate` builds for CGU1, and on/off `milestone`, `tip` and `candidate` builds for CGU16. Names take the form `candidate-cgu1-off`. The worker captures 68 profiles, including freshly rebuilt reference roles, checks their execution receipts against roles and raw files, and runs both analyzers under the input sandbox. The host configuration must declare `valgrind_path`, `valgrind_root` and `valgrind_identity`; the installation tree and measured binaries' dynamic dependencies are pinned. Local captures can be sealed and rechecked. Run its synthetic end-to-end controls with `python3 -I -B tools/xtask/measure/test_callgrind_controller.py`.

`milestones.json` records only committed milestone source identities; M7 is the current anchor. Historical measurement outputs are not inputs to the registry.

`cargo xtask record --help` describes the local ledger command. Its JSON specification contains exactly `schema: 1`, `ledger` (an absolute path, SHA-256 and byte count), and `expected_entries`. For `append` it also contains `entry`, an optional `milestone` (or null), and five `seals` containing `spec` and `seal` paths. The entry's `evidence` must reproduce the five recomputed seal assessments and coverage exactly. The worker checks candidate, tip, milestone and protocol coverage and revalidates the seals while holding the configured measurement lock before atomically replacing the ledger. This interface and its controls have not yet been validated with real measurement captures. Run the controls with `python3 -I -B tools/xtask/measure/test_record_store.py`, `test_record_policy.py`, `test_ledger_transition.py` and `test_ledger_store.py` (under `tools/xtask/measure/`).

`tool_identity.py` checks ELF comments and traced compiler and linker invocations against caller-supplied identities, and inventories compiler and target libraries. Run its controls with `python3 -I -B tools/xtask/measure/test_tool_identity.py`.

The historical S7b2 replay was used as a one-time migration check. Fresh measurement baselines must be rebuilt and measured from their source commits.
