//! Operator-only construction micro; compile the same fixture against both subjects.

use super::{Nominal, Shape, Staging, PROVISIONAL};
use crate::experimental_core::heap::{Limits, Store};
use crate::experimental_core::value::{Type, Value};

#[inline(never)]
fn construct(store: &mut Store, nominal: Nominal, value: i64) {
	let value = store
		.list(Type::Struct(nominal), &[Value::I64(value)])
		.unwrap();
	store.release(std::hint::black_box(value)).unwrap();
}

#[test]
#[ignore = "operator-only multi-thread construction measurement"]
fn construction_throughput() {
	let threads: usize = std::env::var("N1_THREADS").unwrap().parse().unwrap();
	let iterations: usize = std::env::var("N1_ITERATIONS").unwrap().parse().unwrap();
	assert!([1, 8, 32].contains(&threads));
	assert!(iterations > 0 && iterations <= 1_000_000);
	let mut staging = Staging::default();
	let nominal = staging
		.stage_shape(
			0x004e_314d_4943_524f,
			format_args!("N1Micro::T"),
			Shape::Tuple(1),
		)
		.unwrap();
	let mut remap = staging.prepare_remap().unwrap();
	staging.commit(&mut remap).unwrap();
	let nominal = if nominal.0 & PROVISIONAL == 0 {
		nominal
	} else {
		remap[(nominal.0 & !PROVISIONAL) as usize]
	};
	let start = std::sync::Barrier::new(threads + 1);
	let finish = std::sync::Barrier::new(threads + 1);
	let seconds = std::thread::scope(|scope| {
		for _ in 0..threads {
			let start = &start;
			let finish = &finish;
			scope.spawn(move || {
				let mut store = Store::new(Limits::default()).unwrap();
				construct(&mut store, nominal, 0);
				start.wait();
				for i in 0..iterations {
					construct(&mut store, nominal, std::hint::black_box(i as i64));
				}
				finish.wait();
			});
		}
		start.wait();
		let now = std::time::Instant::now();
		finish.wait();
		now.elapsed().as_secs_f64()
	});
	std::println!(
		"N1_MICRO {}",
		serde_json::json!({"threads":threads,"iterations_per_thread":iterations,"operations":threads*iterations,"seconds":seconds,"operations_per_second":threads as f64*iterations as f64/seconds})
	);
}
