//! Owner-free construction checks over the crate-private store and executor API.
//! Hosts never see these routes; the sealed public boundary is tested by the
//! `core_*` integration tests.
//!
//! # Miri
//!
//! Every execution test runs natively (`cfg(miri)` is false there). Under Miri, tests that
//! build a std-module `Context` and compile Rune source, the differential tier and corpus,
//! and the `core_stdout` process tests are marked `#[cfg_attr(miri, ignore = "...")]`,
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
//! cargo +nightly-2026-09-01 miri test --locked -p rune --all-features --test core_api -- --ignored --test-threads=1
//! cargo +nightly-2026-09-01 miri test --locked -p rune --all-features --test core_boundary -- --ignored --test-threads=1
//! cargo +nightly-2026-09-01 miri test --locked -p rune --all-features --test core_repeat_entry -- --ignored --test-threads=1
//! ```
//!
//! The `core_stdout` tests have no Miri opt-in: they spawn the test binary, which Miri
//! cannot do; they run natively only.
//!
//! Many ignored tests take several minutes to tens of minutes each under Miri, and some
//! exceeded a 30-minute per-test budget in the one-time validation runs. Whole-suite Miri
//! times quoted for the ignored set (on the order of 13 hours) are projections, not
//! measurements. A one-time deep run is not continuous coverage: only the default
//! selection runs on every Miri CI pass.

mod allocation_path;
mod boundary_internal;
#[cfg(feature = "std")]
mod compile_diagnostics;
mod diagnostics;
mod differential;
mod dispatch_state;
mod embedding_boundary;
mod executor;
mod formatting;
mod frontend;
#[cfg(all(feature = "std", not(miri)))]
mod gap_hunt;
mod io_policy;
mod lowering_shape;
mod macro_certification;
#[cfg(feature = "std")]
mod miri_fixtures;
mod patterns_safety;
mod predecode;
mod semantics;
mod store_integrity;
mod strict_conditions;
mod struct_names;
mod values;

// The default-context corpus is excluded under Miri, but the pure pinned
// diagnostic policy is eligible and uses the same helper and frozen fixture.
#[cfg(miri)]
mod deferred_index_pins {
	use serde_json::{json, Value as Json};

	fn hex(bytes: &[u8]) -> std::string::String {
		bytes.iter().map(|b| std::format!("{b:02x}")).collect()
	}

	include!("gap_hunt/deferred_index/retained.rs");
}

// Exercise the exact C8 policy helper under Miri as well as native tests.
#[cfg(miri)]
mod tuple_let_pins {
	use serde_json::{json, Value as Json};
	use std::vec::Vec;
	include!("gap_hunt/tuple_let/policy.rs");
	include!("gap_hunt/tuple_let/retained.rs");
}

#[cfg(miri)]
mod cmp_ordering_pins {
	use serde_json::{json, Value as Json};
	use std::{string::String, vec::Vec};

	fn hex(bytes: &[u8]) -> String {
		bytes.iter().map(|b| std::format!("{b:02x}")).collect()
	}
	use crate::{Context, Diagnostics, Hash, Source, Sources};
	use std::{format, string::ToString};
	const FUEL: usize = 2_000_000;
	include!("gap_hunt/runtime_error_class.rs");
	include!("gap_hunt/cmp_ordering/fixture.rs");
	include!("gap_hunt/cmp_ordering/stock_observer.rs");
	include!("gap_hunt/cmp_ordering/control_stock.rs");
	mod d1b_diagnostics {
		include!("gap_hunt/d1b_diagnostics.rs");
	}
	include!("gap_hunt/cmp_ordering/p1.rs");
	include!("gap_hunt/cmp_ordering/pins.rs");
	include!("gap_hunt/cmp_ordering/p1_controls.rs");
	include!("gap_hunt/cmp_ordering/retained.rs");
	include!("gap_hunt/cmp_ordering/site_enforce.rs");
}

mod name_projection;
