fn check_intentional(label: &str, source: &str, stock: &Json, fresh: &Json, ruling: &Json) {
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
			assert_eq!(*fresh, json!({"status":"value","type":"I64","value":"7"}));
			assert_eq!(ruling["invoked"], true);
		}
		"syntax/duplicate-name" => {
			assert_eq!(source, "pub fn main(){let(a,a)=(1,2);a}");
			assert_eq!(*stock, json!({"status":"value","type":"I64","value":"2"}));
			assert_eq!(
				*fresh,
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

#[test]
#[cfg(not(miri))]
fn intentional_rulings_reject_drift() {
	let frozen = include_str!("../../tuple_let_stock.tsv");
	let mut checked = 0;
	for line in frozen.lines().skip(1) {
		let c = line.split('\t').collect::<Vec<_>>();
		if c[6] != "Intentional-divergence" {
			continue;
		}
		let source = policy_decode(c[2]);
		let stock: Json = serde_json::from_str(&policy_decode(c[3])).unwrap();
		let fresh: Json = serde_json::from_str(&policy_decode(c[7])).unwrap();
		let ruling: Json = serde_json::from_str(&policy_decode(c[8])).unwrap();
		check_intentional(c[0], &source, &stock, &fresh, &ruling);
		assert_ne!(stock, fresh);
		let rejects = |label: &str, source: &str, stock: &Json, fresh: &Json, ruling: &Json| {
			assert!(std::panic::catch_unwind(|| check_intentional(
				label, source, stock, fresh, ruling
			))
			.is_err());
		};
		rejects(c[0], &source, &stock, &stock, &ruling);
		rejects("unlisted", &source, &stock, &fresh, &ruling);
		rejects(c[0], &(source.clone() + " "), &stock, &fresh, &ruling);
		let mut bad = ruling.clone();
		bad["message_id"] = json!("unreviewed");
		rejects(c[0], &source, &stock, &fresh, &bad);
		if c[0] == "syntax/duplicate-name" {
			let mut shifted = fresh.clone();
			shifted["start"] = json!(19);
			rejects(c[0], &source, &stock, &shifted, &ruling);
			let mut changed = stock.clone();
			changed["value"] = json!("3");
			rejects(c[0], &source, &changed, &fresh, &ruling);
		} else {
			let mut changed = fresh.clone();
			changed["value"] = json!("8");
			rejects(c[0], &source, &stock, &changed, &ruling);
		}
		checked += 1;
	}
	assert_eq!(checked, 2);
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
		let fresh: Json = serde_json::from_str(&policy_decode(c[7])).unwrap();
		let ruling: Json = serde_json::from_str(&policy_decode(c[8])).unwrap();
		check_intentional(c[0], &source, &stock, &fresh, &ruling);
		assert_ne!(stock, fresh);
		let rejects = |label: &str, source: &str, stock: &Json, fresh: &Json, ruling: &Json| {
			assert!(std::panic::catch_unwind(|| check_intentional(
				label, source, stock, fresh, ruling
			))
			.is_err());
		};
		rejects(c[0], &source, &stock, &stock, &ruling);
		rejects("unlisted", &source, &stock, &fresh, &ruling);
		rejects(c[0], &(source.clone() + " "), &stock, &fresh, &ruling);
		let mut bad = ruling.clone();
		bad["message_id"] = json!("unreviewed");
		rejects(c[0], &source, &stock, &fresh, &bad);
		if c[0] == "syntax/duplicate-name" {
			let mut shifted = fresh.clone();
			shifted["start"] = json!(19);
			rejects(c[0], &source, &stock, &shifted, &ruling);
			let mut changed = stock.clone();
			changed["value"] = json!("3");
			rejects(c[0], &source, &changed, &fresh, &ruling);
		} else {
			let mut changed = fresh.clone();
			changed["value"] = json!("8");
			rejects(c[0], &source, &stock, &changed, &ruling);
		}
		checked += 1;
	}
	assert_eq!(checked, 2);
}
