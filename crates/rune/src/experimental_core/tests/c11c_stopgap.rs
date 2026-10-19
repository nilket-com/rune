//! Temporary P0 protection; frozen stock admissions remain untouched.
use super::*;
use serde_json::Value as Json;
use std::vec::Vec;

fn refusal(source: &str) -> Refusal {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	let error = match frontend::compile(&mut sources, &context, &mut diagnostics, 7) {
		Ok(_) => panic!("P0 source must not be admitted"),
		Err(error) => error,
	};
	error.refusal.expect("compile-time refusal required")
}

#[test]
fn all_ten_p0_seed_forms_are_refused_before_execution() {
	let rows: Vec<Json> = serde_json::from_str(include_str!("c11c_stopgap/seeds.json")).unwrap();
	assert_eq!(rows.len(), 10);
	for row in rows {
		let got = refusal(row["source"].as_str().unwrap());
		assert!(
			matches!(got, Refusal::Call | Refusal::CallClosure),
			"{}: {got:?}",
			row["label"]
		);
	}
}

#[test]
fn frozen_c11b_move_admissions_migrate_to_refusal() {
	let rows: Vec<Json> =
		serde_json::from_str(include_str!("gap_hunt/captures/cases.json")).unwrap();
	let frozen: Json =
		serde_json::from_str(include_str!("gap_hunt/captures/dispositions.json")).unwrap();
	for (index, label, graph) in [
		(
			5,
			"capture/move-scalar",
			r#"{"nodes":[["I64","0x1cad9186c9641c4f","9"]],"root":0}"#,
		),
		(
			6,
			"capture/move-heap",
			r#"{"nodes":[["Tuple","0xfeb65b3f4755aef0",[1,2]],["U64","0xb41669ccb4fdb76a","1"],["U64","0xb41669ccb4fdb76a","2"]],"root":0}"#,
		),
		(
			34,
			"capture/closure-captures-closure",
			r#"{"nodes":[["I64","0x1cad9186c9641c4f","4"]],"root":0}"#,
		),
	] {
		let old = &frozen[format!("{index:03}")];
		assert_eq!(rows[index]["label"], label);
		assert_eq!(old["label"], label);
		assert_eq!(old["disposition"], "Exact");
		assert_eq!(old["fresh_expectation"]["outcome"]["status"], "value");
		assert_eq!(
			old["fresh_expectation"]["outcome"]["graph"],
			serde_json::from_str::<Json>(graph).unwrap()
		);
		assert_eq!(
			refusal(rows[index]["source"].as_str().unwrap()),
			Refusal::CallClosure
		);
	}
}

#[test]
fn empty_move_and_nonmove_closures_stay_admitted() {
	assert_eq!(run("pub fn main(){let f=move ||7;f()}", &[]), Value::I64(7));
	assert_eq!(
		run("pub fn main(){let v=[7];let f=||v[0];f()}", &[]),
		Value::I64(7)
	);
	assert_eq!(
		run("fn make(v){||v[0]}pub fn main(){let f=make([7]);f()}", &[]),
		Value::I64(7)
	);
}
