//! Frozen stock observations before named-field receiver proofs.
use super::*;

fn contract() -> Json {
	serde_json::from_str(include_str!("field_receiver_contract.json")).unwrap()
}

#[test]
#[ignore = "operator-only pre-implementation field freeze; requires new C18F_ORACLE_OUT"]
fn field_receiver_freeze_oracle() {
	use std::io::Write;
	let mut data = contract();
	for row in data["rows"].as_array_mut().unwrap() {
		row["stock"] = stock(row);
		row["before"] = baseline(row);
	}
	let mut out = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(std::env::var_os("C18F_ORACLE_OUT").unwrap())
		.unwrap();
	out.write_all(serde_json::to_string_pretty(&data).unwrap().as_bytes())
		.unwrap();
}

#[test]
fn field_receiver_stock_is_frozen() {
	let data = contract();
	for original in data["rows"].as_array().unwrap() {
		assert_eq!(
			digest(original["source"].as_str().unwrap().as_bytes()),
			original["source_sha256"]
		);
		let mut row = original.clone();
		let actual = stock(&mut row);
		let mut expected = original["stock"].clone();
		if !cfg!(feature = "serde") {
			expected["unit_sha256"] = Json::Null;
		}
		assert_eq!(actual, expected, "{}", original["name"]);
	}
}

#[test]
fn field_receiver_existing_admissions_are_frozen() {
	for row in contract()["rows"].as_array().unwrap() {
		if row["before"]["admitted"] == true {
			assert_eq!(baseline(row), row["before"], "{}", row["name"]);
		}
	}
}

#[test]
#[ignore = "operator-only synthetic-self provenance freeze"]
fn field_receiver_freeze_self_oracle() {
	use std::io::Write;
	let mut data: Json =
		serde_json::from_str(include_str!("field_receiver_self_contract.json")).unwrap();
	for row in data["rows"].as_array_mut().unwrap() {
		row["stock"] = stock(row);
		row["before"] = baseline(row);
	}
	let mut out = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(std::env::var_os("C18F_ORACLE_OUT").unwrap())
		.unwrap();
	out.write_all(serde_json::to_string_pretty(&data).unwrap().as_bytes())
		.unwrap();
}

#[test]
fn field_receiver_self_stock_is_frozen() {
	let data: Json =
		serde_json::from_str(include_str!("field_receiver_self_contract.json")).unwrap();
	for original in data["rows"].as_array().unwrap() {
		assert_eq!(
			digest(original["source"].as_str().unwrap().as_bytes()),
			original["source_sha256"]
		);
		let mut row = original.clone();
		let actual = stock(&mut row);
		let mut expected = original["stock"].clone();
		if !cfg!(feature = "serde") {
			expected["unit_sha256"] = Json::Null;
		}
		assert_eq!(actual, expected, "{}", original["name"]);
	}
}

fn contracts() -> [Json; 3] {
	[
		contract(),
		serde_json::from_str(include_str!("field_receiver_self_contract.json")).unwrap(),
		serde_json::from_str(include_str!("field_receiver_group_contract.json")).unwrap(),
	]
}

#[test]
fn field_receiver_frontier_and_execution() {
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
		value::Value,
	};
	for data in contracts() {
		for row in data["rows"].as_array().unwrap() {
			let observed = baseline(row);
			if row["before"]["admitted"] == true {
				assert_eq!(observed, row["before"], "{}", row["name"]);
			}
			if row["after_admitted"] != true {
				assert_eq!(observed, row["before"], "{}", row["name"]);
				continue;
			}
			assert_eq!(observed["admitted"], true, "{}: {}", row["name"], observed);
			let (program, _) = fresh(row);
			let main = program.function(Hash::type_hash(["main"])).unwrap();
			assert!(program.entry_eligible(main), "{}", row["name"]);
			let mut executor = Executor::new().unwrap();
			let mut store = Store::new(Limits::default()).unwrap();
			let result = executor
				.invoke(&program, &mut store, main, &[], 10000)
				.unwrap();
			assert!(
				matches!(store.import(&result).unwrap(), Value::I64(7)),
				"{}",
				row["name"]
			);
			store.root_release(result).unwrap();
			assert_eq!(
				(store.live_entries(), store.payload_bytes()),
				(0, 0),
				"{}",
				row["name"]
			);
		}
	}
}

