fn check_intentional(label: &str, source: &str, stock: &Json, execution: &Json, ruling: &Json) {
	assert_eq!(ruling["message_id"], "01a13d6d-8e87-7253-b2ad-80633812cd9a");
	assert_eq!(ruling["seq"], 14203);
	assert_eq!(ruling["default_switch_disclosure"], true);
	match label {
		"effects/return-from-rhs" => {
			assert_eq!(source, "pub fn main(){let(a,b)={return 7;(1,2)};a+b}");
			assert_eq!(stock["status"], "compile-error");
			assert!(stock["diagnostics"]
				.as_str()
				.unwrap()
				.contains("Unbound name in pattern"));
			assert_eq!(*execution, json!({"status":"value","type":"I64","value":"7"}));
			assert_eq!(ruling["invoked"], true);
		}
		"syntax/duplicate-name" => {
			assert_eq!(source, "pub fn main(){let(a,a)=(1,2);a}");
			assert_eq!(*stock, json!({"status":"value","type":"I64","value":"2"}));
			assert_eq!(
				*execution,
				json!({"status":"refused","kind":"Unsupported","refusal":"Pattern","source":0,"start":20,"end":21,"span_hex":"61","invoked":false})
			);
			assert_eq!(ruling["invoked"], false);
		}
		_ => panic!("unlisted intentional divergence"),
	}
}

fn policy_decode(text: &str) -> std::string::String {
	assert_eq!(text.len() % 2, 0);
	let bytes = (0..text.len())
		.step_by(2)
		.map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
		.collect::<std::vec::Vec<_>>();
	std::string::String::from_utf8(bytes).unwrap()
}

/// The reviewed execution expectation and ruling for each intentional divergence.
fn intentional_expectation(label: &str) -> (Json, Json) {
	match label {
		"effects/return-from-rhs" => (
			json!({"status":"value","type":"I64","value":"7"}),
			json!({"default_switch_disclosure":true,"invoked":true,"message_id":"01a13d6d-8e87-7253-b2ad-80633812cd9a","reason":"Stock assembler checks unbound names after a diverging RHS; fresh must return7.","seq":14203}),
		),
		"syntax/duplicate-name" => (
			json!({"end":21,"invoked":false,"kind":"Unsupported","refusal":"Pattern","source":0,"span_hex":"61","start":20,"status":"refused"}),
			json!({"default_switch_disclosure":true,"invoked":false,"message_id":"01a13d6d-8e87-7253-b2ad-80633812cd9a","reason":"Reject duplicate binding at the second name; do not reproduce silent second-wins.","seq":14203}),
		),
		_ => panic!("unlisted intentional divergence"),
	}
}

/// The compared stock projection of `source`, observed in process.
#[cfg(not(miri))]
fn legacy_projection(label: &str, source: &str, mode: &str) -> Json {
	project(&super::d1c_diagnostics::before(
		"tuple_let",
		label,
		source,
		"stock",
		&legacy_observe(source, mode),
	))
}

#[test]
#[cfg(not(miri))]
fn intentional_rulings_reject_drift() {
	let mut checked = 0;
	for &(label, source, mode, policy) in CASES {
		if policy != "Intentional-divergence" {
			continue;
		}
		let stock = legacy_projection(label, source, mode);
		let (execution, ruling) = intentional_expectation(label);
		check_intentional(label, source, &stock, &execution, &ruling);
		assert_ne!(stock, execution);
		let rejects = |label: &str, source: &str, stock: &Json, execution: &Json, ruling: &Json| {
			assert!(std::panic::catch_unwind(|| check_intentional(
				label, source, stock, execution, ruling
			))
			.is_err());
		};
		rejects(label, source, &stock, &stock, &ruling);
		rejects("unlisted", source, &stock, &execution, &ruling);
		rejects(label, &format!("{source} "), &stock, &execution, &ruling);
		let mut bad = ruling.clone();
		bad["message_id"] = json!("unreviewed");
		rejects(label, source, &stock, &execution, &bad);
		if label == "syntax/duplicate-name" {
			let mut shifted = execution.clone();
			shifted["start"] = json!(19);
			rejects(label, source, &stock, &shifted, &ruling);
			let mut changed = stock.clone();
			changed["value"] = json!("3");
			rejects(label, source, &changed, &execution, &ruling);
		} else {
			let mut changed = execution.clone();
			changed["value"] = json!("8");
			rejects(label, source, &stock, &changed, &ruling);
		}
		checked += 1;
	}
	assert_eq!(checked, 2);
}

