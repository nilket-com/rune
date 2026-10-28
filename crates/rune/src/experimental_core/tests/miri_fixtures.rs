// Fixed Miri rows, validated against the full frozen assets by the native test below.
use serde_json::Value as Json;

pub(super) fn fixture(family: &str, label: &str) -> Json {
	let text = match (family, label) {
		("C8-intentional", "effects/return-from-rhs") => {
			r########"["effects/return-from-rhs","c8-main","70756220666e206d61696e28297b6c657428612c62293d7b72657475726e20373b28312c32297d3b612b627d","7b22646961676e6f7374696373223a225b466174616c28466174616c446961676e6f73746963207b20736f757263655f69643a20302c206b696e643a20436f6d70696c654572726f72284572726f72207b207370616e3a2031373a32322c206b696e643a20437573746f6d207b206572726f723a205c22556e626f756e64206e616d6520696e207061747465726e3a20315c22207d207d29207d295d222c226572726f72223a224275696c644572726f72207b206b696e643a2044656661756c74207d222c22737461747573223a22636f6d70696c652d6572726f72227d","","","Intentional-divergence","7b22737461747573223a2276616c7565222c2274797065223a22493634222c2276616c7565223a2237227d","7b2264656661756c745f7377697463685f646973636c6f73757265223a747275652c22696e766f6b6564223a747275652c226d6573736167655f6964223a2230316131336436642d386538372d373235332d623261642d383036333338313263643961222c22726561736f6e223a2253746f636b20617373656d626c657220636865636b7320756e626f756e64206e616d6573206166746572206120646976657267696e67205248533b206672657368206d7573742072657475726e372e222c22736571223a31343230337d"]"########
		}
		("C8-intentional", "syntax/duplicate-name") => {
			r########"["syntax/duplicate-name","c8-main","70756220666e206d61696e28297b6c657428612c61293d28312c32293b617d","7b22737461747573223a2276616c7565222c2274797065223a22493634222c2276616c7565223a2232227d","","","Intentional-divergence","7b22656e64223a32312c22696e766f6b6564223a66616c73652c226b696e64223a22556e737570706f72746564222c227265667573616c223a225061747465726e222c22736f75726365223a302c227370616e5f686578223a223631222c227374617274223a32302c22737461747573223a2272656675736564227d","7b2264656661756c745f7377697463685f646973636c6f73757265223a747275652c22696e766f6b6564223a66616c73652c226d6573736167655f6964223a2230316131336436642d386538372d373235332d623261642d383036333338313263643961222c22726561736f6e223a2252656a656374206475706c69636174652062696e64696e6720617420746865207365636f6e64206e616d653b20646f206e6f7420726570726f647563652073696c656e74207365636f6e642d77696e732e222c22736571223a31343230337d"]"########
		}
		("C9-P1", "receiver/i64/less") => {
			r########"["receiver/i64/less","c9-main","70756220666e206d61696e28297b2831292e636d702832297d0a70756220666e2063395f726575736528297b377d","-","0a72756e6e696e67203120746573740a","","Exact","-","-"]"########
		}
		("C9-P1", "receiver/i64/bool") => {
			r########"["receiver/i64/bool","c9-main","70756220666e206d61696e28297b2831292e636d702874727565297d0a70756220666e2063395f726575736528297b377d","-","0a72756e6e696e67203120746573740a","","Exact","-","-"]"########
		}
		("C9-P1", "receiver/f64/int") => {
			r########"["receiver/f64/int","c9-main","70756220666e206d61696e28297b28312e30292e636d702831297d0a70756220666e2063395f726575736528297b377d","-","0a72756e6e696e67203120746573740a","","Exact","-","-"]"########
		}
		("C9-P1", "float-bits/nan-left") => {
			r########"["float-bits/nan-left","c9-f64x2-7ff8000000000042-3ff0000000000000","70756220666e206d61696e28782c79297b782e636d702879297d0a70756220666e2063395f726575736528297b377d","-","0a72756e6e696e67203120746573740a","","Exact","-","-"]"########
		}
		("C9-P1", "format/display/Less") => {
			r########"["format/display/Less","c9-main","70756220666e206d61696e28297b666f726d61742128227b7d222c7374643a3a636d703a3a4f72646572696e673a3a4c657373297d0a70756220666e2063395f726575736528297b377d","-","0a72756e6e696e67203120746573740a","","Exact","-","-"]"########
		}
		("C9-P1", "refusal/partial-cmp") => {
			r########"["refusal/partial-cmp","c9-main","70756220666e206d61696e28297b312e7061727469616c5f636d702832297d0a70756220666e2063395f726575736528297b377d","-","0a72756e6e696e67203120746573740a","","Refused-surface","7b22656e64223a33302c22696e766f6b6564223a66616c73652c226b696e64223a22556e737570706f72746564222c226f7065726174696f6e223a224d6574686f64222c227265667573616c223a224d6574686f64222c22736f75726365223a302c227370616e5f686578223a223331326537303631373237343639363136633566363336643730323833323239222c227374617274223a31342c22737461747573223a2272656675736564227d","-"]"########
		}
		("C9-P1", "runtime-refusal/tuple") => {
			r########"["runtime-refusal/tuple","c9-main","70756220666e206d61696e28297b2828312c3229292e636d702828312c3329297d0a70756220666e2063395f726575736528297b377d","-","0a72756e6e696e67203120746573740a","","Refused-runtime","7b2263616c6c5f636861696e223a5b5d2c22656e64223a33322c22696e766f6b6564223a747275652c226b696e64223a22556e737570706f72746564222c226f7065726174696f6e223a224d6574686f64222c227265667573616c223a6e756c6c2c22736f75726365223a302c227370616e5f686578223a22323832383331326333323239323932653633366437303238323833313263333332393239222c227374617274223a31342c22737461747573223a2272756e74696d652d756e737570706f72746564227d","-"]"########
		}
		("C9-P1", "arity/i64/none") => {
			r########"["arity/i64/none","c9-main","70756220666e206d61696e28297b2831292e636d7028297d0a70756220666e2063395f726575736528297b377d","-","0a72756e6e696e67203120746573740a","","Retained-arity","7b22656e64223a32332c22696e766f6b6564223a66616c73652c226b696e64223a22556e737570706f72746564222c226f7065726174696f6e223a224172697479222c227265667573616c223a224172697479222c22736f75726365223a302c227370616e5f686578223a22323833313239326536333664373032383239222c227374617274223a31342c22737461747573223a2272656675736564227d","-"]"########
		}
		("C9-leased", "access/leased-parent-mutate") => {
			r########"{"disposition":{"index":170,"label":"access/leased-parent-mutate","source_sha256":"ed04c37527950d21b66bfea500cac197003fc09b5c6ee69d008c32ef432a8519","policy":"Exact","ruling":null},"counts":{"/chain/0/kind":0,"/kind":0,"/raw":0,"/typed/raw_kind":0}}"########
		}
		("C9-diagnostics", "format/display/Less") => {
			r########"{"disposition":{"index":120,"label":"format/display/Less","source_sha256":"a335297156466dd086cfa6418d55554aac98fd507c89215442586ab8b49e6b53","policy":"Exact","ruling":null},"counts":{"/kind":1,"/raw":1,"/typed/raw_kind":1}}"########
		}
		("C9-diagnostics", "frames/missing") => {
			r########"{"disposition":{"index":173,"label":"frames/missing","source_sha256":"cbb65703d94e95efe5f33c566ddf3fba430c9fadfa54b9125c39fc2e0eacc676","policy":"Exact","ruling":null},"counts":{"/kind":1,"/raw":1}}"########
		}
		("C9-counts", "receiver/i64/less") => r########"{}"########,
		("C9-counts", "receiver/i64/bool") => {
			r########"{"/chain/0/kind":0,"/kind":1,"/raw":1,"/typed/raw_kind":1}"########
		}
		("C9-counts", "receiver/f64/int") => {
			r########"{"/chain/0/kind":0,"/kind":2,"/raw":2,"/typed/raw_kind":2}"########
		}
		("C9-counts", "float-bits/nan-left") => r########"{"/kind":0,"/raw":0}"########,
		("C9-counts", "format/display/Less") => {
			r########"{"/kind":1,"/raw":1,"/typed/raw_kind":1}"########
		}
		("C9-counts", "refusal/partial-cmp") => r########"{}"########,
		("C9-counts", "runtime-refusal/tuple") => r########"{}"########,
		("C9-counts", "arity/i64/none") => r########"{"/kind":0,"/raw":0}"########,
		_ => panic!("unlisted Miri fixture"),
	};
	serde_json::from_str(text).unwrap()
}