#[test]
fn field_receiver_host_entries_refuse_before_roots_fuel_and_cancel() {
	use crate::experimental_core::{
		error::ErrorKind,
		execute::Executor,
		heap::{Limits, Store},
	};
	for data in contracts() {
		for row in data["rows"]
			.as_array()
			.unwrap()
			.iter()
			.filter(|r| r["host_policy"].is_string())
		{
			let (program, _) = fresh(row);
			let target = program
				.function(Hash::type_hash([row["host_policy"].as_str().unwrap()]))
				.unwrap();
			assert!(!program.entry_eligible(target), "{}", row["name"]);
			let main = program.function(Hash::type_hash(["main"])).unwrap();
			let mut executor = Executor::new().unwrap();
			let mut store = Store::new(Limits::default()).unwrap();
			let wrong = store.root_i64(11);
			let foreign_store = Store::new(Limits::default()).unwrap();
			let foreign = foreign_store.root_i64(1);
			let before = (store.live_entries(), store.payload_bytes());
			executor.request_cancel();
			for fuel in [0, 1000] {
				for args in [
					std::vec![],
					std::vec![&wrong],
					std::vec![&foreign],
					std::vec![&wrong, &wrong],
				] {
					let error = executor
						.invoke(&program, &mut store, target, &args, fuel)
						.unwrap_err();
					assert_eq!(error.error.refusal, Some(Refusal::Entry), "{}", row["name"]);
					assert!(
						error.call_chain.is_empty()
							&& error.cleanup_error.is_none()
							&& executor.is_idle()
					);
					assert_eq!((store.live_entries(), store.payload_bytes()), before);
				}
			}
			let error = executor
				.invoke(&program, &mut store, main, &[], 1000)
				.unwrap_err();
			assert_eq!(error.error.kind, ErrorKind::Cancelled);
			let result = executor
				.invoke(&program, &mut store, main, &[], 1000)
				.unwrap();
			store.root_release(result).unwrap();
			store.root_release(wrong).unwrap();
			assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
		}
	}
}

#[test]
#[ignore = "operator-only field frontier report"]
fn field_receiver_observe_frontier() {
	for data in contracts() {
		for row in data["rows"].as_array().unwrap() {
			std::println!(
				"C18F_FRONTIER {}",
				json!({"name":row["name"],"after":baseline(row)})
			);
		}
	}
}

#[test]
fn field_receiver_host_mutation_cannot_reenter_a_contents_proof() {
	use crate::experimental_core::{
		execute::Executor,
		heap::{Limits, Store},
		value::{Type, Value},
	};
	let data = contract();
	let row = data["rows"]
		.as_array()
		.unwrap()
		.iter()
		.find(|r| r["name"] == "returned-struct-reentry")
		.unwrap();
	let (program, _) = fresh(row);
	let mut executor = Executor::new().unwrap();
	let mut store = Store::new(Limits::default()).unwrap();
	let make = program.function(Hash::type_hash(["make"])).unwrap();
	let take = program.function(Hash::type_hash(["take"])).unwrap();
	let main = program.function(Hash::type_hash(["main"])).unwrap();
	assert!(program.entry_eligible(make));
	assert!(!program.entry_eligible(take));
	let returned = executor
		.invoke(&program, &mut store, make, &[], 1000)
		.unwrap();
	let Type::Struct(nominal) = store.root_tag(&returned).unwrap() else {
		panic!("named struct")
	};
	let wrong = store.root_i64(99);
	store.root_set_field(&returned, "f", &wrong).unwrap();
	let built = store.root_record(Some(nominal), &[("f", &wrong)]).unwrap();
	let before = (store.live_entries(), store.payload_bytes());
	for root in [&returned, &built] {
		for fuel in [0, 1000] {
			let error = executor
				.invoke(&program, &mut store, take, &[root], fuel)
				.unwrap_err();
			assert_eq!(error.error.refusal, Some(Refusal::Entry));
			assert!(
				executor.is_idle() && error.cleanup_error.is_none() && error.call_chain.is_empty()
			);
			assert_eq!((store.live_entries(), store.payload_bytes()), before);
		}
	}
	let result = executor
		.invoke(&program, &mut store, main, &[], 1000)
		.unwrap();
	assert!(matches!(store.import(&result).unwrap(), Value::I64(7)));
	for root in [result, returned, built, wrong] {
		store.root_release(root).unwrap();
	}
	assert_eq!((store.live_entries(), store.payload_bytes()), (0, 0));
}

