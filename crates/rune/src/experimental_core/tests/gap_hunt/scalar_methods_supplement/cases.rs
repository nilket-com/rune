const COUNT: usize = 7;
const CASES: &[(&str, &str, &str, &str)] = &[
	(
		r###"supplement/direct-u64-max"###,
		r###"pub fn main(){i64::abs(18446744073709551615u64)}"###,
		r###"c6-main"###,
		r###"Exact"###,
	),
	(
		r###"supplement/method-u64-max"###,
		r###"pub fn main(){(18446744073709551615u64).abs()}"###,
		r###"c6-main"###,
		r###"Exact"###,
	),
	(
		r###"supplement/direct-u64-in-range"###,
		r###"pub fn main(){i64::abs(9223372036854775807u64)}"###,
		r###"c6-main"###,
		r###"Exact"###,
	),
	(
		r###"supplement/direct-u64-out-of-range"###,
		r###"pub fn main(){i64::abs(9223372036854775808u64)}"###,
		r###"c6-main"###,
		r###"Exact"###,
	),
	(
		r###"supplement/direct-bool"###,
		r###"pub fn main(){i64::abs(true)}"###,
		r###"c6-main"###,
		r###"Exact"###,
	),
	(
		r###"supplement/direct-unit"###,
		r###"pub fn main(){i64::abs(())}"###,
		r###"c6-main"###,
		r###"Exact"###,
	),
	(
		r###"supplement/receiver-write"###,
		r###"pub fn main(){{let x=2.0;let y=x.powf({x=3.0;2.0});y}}"###,
		r###"c6-main"###,
		r###"Exact"###,
	),
];