#[test]
#[cfg(not(miri))]
fn selected_miri_fixtures_equal_frozen_assets() {
	for (family, frozen, labels) in [(
		"C9-P1",
		include_str!("cmp_ordering_inputs.tsv"),
		&[
			"receiver/i64/less",
			"receiver/i64/bool",
			"receiver/f64/int",
			"float-bits/nan-left",
			"format/display/Less",
			"refusal/partial-cmp",
			"runtime-refusal/tuple",
			"arity/i64/none",
		][..],
	)] {
		for label in labels {
			let cells = frozen
				.lines()
				.skip(1)
				.find_map(|line| {
					let cells = line.split('\t').collect::<std::vec::Vec<_>>();
					(cells[0] == *label).then_some(cells)
				})
				.expect("pinned label in frozen asset");
			let mut expected = serde_json::json!(cells);
			expected[3] = serde_json::json!("-");
			expected[8] = serde_json::json!("-");
			if expected[6] == "Exact" {
				expected[7] = serde_json::json!("-");
			}
			assert_eq!(fixture(family, label), expected);
		}
	}
	let dispositions: Json =
		serde_json::from_str(include_str!("cmp_ordering_dispositions.json")).unwrap();
	let counts: Json = serde_json::from_str(include_str!("cmp_ordering_p1_counts.json")).unwrap();
	for label in [
		"receiver/i64/less",
		"receiver/i64/bool",
		"receiver/f64/int",
		"float-bits/nan-left",
		"format/display/Less",
		"refusal/partial-cmp",
		"runtime-refusal/tuple",
		"arity/i64/none",
	] {
		assert_eq!(fixture("C9-counts", label), counts[label]);
	}
	for (family, labels) in [
		("C9-leased", &["access/leased-parent-mutate"][..]),
		(
			"C9-diagnostics",
			&["format/display/Less", "frames/missing"][..],
		),
	] {
		for label in labels {
			assert_eq!(
				fixture(family, label),
				serde_json::json!({"disposition":dispositions[*label],"counts":counts[*label]})
			);
		}
	}
}
