// Physical VM windows are retained in raw receipts. D1e logical frames/context
// are the shared user-facing contract for both engines.
fn semantic_projection(p: &Json) -> Json {
	let mut p = p.clone();
	if p["outcome"]["status"] == "error" {
		let o = &p["outcome"];
		p["outcome"] = json!({"status":o["status"],"class":o["class"],"display":o["display"]});
	} else if p["outcome"]["status"] == "compile-error" {
		p["outcome"] = json!({"status":"compile-error"});
	}
	p
}
fn verify_record(actual: &Json, case: &Json, stock: &Json, disposition: &Json) {
	assert_eq!(actual["source_sha256"], case["source_sha256"]);
	assert_eq!(disposition["source_sha256"], case["source_sha256"]);
	let a = &actual["fresh_audit"];
	assert_eq!(a["cleanup_error"], false);
	for key in ["row_idle", "reuse_idle"] {
		assert_eq!(a[key], true, "{}: {key}", case["label"]);
	}
	for key in ["reuse_live_entries", "reuse_payload_bytes"] {
		assert_eq!(a[key], 0);
	}
	if case["origin"] == "C11b/020" {
		assert_eq!(a["row_live_entries"], 2);
		assert_eq!(a["row_payload_bytes"], 32);
		assert_eq!(a["reuse_same_store"], false);
	} else {
		assert_eq!(a["row_live_entries"], 0);
		assert_eq!(a["row_payload_bytes"], 0);
		assert_eq!(a["reuse_same_store"], true);
	}
	if actual["observation"]["outcome"]["status"] == "error" {
		// Actual core ErrorKind, never a test-side reclassification.
		assert_eq!(
			actual["observation"]["outcome"]["kind"],
			actual["observation"]["outcome"]["class"]
		);
	}
	match disposition["classification"].as_str().unwrap() {
		"exact" => assert_eq!(
			semantic_projection(&actual["projected"]),
			semantic_projection(&stock["projected"]),
			"{}",
			case["label"]
		),
		"inherited-refusal" => {
			let o = &actual["observation"]["outcome"];
			assert_eq!(o["status"], "refused");
			assert_eq!(o["kind"], "Unsupported");
			assert_eq!(o["refusal"], disposition["refusal"]);
			assert_eq!(o["location"], disposition["site"]);
			assert_eq!(
				disposition["protected_main_refusal"]["refusal"],
				o["refusal"]
			);
			assert_eq!(
				disposition["protected_main_refusal"]["location"],
				o["location"]
			);
			assert_eq!(a["invoked"], false);
		}
		"policy-presentation" => {
            let mut fresh = semantic_projection(&actual["projected"]);
            let old = semantic_projection(&stock["projected"]);
            match case["label"].as_str().unwrap() {
                "c11b/error/wrong-arity" => {
                    assert_eq!(fresh["outcome"]["display"], "Wrong number of arguments 0, expected 1");
                    assert_eq!(old["outcome"]["display"], "Wrong number of arguments 1, expected 2");
                    let old_rendered = old["logical"]["rendered"].as_str().unwrap();
                    let new_rendered = fresh["logical"]["rendered"].as_str().unwrap();
                    assert_eq!(old_rendered.split_once('\n').unwrap().1, new_rendered.split_once('\n').unwrap().1);
                    fresh["logical"]["rendered"] = old["logical"]["rendered"].clone();
                    fresh["outcome"]["display"] = old["outcome"]["display"].clone();
                }
                "c11b/error/fuel-in-call" => {
                    assert_eq!(fresh["outcome"]["class"], "FuelExhausted");
                    assert_eq!(old["outcome"]["class"], "FuelExhausted");
                    assert_eq!(fresh["outcome"]["display"], "execute: FuelExhausted");
                    assert_eq!(old["outcome"]["display"], "Exhausted the budget it was given");
                    assert_eq!(actual["projected"]["outcome"]["sites"], json!([{"source":0,"start":39,"end":45,"frames":"[{\"end\":50,\"source\":0,\"start\":47}]"}]));
                    assert_eq!(stock["projected"]["outcome"]["sites"], json!([]));
                    assert_eq!(fresh["logical"]["frames"], json!([{"role":"Origin","location":{"source":0,"start":39,"end":45}},{"role":"Caller","location":{"source":0,"start":47,"end":50}}]));
                    assert_eq!(old["logical"]["frames"], json!([{"role":"Origin","location":null}]));
                    assert_eq!(fresh["logical"]["context"], json!([]));
                    assert_eq!(old["logical"]["context"], json!([]));
                    assert_eq!(fresh["logical"]["rendered"], "execute: FuelExhausted\n  at c11b.rn:1:40\n  called from c11b.rn:1:48");
                    assert_eq!(old["logical"]["rendered"], "Exhausted the budget it was given\n  at <unknown>");
                    assert_eq!(fresh["outcome"]["status"], "error");
                    assert_eq!(old["outcome"]["status"], "error");
                    fresh["outcome"] = old["outcome"].clone(); fresh["logical"] = old["logical"].clone();
                }
                "c11b/identity/format" => {
                    let decode = |value: &Json| unhex(value["outcome"]["graph"]["nodes"][0][2].as_str().unwrap());
                    let old_text = decode(&old); let new_text = decode(&fresh);
                    assert!(old_text.starts_with("closure (at: ") && old_text.ends_with(", env:[2])"));
                    assert!(new_text.starts_with("closure (function: ") && new_text.ends_with(", env:[2])"));
                    fresh["outcome"]["graph"]["nodes"][0][2] = old["outcome"]["graph"]["nodes"][0][2].clone();
                }
                "c11b/capture/store-object-method-call" => {
                    assert_eq!(fresh["outcome"]["display"], "Type `std::object::Object` has no method named `f`");
                    assert_eq!(old["outcome"]["display"], "Type `std::object::Object` has no matching method");
                    fresh["outcome"]["display"] = old["outcome"]["display"].clone();
                }
                label => panic!("unreviewed presentation {label}"),
            }
            assert_eq!(fresh, old, "{}", case["label"]);
        }

		_ => panic!("unreviewed C11c disposition"),
	}
}
#[cfg(target_os = "linux")]
#[test]
#[ignore = "C11c strict execution71 against live stock, bounded children and exclusive GAP_OUT"]
fn contract() {
	run_contract_cases(false);
}

