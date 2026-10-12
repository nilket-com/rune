//! Owner-free construction checks over the crate-private store and executor API.
//! Hosts never see these routes; the sealed public boundary is tested by the
//! `fresh_core_*` integration tests.
//!
//! # Miri
//!
//! Every fresh-core test runs natively (`cfg(miri)` is false there). Under Miri, tests that
//! build a std-module `Context` and compile Rune source, the differential tier and corpus,
//! and the `fresh_core_stdout` process tests are marked `#[cfg_attr(miri, ignore = "...")]`,
//! one fixed reason per class. The default `cargo miri test` therefore runs the pure store,
//! hand-built-code and executor tests, plus the two empty-`Context` tests
//! (`frontend::a_custom_macro_at_the_builtin_path_is_not_trusted`,
//! `io_policy::custom_first_then_builtin_conflicts_and_stays_uncertified`), each measured
//! once at about 11 s.
//!
//! Opt-in commands for the ignored tests (toolchain pinned as in the validation runs):
//!
//! ```text
//! # the 20-row differential tier (one measured run: about 20 min)
//! cargo +nightly-2026-09-01 miri test --locked -p rune --all-features --lib -- --ignored --exact experimental_core::tests::differential::differential_miri_tier --test-threads=1
//! # one module's ignored tests
//! cargo +nightly-2026-09-01 miri test --locked -p rune --all-features --lib -- --ignored experimental_core::tests::semantics --test-threads=1
//! # one test
//! cargo +nightly-2026-09-01 miri test --locked -p rune --all-features --lib -- --ignored --exact experimental_core::tests::io_policy::capture_is_refused --test-threads=1
//! # integration targets: these opt-ins are UNPROVEN under Miri (never run there)
//! cargo +nightly-2026-09-01 miri test --locked -p rune --all-features --test fresh_core_api -- --ignored --test-threads=1
//! cargo +nightly-2026-09-01 miri test --locked -p rune --all-features --test fresh_core_boundary -- --ignored --test-threads=1
//! cargo +nightly-2026-09-01 miri test --locked -p rune --all-features --test fresh_core_repeat_entry -- --ignored --test-threads=1
//! ```
//!
//! The `fresh_core_stdout` tests have no Miri opt-in: they spawn the test binary, which Miri
//! cannot do; they run natively only.
//!
//! Many ignored tests take several minutes to tens of minutes each under Miri, and some
//! exceeded a 30-minute per-test budget in the one-time validation runs. Whole-suite Miri
//! times quoted for the ignored set (on the order of 13 hours) are projections, not
//! measurements. A one-time deep run is not continuous coverage: only the default
//! selection runs on every Miri CI pass.

mod allocation_path;
mod boundary_internal;
mod diagnostics;
mod differential;
mod dispatch_state;
mod executor;
mod formatting;
mod frontend;
mod io_policy;
mod lowering_shape;
mod macro_certification;
mod predecode;
mod semantics;
mod store_integrity;
mod strict_conditions;
mod struct_names;
mod values;
