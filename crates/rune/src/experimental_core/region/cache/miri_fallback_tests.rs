//! The native tier under Miri. `checkout` refuses before it touches the thread's cache, so
//! no environment or page-size query, no analysis, no Cranelift compile and no mapping
//! happens, and the interpreter runs every invocation with its usual results and errors.
//! Outside Miri the same test checks that its program is built and loaded natively, which
//! is what gives the Miri assertions their meaning. Hand-built code only: no Rune source is
//! compiled, so the test stays cheap under Miri.
use super::super::hook;
use super::{cache_state_for_test, clear_for_test, counters_for_test, enable_for_test};
use super::{enabled_decided_for_test, summary};

use crate::experimental_core::{
	arithmetic::Binary,
	code::{Builder, Code, Function, Op},
	error::{ErrorKind, Location},
	execute::Executor,
	heap::{Limits, Store},
	value::{Scalar, Value},
};

fn loc(n: u32) -> Location {
	Location {
		source: 1,
		revision: 1,
		start: n,
		end: n + 1,
	}
}

/// `fn(limit) { i = 0; s = 100; while i < limit { s = s <op> step; i = i + 1 } return s }`
fn counted(op: Binary, step: i64) -> Code {
	// slots: 0 limit/param, 1 i, 2 s, 3 cond, 4 one, 5 step
	let mut f = Function::new(6, 1).unwrap();
	let ops = [
		Op::Literal {
			out: 1,
			value: Value::I64(0),
		},
		Op::Literal {
			out: 2,
			value: Value::I64(100),
		},
		// pc 2: the loop header.
		Op::Binary {
			out: 3,
			op: Binary::Lt,
			lhs: 1,
			rhs: 0,
		},
		Op::Branch {
			condition: 3,
			yes: 4,
			no: 9,
		},
		Op::Literal {
			out: 5,
			value: Value::I64(step),
		},
		Op::Binary {
			out: 2,
			op,
			lhs: 2,
			rhs: 5,
		},
		Op::Literal {
			out: 4,
			value: Value::I64(1),
		},
		Op::Binary {
			out: 1,
			op: Binary::Add,
			lhs: 1,
			rhs: 4,
		},
		Op::Jump { target: 2 },
		Op::Return { value: 2 },
	];

	for (pc, op) in ops.into_iter().enumerate() {
		f.push(op, loc(pc as u32)).unwrap();
	}

	let mut b = Builder::default();
	b.push(f).unwrap();
	b.finish().unwrap()
}

/// One invocation on a new executor and store: the returned scalar or the error kind.
fn run(code: &Code, limit: i64, fuel: usize) -> Result<Option<Scalar>, ErrorKind> {
	let mut store = Store::new(Limits {
		entries: 256,
		bytes: 65_536,
	})
	.unwrap();
	let arg = store.root_i64(limit);
	let mut executor = Executor::new().unwrap();
	let result = match executor.invoke_code(code, &mut store, 0, &[&arg], fuel) {
		Ok(root) => Ok(store.root_scalar(&root).unwrap()),
		Err(failure) => Err(failure.error.kind),
	};
	assert!(executor.is_idle());
	result
}

#[test]
fn native_tier_is_never_compiled_under_miri() {
	let add = counted(Binary::Add, 3);
	let mul = counted(Binary::Mul, 1 << 40);
	hook::uninstall();

	// The interpreter's answers, with the production tier off: a value, fuel exhaustion
	// inside the loop and an arithmetic error inside the loop.
	clear_for_test();
	enable_for_test(false);
	let expected = [run(&add, 10, 1_000), run(&add, 10, 5), run(&mul, 10, 1_000)];
	assert_eq!(expected[0], Ok(Some(Scalar::I64(130))));
	assert_eq!(expected[1], Err(ErrorKind::FuelExhausted));
	assert_eq!(expected[2], Err(ErrorKind::Overflow));

	// The same invocations with the production tier on.
	clear_for_test();
	enable_for_test(true);
	let (checkouts, builds) = counters_for_test();
	let observed = [run(&add, 10, 1_000), run(&add, 10, 5), run(&mul, 10, 1_000)];
	let (checkouts_after, builds_after) = counters_for_test();
	enable_for_test(false);

	assert_eq!(observed, expected);
	assert_eq!(
		checkouts_after - checkouts,
		3,
		"every invocation consults the tier"
	);

	if cfg!(miri) {
		assert_eq!(builds_after - builds, 0, "no analysis and no compile");
		assert!(
			!enabled_decided_for_test(),
			"no environment or page-size query"
		);
		assert_eq!(
			cache_state_for_test(),
			(0, 0, 0, 0),
			"nothing cached or mapped"
		);
		assert!(summary(add.id()).is_none());
	} else {
		assert_eq!(builds_after - builds, 2, "one build per program");
		assert!(enabled_decided_for_test());
		let loaded = summary(add.id()).expect("resident").loaded;
		assert_eq!(
			loaded, 1,
			"the program this test runs is a native candidate"
		);
	}

	clear_for_test();
}