#[test]
fn field_receiver_disjoint_domains_keep_their_host_entry_obligations() {
	for source in [
		"struct R; struct S; struct Box{f} impl R{fn capacity(self){7}} impl S{fn capacity(self){11}} fn write(x){x.f=S;} pub fn main(){write(#{f:R});let b=Box{f:R};b.f.capacity()}",
		"struct R; struct S; struct Box{f} struct Other{f} impl R{fn capacity(self){7}} impl S{fn capacity(self){11}} fn write(x){x.f=S;} pub fn main(){write(Other{f:R});let b=Box{f:R};b.f.capacity()}",
		"struct R; struct S; struct Box{f} struct Other; impl R{fn capacity(self){7}} impl S{fn capacity(self){11}} impl Box{fn read(self){self.f.capacity()}} impl Other{fn read(self){7}} fn take(x){x.read()} pub fn main(){let b=Box{f:R};b.read();take(Other)}",
	] {
		let row=json!({"source":source});
		let (program,_)=fresh(&row);
		let name=if source.contains("fn write") {"write"} else {"take"};
		let entry=program.function(Hash::type_hash([name])).unwrap();
		assert!(!program.entry_eligible(entry),"{name}");
	}
}

#[test]
#[ignore = "operator-only grouped-write stock freeze"]
fn field_receiver_freeze_group_oracle() {
	use std::io::Write;
	let mut data: Json =
		serde_json::from_str(include_str!("field_receiver_group_contract.json")).unwrap();
	for row in data["rows"].as_array_mut().unwrap() {
		row["stock"] = grouped_stock(row);
		row["before"] = baseline(row);
	}
	let mut out = std::fs::OpenOptions::new()
		.create_new(true)
		.write(true)
		.open(std::env::var_os("C18F_ORACLE_OUT").unwrap())
		.unwrap();
	out.write_all(serde_json::to_string_pretty(&data).unwrap().as_bytes())
		.unwrap();
}

fn grouped_stock(row: &mut Json) -> Json {
	let context = context(row);
	let mut sources = Sources::new();
	sources
		.insert(crate::Source::memory(row["source"].as_str().unwrap()).unwrap())
		.unwrap();
	let mut diagnostics = Diagnostics::new();
	match crate::prepare(&mut sources)
		.with_context(&context)
		.with_diagnostics(&mut diagnostics)
		.build_vm()
	{
		Ok(_) => stock(row),
		Err(error) => {
			json!({"compile_error":error.to_string(),"diagnostics":std::format!("{diagnostics:?}")})
		}
	}
}

#[test]
fn field_receiver_group_stock_is_frozen() {
	let data: Json =
		serde_json::from_str(include_str!("field_receiver_group_contract.json")).unwrap();
	for original in data["rows"].as_array().unwrap() {
		let mut row = original.clone();
		assert_eq!(
			digest(row["source"].as_str().unwrap().as_bytes()),
			row["source_sha256"]
		);
		let actual = grouped_stock(&mut row);
		let mut expected = original["stock"].clone();
		if !cfg!(feature = "serde") && !expected["unit_sha256"].is_null() {
			expected["unit_sha256"] = Json::Null;
		}
		assert_eq!(actual, expected, "{}", original["name"]);
	}
}
