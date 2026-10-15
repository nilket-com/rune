// Prospective policies; stock observations and peer dispositions precede freeze.
const COUNT: usize = 126;
const CASES: &[(&str, &str, &str)] = &[
	(
		r#####"plain/unit"#####,
		r#####"pub fn main(){c10::make("unit")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/unit"#####,
		r#####"pub fn main(){c10::fallible("unit")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/bool"#####,
		r#####"pub fn main(){c10::make("bool")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/bool"#####,
		r#####"pub fn main(){c10::fallible("bool")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/i64-min"#####,
		r#####"pub fn main(){c10::make("i64-min")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/i64-min"#####,
		r#####"pub fn main(){c10::fallible("i64-min")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/i64-max"#####,
		r#####"pub fn main(){c10::make("i64-max")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/i64-max"#####,
		r#####"pub fn main(){c10::fallible("i64-max")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/u64-max"#####,
		r#####"pub fn main(){c10::make("u64-max")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/u64-max"#####,
		r#####"pub fn main(){c10::fallible("u64-max")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/char"#####,
		r#####"pub fn main(){c10::make("char")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/char"#####,
		r#####"pub fn main(){c10::fallible("char")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/less"#####,
		r#####"pub fn main(){c10::make("less")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/less"#####,
		r#####"pub fn main(){c10::fallible("less")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/equal"#####,
		r#####"pub fn main(){c10::make("equal")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/equal"#####,
		r#####"pub fn main(){c10::fallible("equal")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/greater"#####,
		r#####"pub fn main(){c10::make("greater")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/greater"#####,
		r#####"pub fn main(){c10::fallible("greater")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/string-empty"#####,
		r#####"pub fn main(){c10::make("string-empty")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/string-empty"#####,
		r#####"pub fn main(){c10::fallible("string-empty")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/string"#####,
		r#####"pub fn main(){c10::make("string")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/string"#####,
		r#####"pub fn main(){c10::fallible("string")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/vec-empty"#####,
		r#####"pub fn main(){c10::make("vec-empty")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/vec-empty"#####,
		r#####"pub fn main(){c10::fallible("vec-empty")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/tuple-empty"#####,
		r#####"pub fn main(){c10::make("tuple-empty")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/tuple-empty"#####,
		r#####"pub fn main(){c10::fallible("tuple-empty")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/object-empty"#####,
		r#####"pub fn main(){c10::make("object-empty")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/object-empty"#####,
		r#####"pub fn main(){c10::fallible("object-empty")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/none"#####,
		r#####"pub fn main(){c10::make("none")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/none"#####,
		r#####"pub fn main(){c10::fallible("none")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/some"#####,
		r#####"pub fn main(){c10::make("some")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/some"#####,
		r#####"pub fn main(){c10::fallible("some")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/ok"#####,
		r#####"pub fn main(){c10::make("ok")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/ok"#####,
		r#####"pub fn main(){c10::fallible("ok")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/err"#####,
		r#####"pub fn main(){c10::make("err")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/err"#####,
		r#####"pub fn main(){c10::fallible("err")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/mixed"#####,
		r#####"pub fn main(){c10::make("mixed")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/mixed"#####,
		r#####"pub fn main(){c10::fallible("mixed")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/f64/0000000000000000"#####,
		r#####"pub fn main(){c10::make("f64/0000000000000000")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/f64/8000000000000000"#####,
		r#####"pub fn main(){c10::make("f64/8000000000000000")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/f64/3ff0000000000000"#####,
		r#####"pub fn main(){c10::make("f64/3ff0000000000000")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/f64/0000000000000001"#####,
		r#####"pub fn main(){c10::make("f64/0000000000000001")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/f64/7ff0000000000000"#####,
		r#####"pub fn main(){c10::make("f64/7ff0000000000000")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/f64/fff0000000000000"#####,
		r#####"pub fn main(){c10::make("f64/fff0000000000000")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/f64/7ff8000000000042"#####,
		r#####"pub fn main(){c10::make("f64/7ff8000000000042")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/f64/fff800000000002a"#####,
		r#####"pub fn main(){c10::make("f64/fff800000000002a")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"plain/f64/7ff0000000000001"#####,
		r#####"pub fn main(){c10::make("f64/7ff0000000000001")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/error"#####,
		r#####"pub fn main(){c10::fallible("error")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"fallible/error-try"#####,
		r#####"pub fn main(){c10::fallible("error")?}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/graph/dag-vec"#####,
		r#####"pub fn main(){c10::make("dag-vec")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/graph/dag-object"#####,
		r#####"pub fn main(){c10::make("dag-object")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/graph/dag-string"#####,
		r#####"pub fn main(){c10::make("dag-string")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/graph/diamond"#####,
		r#####"pub fn main(){c10::make("diamond")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/graph/distinct-equal"#####,
		r#####"pub fn main(){c10::make("distinct-equal")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/result-graph/dag-vec"#####,
		r#####"pub fn main(){c10::fallible("dag-vec")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/result-graph/dag-object"#####,
		r#####"pub fn main(){c10::fallible("dag-object")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/result-graph/dag-string"#####,
		r#####"pub fn main(){c10::fallible("dag-string")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/result-graph/diamond"#####,
		r#####"pub fn main(){c10::fallible("diamond")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/result-graph/distinct-equal"#####,
		r#####"pub fn main(){c10::fallible("distinct-equal")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/mutate-vec"#####,
		r#####"pub fn main(){let v=c10::make("dag-vec");v[0][0]=99;v[1][0]}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/mutate-object"#####,
		r#####"pub fn main(){let v=c10::make("dag-object");v[0].n=99;v[1].n}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/mutate-diamond"#####,
		r#####"pub fn main(){let v=c10::make("diamond");v[0][0][0]=99;v[1][0][0]}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"sharing/distinct-equal"#####,
		r#####"pub fn main(){let v=c10::make("distinct-equal");v[0][0]=99;v[1][0]}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"isolation/external-host-mutation"#####,
		r#####"pub fn main(){let a=c10::make("external");c10::mutate();a[0]}
pub fn c10_reuse(){7}"#####,
		r#####"Retained-isolation"#####,
	),
	(
		r#####"isolation/external-script-mutation"#####,
		r#####"pub fn main(){let a=c10::make("external");a[0]=42;c10::inspect()}
pub fn c10_reuse(){7}"#####,
		r#####"Retained-isolation"#####,
	),
	(
		r#####"isolation/cross-call-alias"#####,
		r#####"pub fn main(){let a=c10::make("external");let b=c10::make("external");a[0]=42;b[0]}
pub fn c10_reuse(){7}"#####,
		r#####"Retained-isolation"#####,
	),
	(
		r#####"isolation/cross-call-later-snapshot"#####,
		r#####"pub fn main(){let a=c10::make("external");a[0]=42;c10::make("external")[0]}
pub fn c10_reuse(){7}"#####,
		r#####"Retained-isolation"#####,
	),
	(
		r#####"cycle/cycle-vec"#####,
		r#####"pub fn main(){c10::make("cycle-vec")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"cycle/cycle-object"#####,
		r#####"pub fn main(){c10::make("cycle-object")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"cycle/cycle-two"#####,
		r#####"pub fn main(){c10::make("cycle-two")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"cycle/cycle-tuple"#####,
		r#####"pub fn main(){c10::make("cycle-tuple")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"cycle/cycle-option"#####,
		r#####"pub fn main(){c10::make("cycle-option")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"cycle/cycle-result"#####,
		r#####"pub fn main(){c10::make("cycle-result")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"cycle/unused-result"#####,
		r#####"pub fn main(){let v=c10::make("cycle-vec");7}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"unsupported/unused-result"#####,
		r#####"pub fn main(){let v=c10::make("custom");7}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"access/shared-read"#####,
		r#####"pub fn main(){c10::make("shared-read")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"access/exclusive"#####,
		r#####"pub fn main(){c10::make("exclusive")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"access/moved"#####,
		r#####"pub fn main(){c10::make("moved")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"access/deep-exclusive"#####,
		r#####"pub fn main(){c10::make("deep-exclusive")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"access/deep-moved"#####,
		r#####"pub fn main(){c10::make("deep-moved")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"unsupported/empty"#####,
		r#####"pub fn main(){c10::make("empty")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"unsupported/type"#####,
		r#####"pub fn main(){c10::make("type")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"unsupported/hash"#####,
		r#####"pub fn main(){c10::make("hash")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"unsupported/bytes"#####,
		r#####"pub fn main(){c10::make("bytes")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"unsupported/bytes-empty"#####,
		r#####"pub fn main(){c10::make("bytes-empty")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"unsupported/custom"#####,
		r#####"pub fn main(){c10::make("custom")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"unsupported/function"#####,
		r#####"pub fn main(){c10::make("function")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"unsupported/future"#####,
		r#####"pub fn main(){c10::make("future")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"unsupported/range"#####,
		r#####"pub fn main(){c10::make("range")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"unsupported/nominal"#####,
		r#####"pub fn main(){c10::make("nominal")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"unsupported/nested-custom"#####,
		r#####"pub fn main(){c10::make("nested-custom")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"depth/plain/0"#####,
		r#####"pub fn main(){c10::make("depth/0")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"depth/plain/1"#####,
		r#####"pub fn main(){c10::make("depth/1")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"depth/plain/127"#####,
		r#####"pub fn main(){c10::make("depth/127")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"depth/plain/128"#####,
		r#####"pub fn main(){c10::make("depth/128")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"depth/plain/129"#####,
		r#####"pub fn main(){c10::make("depth/129")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"depth/result/126"#####,
		r#####"pub fn main(){c10::fallible("depth/126")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"depth/result/127"#####,
		r#####"pub fn main(){c10::fallible("depth/127")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"depth/result/128"#####,
		r#####"pub fn main(){c10::fallible("depth/128")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"visits/plain/65534"#####,
		r#####"pub fn main(){c10::make("visits/65534")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"visits/plain/65535"#####,
		r#####"pub fn main(){c10::make("visits/65535")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"visits/plain/65536"#####,
		r#####"pub fn main(){c10::make("visits/65536")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"visits/result/65533"#####,
		r#####"pub fn main(){c10::fallible("visits/65533")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"visits/result/65534"#####,
		r#####"pub fn main(){c10::fallible("visits/65534")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"visits/result/65535"#####,
		r#####"pub fn main(){c10::fallible("visits/65535")}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"large/key-1MiB"#####,
		r#####"pub fn main(){c10::make("key/1048576")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/0"#####,
		r#####"pub fn main(){c10::parse("null")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/1"#####,
		r#####"pub fn main(){c10::parse("true")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/2"#####,
		r#####"pub fn main(){c10::parse("-9223372036854775808")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/3"#####,
		r#####"pub fn main(){c10::parse("18446744073709551615")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/4"#####,
		r#####"pub fn main(){c10::parse("-0.0")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/5"#####,
		r#####"pub fn main(){c10::parse("\"\"")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/6"#####,
		r#####"pub fn main(){c10::parse("\"α\\u0000🦀\"")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/7"#####,
		r#####"pub fn main(){c10::parse("[]")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/8"#####,
		r#####"pub fn main(){c10::parse("{}")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/9"#####,
		r#####"pub fn main(){c10::parse("[null,true,1,-1,18446744073709551615,1.5]")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/10"#####,
		r#####"pub fn main(){c10::parse("{\"a\":[1,{\"b\":\"α\"}],\"z\":null}")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/11"#####,
		r#####"pub fn main(){c10::parse("{")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/12"#####,
		r#####"pub fn main(){c10::parse("[")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/document/13"#####,
		r#####"pub fn main(){c10::parse("1 2")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/parser-depth/127"#####,
		r#####"pub fn main(){c10::parse("[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[0]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/parser-depth/128"#####,
		r#####"pub fn main(){c10::parse("[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[0]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]")}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"json/orders"#####,
		r#####"pub fn main(){let orders=c10::parse("[{\"region\":\"n\",\"status\":\"shipped\",\"qty\":2,\"unit_cents\":9}]")?;let cents=0;for o in orders {if o.status=="shipped"{cents=cents+o.qty*o.unit_cents;}}cents}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"argument/wrong-type"#####,
		r#####"pub fn main(){c10::make(1)}
pub fn c10_reuse(){7}"#####,
		r#####"Exact"#####,
	),
	(
		r#####"argument/wrong-arity"#####,
		r#####"pub fn main(){c10::make()}
pub fn c10_reuse(){7}"#####,
		r#####"Retained-arity"#####,
	),
	(
		r#####"frames/deep-error"#####,
		r#####"fn inner(){c10::make("custom")}fn outer(){inner()}pub fn main(){outer()}
pub fn c10_reuse(){7}"#####,
		r#####"Refused-runtime"#####,
	),
];