/// The Miri fixtures carry the in-process stock projection and the reviewed
/// expectations, since Miri cannot observe the legacy engine itself.
#[test]
#[cfg(not(miri))]
fn miri_fixtures_match_live_stock() {
	for label in ["effects/return-from-rhs", "syntax/duplicate-name"] {
		let fixture =
			crate::experimental_core::tests::miri_fixtures::fixture("C8-intentional", label);
		let c = fixture
			.as_array()
			.unwrap()
			.iter()
			.map(|v| v.as_str().unwrap())
			.collect::<Vec<_>>();
		assert_eq!(c.len(), 9);
		let &(_, source, mode, policy) = CASES.iter().find(|case| case.0 == label).unwrap();
		assert_eq!(c[0], label);
		assert_eq!(c[1], mode);
		assert_eq!(policy_decode(c[2]), source);
		let stock: Json = serde_json::from_str(&policy_decode(c[3])).unwrap();
		assert_eq!(stock, legacy_projection(label, source, mode), "{label}");
		// Miri does not read captured streams.
		assert_eq!((c[4], c[5]), ("", ""));
		assert_eq!(c[6], policy);
		let (execution, ruling) = intentional_expectation(label);
		let fixed: Json = serde_json::from_str(&policy_decode(c[7])).unwrap();
		assert_eq!(fixed, execution);
		let fixed: Json = serde_json::from_str(&policy_decode(c[8])).unwrap();
		assert_eq!(fixed, ruling);
	}
}

#[test]
#[cfg(miri)]
fn miri_intentional_rulings_reject_drift() {
	let selected = ["effects/return-from-rhs", "syntax/duplicate-name"];
	let mut checked = 0;
	for label in selected {
		let fixture =
			crate::experimental_core::tests::miri_fixtures::fixture("C8-intentional", label);
		let c = fixture
			.as_array()
			.unwrap()
			.iter()
			.map(|v| v.as_str().unwrap())
			.collect::<Vec<_>>();
		assert_eq!(c[0], label);
		let source = policy_decode(c[2]);
		let stock: Json = serde_json::from_str(&policy_decode(c[3])).unwrap();
		let execution: Json = serde_json::from_str(&policy_decode(c[7])).unwrap();
		let ruling: Json = serde_json::from_str(&policy_decode(c[8])).unwrap();
		check_intentional(c[0], &source, &stock, &execution, &ruling);
		assert_ne!(stock, execution);
		let rejects = |label: &str, source: &str, stock: &Json, execution: &Json, ruling: &Json| {
			assert!(std::panic::catch_unwind(|| check_intentional(
				label, source, stock, execution, ruling
			))
			.is_err());
		};
		rejects(c[0], &source, &stock, &stock, &ruling);
		rejects("unlisted", &source, &stock, &execution, &ruling);
		rejects(c[0], &(source.clone() + " "), &stock, &execution, &ruling);
		let mut bad = ruling.clone();
		bad["message_id"] = json!("unreviewed");
		rejects(c[0], &source, &stock, &execution, &bad);
		if c[0] == "syntax/duplicate-name" {
			let mut shifted = execution.clone();
			shifted["start"] = json!(19);
			rejects(c[0], &source, &stock, &shifted, &ruling);
			let mut changed = stock.clone();
			changed["value"] = json!("3");
			rejects(c[0], &source, &changed, &execution, &ruling);
		} else {
			let mut changed = execution.clone();
			changed["value"] = json!("8");
			rejects(c[0], &source, &stock, &changed, &ruling);
		}
		checked += 1;
	}
	assert_eq!(checked, 2);
}