// The independent new child never reads these expected outcomes. Only
// immutable function-token moves differ from stock.
fn verify_function_copy_record(actual: &Json, case: &Json, stock: &Json, disposition: &Json) {
	assert_eq!(actual["source_sha256"], case["source_sha256"]);
	assert_eq!(disposition["source_sha256"], case["source_sha256"]);
	assert_eq!(disposition["classification"], "Intentional");
	assert_eq!(disposition["authority"], 20725);
	let mut fresh = semantic_projection(&actual["projected"]);
    let old = semantic_projection(&stock["projected"]);
    let label = case["label"].as_str().unwrap();
    let value = match label {
        "token/named-alias" | "token/empty-closure-alias" | "token/independent-values" | "token/container-moved-into-capture" | "token/function-capture-returned" => json!({"status":"value","graph":{"root":0,"nodes":[["I64","0x1cad9186c9641c4f","16"]]}}),
        "token/vector-alias" | "token/object-alias" => json!({"status":"value","graph":{"root":0,"nodes":[["I64","0x1cad9186c9641c4f","8"]]}}),
        "token/callback-after-move" => json!({"status":"value","graph":{"root":0,"nodes":[["Vec","0xc11b1e769aea94f2",[1,2]],["I64","0x1cad9186c9641c4f","1"],["I64","0x1cad9186c9641c4f","2"]]}}),
        _ => panic!("unreviewed token case {label}"),
    };
    assert_eq!(fresh["outcome"], value);
    if matches!(label, "token/independent-values" | "token/container-moved-into-capture") {
        assert_eq!(fresh, old);
    } else {
        assert_eq!(old["outcome"]["status"], "error");
        assert_eq!(old["outcome"]["class"], "AccessConflict");
        assert_eq!(old["outcome"]["display"], "Cannot read the value: it has been moved");
        assert!(fresh["logical"].is_null());
        assert!(old["logical"]["frames"].as_array().is_some_and(|frames| !frames.is_empty()));
        assert!(old["logical"]["rendered"].as_str().is_some_and(|text| text.contains("moved")));
        fresh["outcome"] = old["outcome"].clone();
        fresh["logical"] = old["logical"].clone();
        assert_eq!(fresh, old);
    }
	let audit = &actual["fresh_audit"];
	assert_eq!(audit["cleanup_error"], false);
	for key in ["row_idle", "reuse_idle", "reuse_same_store"] {
		assert_eq!(audit[key], true);
	}
	for key in [
		"row_live_entries",
		"row_payload_bytes",
		"reuse_live_entries",
		"reuse_payload_bytes",
	] {
		assert_eq!(audit[key], 0);
	}
}
#[cfg(target_os = "linux")]
#[test]
#[ignore = "C11c Copy policy against live stock, bounded independent execution8 children"]
fn function_copy_contract() {
	run_contract_cases(true);
}
