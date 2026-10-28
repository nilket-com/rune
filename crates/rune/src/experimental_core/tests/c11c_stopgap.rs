//! C11c root migration: retain the stopgap history and restore proved admissions.
use super::*;
use serde_json::Value as Json;
use std::vec::Vec;

fn compile_refusal(source: &str) -> core::result::Result<Option<Refusal>, ErrorKind> {
	let context = Context::with_default_modules().unwrap();
	let mut sources = Sources::new();
	sources.insert(Source::memory(source).unwrap()).unwrap();
	let mut diagnostics = Diagnostics::new();
	let error = match frontend::compile(&mut sources, &context, &mut diagnostics, 7) {
		Ok(_) => return Ok(None),
		Err(error) => error,
	};
	match error.refusal {
		Some(refusal) => Ok(Some(refusal)),
		None => Err(error.kind),
	}
}

#[test]
fn p0_seed_admissions_follow_verified_root_dispositions() {
	let rows: Vec<Json> = serde_json::from_str(include_str!("c11c_stopgap/seeds.json")).unwrap();
	let root: Json =
		serde_json::from_str(include_str!("gap_hunt/move_captures/disposition.json")).unwrap();
	assert_eq!(rows.len(), 10);
	for (index, row) in rows.iter().enumerate() {
		let disposition = &root["rows"][index];
		let got = compile_refusal(row["source"].as_str().unwrap());
		if disposition["classification"] == "inherited-refusal" {
			assert_eq!(got, Ok(Some(Refusal::Call)), "{}", row["label"]);
		} else {
			assert_eq!(disposition["classification"], "exact");
			if matches!(index, 6 | 7) {
				// Real VariableMoved diagnostic remains a compile error.
				assert_eq!(got, Ok(Some(Refusal::FrontendDiagnostic)));
			} else {
				assert_eq!(got, Ok(None), "{}", row["label"]);
			}
		}
	}
}

#[test]
fn c11b_move_admissions_restored_after_root_proof() {
	let rows: Vec<Json> =
		serde_json::from_str(include_str!("gap_hunt/captures/cases.json")).unwrap();
	let frozen: Json =
		serde_json::from_str(include_str!("gap_hunt/captures/dispositions.json")).unwrap();
	for (index, label) in [
		(5, "capture/move-scalar"),
		(6, "capture/move-heap"),
		(34, "capture/closure-captures-closure"),
	] {
		let old = &frozen[format!("{index:03}")];
		assert_eq!(rows[index]["label"], label);
		assert_eq!(old["label"], label);
		assert_eq!(old["disposition"], "Exact");
		assert_eq!(
			compile_refusal(rows[index]["source"].as_str().unwrap()),
			Ok(None)
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
