// Source-only catalogue; policies require stock collection and peer freeze.
const COUNT: usize = 210;
const CASES: &[(&str, &str, &str, &str)] = &[
	(
		r#####"receiver/i64/less"#####,
		r#####"pub fn main(){(1).cmp(2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/equal"#####,
		r#####"pub fn main(){(1).cmp(1)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/greater"#####,
		r#####"pub fn main(){(1).cmp(-1)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/min"#####,
		r#####"pub fn main(){(1).cmp(-9223372036854775808)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/max"#####,
		r#####"pub fn main(){(1).cmp(9223372036854775807)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/cross-ok"#####,
		r#####"pub fn main(){(1).cmp(2u64)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/cross-max"#####,
		r#####"pub fn main(){(1).cmp(18446744073709551615u64)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/bool"#####,
		r#####"pub fn main(){(1).cmp(true)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/float"#####,
		r#####"pub fn main(){(1).cmp(2.0)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/string"#####,
		r#####"pub fn main(){(1).cmp("x")}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/ordering"#####,
		r#####"pub fn main(){(1).cmp(std::cmp::Ordering::Less)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/tuple"#####,
		r#####"pub fn main(){(1).cmp((1,2))}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/u64/less"#####,
		r#####"pub fn main(){(1u64).cmp(2u64)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/u64/equal"#####,
		r#####"pub fn main(){(1u64).cmp(1u64)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/u64/greater"#####,
		r#####"pub fn main(){(1u64).cmp(0u64)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/u64/max"#####,
		r#####"pub fn main(){(1u64).cmp(18446744073709551615u64)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/u64/cross-ok"#####,
		r#####"pub fn main(){(1u64).cmp(2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/u64/cross-negative"#####,
		r#####"pub fn main(){(1u64).cmp(-1)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/u64/bool"#####,
		r#####"pub fn main(){(1u64).cmp(true)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/u64/float"#####,
		r#####"pub fn main(){(1u64).cmp(2.0)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/u64/string"#####,
		r#####"pub fn main(){(1u64).cmp("x")}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/u64/ordering"#####,
		r#####"pub fn main(){(1u64).cmp(std::cmp::Ordering::Equal)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/f64/less"#####,
		r#####"pub fn main(){(1.0).cmp(2.0)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/f64/equal"#####,
		r#####"pub fn main(){(1.0).cmp(1.0)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/f64/greater"#####,
		r#####"pub fn main(){(1.0).cmp(-1.0)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/f64/int"#####,
		r#####"pub fn main(){(1.0).cmp(1)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/f64/unsigned"#####,
		r#####"pub fn main(){(1.0).cmp(1u64)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/f64/bool"#####,
		r#####"pub fn main(){(1.0).cmp(true)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/f64/string"#####,
		r#####"pub fn main(){(1.0).cmp("x")}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/f64/ordering"#####,
		r#####"pub fn main(){(1.0).cmp(std::cmp::Ordering::Greater)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/string/less"#####,
		r#####"pub fn main(){("α").cmp("β")}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/string/equal"#####,
		r#####"pub fn main(){("α").cmp("α")}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/string/greater"#####,
		r#####"pub fn main(){("α").cmp("a")}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/string/empty"#####,
		r#####"pub fn main(){("α").cmp("")}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/string/prefix"#####,
		r#####"pub fn main(){("α").cmp("αx")}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/string/nul"#####,
		r#####"pub fn main(){("α").cmp("α\0")}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/string/int"#####,
		r#####"pub fn main(){("α").cmp(1)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/string/bool"#####,
		r#####"pub fn main(){("α").cmp(true)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/string/float"#####,
		r#####"pub fn main(){("α").cmp(1.0)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/string/ordering"#####,
		r#####"pub fn main(){("α").cmp(std::cmp::Ordering::Less)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/string/owned"#####,
		r#####"pub fn main(){("α").cmp("α".clone())}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/negative"#####,
		r#####"pub fn main(){(-2).cmp(-1)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/zero"#####,
		r#####"pub fn main(){(0).cmp(0)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"receiver/i64/extremes"#####,
		r#####"pub fn main(){(-9223372036854775808).cmp(9223372036854775807)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"float-bits/pos-neg-zero"#####,
		r#####"pub fn main(x,y){x.cmp(y)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-f64x2-0000000000000000-8000000000000000"#####,
		r#####"Exact"#####,
	),
	(
		r#####"float-bits/neg-pos-zero"#####,
		r#####"pub fn main(x,y){x.cmp(y)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-f64x2-8000000000000000-0000000000000000"#####,
		r#####"Exact"#####,
	),
	(
		r#####"float-bits/pos-inf"#####,
		r#####"pub fn main(x,y){x.cmp(y)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-f64x2-7ff0000000000000-3ff0000000000000"#####,
		r#####"Exact"#####,
	),
	(
		r#####"float-bits/neg-inf"#####,
		r#####"pub fn main(x,y){x.cmp(y)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-f64x2-fff0000000000000-3ff0000000000000"#####,
		r#####"Exact"#####,
	),
	(
		r#####"float-bits/nan-left"#####,
		r#####"pub fn main(x,y){x.cmp(y)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-f64x2-7ff8000000000042-3ff0000000000000"#####,
		r#####"Exact"#####,
	),
	(
		r#####"float-bits/nan-right"#####,
		r#####"pub fn main(x,y){x.cmp(y)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-f64x2-3ff0000000000000-7ff8000000000042"#####,
		r#####"Exact"#####,
	),
	(
		r#####"float-bits/nan-both"#####,
		r#####"pub fn main(x,y){x.cmp(y)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-f64x2-7ff8000000000042-fff8000000000043"#####,
		r#####"Exact"#####,
	),
	(
		r#####"float-bits/signal-nan"#####,
		r#####"pub fn main(x,y){x.cmp(y)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-f64x2-7ff0000000000001-0000000000000000"#####,
		r#####"Exact"#####,
	),
	(
		r#####"float-bits/subnormal"#####,
		r#####"pub fn main(x,y){x.cmp(y)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-f64x2-0000000000000001-0000000000000000"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Less/qualified"#####,
		r#####"pub fn main(){std::cmp::Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Less/imported"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Less/alias"#####,
		r#####"use std::cmp::Ordering as O;pub fn main(){O::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Less/explicit-call"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less()}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Equal/qualified"#####,
		r#####"pub fn main(){std::cmp::Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Equal/imported"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Equal/alias"#####,
		r#####"use std::cmp::Ordering as O;pub fn main(){O::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Equal/explicit-call"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal()}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Greater/qualified"#####,
		r#####"pub fn main(){std::cmp::Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Greater/imported"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Greater/alias"#####,
		r#####"use std::cmp::Ordering as O;pub fn main(){O::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Greater/explicit-call"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater()}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/eq/Less-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less==Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/eq/Less-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less==Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/eq/Less-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less==Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/eq/Equal-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal==Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/eq/Equal-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal==Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/eq/Equal-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal==Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/eq/Greater-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater==Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/eq/Greater-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater==Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/eq/Greater-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater==Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ne/Less-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less!=Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ne/Less-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less!=Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ne/Less-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less!=Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ne/Equal-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal!=Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ne/Equal-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal!=Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ne/Equal-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal!=Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ne/Greater-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater!=Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ne/Greater-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater!=Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ne/Greater-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater!=Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/lt/Less-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less<Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/lt/Less-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less<Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/lt/Less-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less<Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/lt/Equal-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal<Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/lt/Equal-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal<Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/lt/Equal-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal<Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/lt/Greater-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater<Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/lt/Greater-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater<Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/lt/Greater-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater<Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/le/Less-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less<=Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/le/Less-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less<=Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/le/Less-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less<=Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/le/Equal-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal<=Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/le/Equal-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal<=Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/le/Equal-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal<=Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/le/Greater-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater<=Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/le/Greater-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater<=Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/le/Greater-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater<=Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/gt/Less-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less>Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/gt/Less-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less>Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/gt/Less-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less>Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/gt/Equal-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal>Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/gt/Equal-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal>Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/gt/Equal-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal>Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/gt/Greater-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater>Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/gt/Greater-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater>Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/gt/Greater-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater>Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ge/Less-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less>=Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ge/Less-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less>=Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ge/Less-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Less>=Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ge/Equal-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal>=Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ge/Equal-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal>=Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ge/Equal-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Equal>=Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ge/Greater-Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater>=Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ge/Greater-Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater>=Ordering::Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"ordering/ge/Greater-Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){Ordering::Greater>=Ordering::Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"format/debug/Less"#####,
		r#####"pub fn main(){format!("{:?}",std::cmp::Ordering::Less)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"format/display/Less"#####,
		r#####"pub fn main(){format!("{}",std::cmp::Ordering::Less)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"format/debug/Equal"#####,
		r#####"pub fn main(){format!("{:?}",std::cmp::Ordering::Equal)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"format/display/Equal"#####,
		r#####"pub fn main(){format!("{}",std::cmp::Ordering::Equal)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"format/debug/Greater"#####,
		r#####"pub fn main(){format!("{:?}",std::cmp::Ordering::Greater)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"format/display/Greater"#####,
		r#####"pub fn main(){format!("{}",std::cmp::Ordering::Greater)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"format/debug-nested"#####,
		r#####"pub fn main(){format!("{:?}",(std::cmp::Ordering::Less,[std::cmp::Ordering::Equal,std::cmp::Ordering::Greater]))}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"format/debug-width"#####,
		r#####"pub fn main(){format!("{:>12?}",std::cmp::Ordering::Greater)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"format/display-nested"#####,
		r#####"pub fn main(){format!("{}",(std::cmp::Ordering::Less,))}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"mixed-ordering/ordering-int/eq"#####,
		r#####"pub fn main(){std::cmp::Ordering::Less==1}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"mixed-ordering/ordering-int/lt"#####,
		r#####"pub fn main(){std::cmp::Ordering::Less<1}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"mixed-ordering/int-ordering/eq"#####,
		r#####"pub fn main(){1==std::cmp::Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"mixed-ordering/int-ordering/lt"#####,
		r#####"pub fn main(){1<std::cmp::Ordering::Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"mixed-ordering/ordering-bool/eq"#####,
		r#####"pub fn main(){std::cmp::Ordering::Equal==false}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"mixed-ordering/ordering-bool/lt"#####,
		r#####"pub fn main(){std::cmp::Ordering::Equal<false}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"match/Less"#####,
		r#####"use std::cmp::Ordering;pub fn main(){match Ordering::Less{Ordering::Less=>1,Ordering::Equal=>2,Ordering::Greater=>3}}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"match/Equal"#####,
		r#####"use std::cmp::Ordering;pub fn main(){match Ordering::Equal{Ordering::Less=>1,Ordering::Equal=>2,Ordering::Greater=>3}}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"match/Greater"#####,
		r#####"use std::cmp::Ordering;pub fn main(){match Ordering::Greater{Ordering::Less=>1,Ordering::Equal=>2,Ordering::Greater=>3}}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"match/wildcard"#####,
		r#####"use std::cmp::Ordering;pub fn main(){match Ordering::Greater{Ordering::Less=>1,_=>7}}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"match/if-let"#####,
		r#####"use std::cmp::Ordering;pub fn main(){if let Ordering::Less=1.cmp(2){7}else{0}}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"match/nested-caller"#####,
		r#####"use std::cmp::Ordering;fn inner(x){match x{Ordering::Less=>1,Ordering::Equal=>2,_=>3}}fn outer(x){inner(x)}pub fn main(){outer(2.cmp(1))}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"match/wrong-type"#####,
		r#####"use std::cmp::Ordering;pub fn main(){match 7{Ordering::Less=>1,_=>9}}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"missing-method/bool"#####,
		r#####"pub fn main(){(true).cmp(1)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"missing-method/char"#####,
		r#####"pub fn main(){('α').cmp(1)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"missing-method/ordering"#####,
		r#####"pub fn main(){(std::cmp::Ordering::Less).cmp(1)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"missing-method/object"#####,
		r#####"pub fn main(){(#{x:1}).cmp(1)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"missing-method/struct"#####,
		r#####"struct S{x}pub fn main(){(S{x:1}).cmp(1)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"runtime-refusal/tuple"#####,
		r#####"pub fn main(){((1,2)).cmp((1,3))}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"runtime-refusal/unit"#####,
		r#####"pub fn main(){(()).cmp(())}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"runtime-refusal/vec"#####,
		r#####"pub fn main(){([1,2]).cmp([1,3])}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"runtime-refusal/option"#####,
		r#####"pub fn main(){(Some(1)).cmp(Some(2))}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"runtime-refusal/result"#####,
		r#####"pub fn main(){(Ok(1)).cmp(Ok(2))}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"runtime-refusal/range"#####,
		r#####"pub fn main(){((1..3)).cmp((1..4))}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"arity/i64/none"#####,
		r#####"pub fn main(){(1).cmp()}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Retained-arity"#####,
	),
	(
		r#####"arity/i64/extra"#####,
		r#####"pub fn main(){(1).cmp(1,2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Retained-arity"#####,
	),
	(
		r#####"arity/u64/none"#####,
		r#####"pub fn main(){(1u64).cmp()}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Retained-arity"#####,
	),
	(
		r#####"arity/u64/extra"#####,
		r#####"pub fn main(){(1u64).cmp(1,2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Retained-arity"#####,
	),
	(
		r#####"arity/f64/none"#####,
		r#####"pub fn main(){(1.0).cmp()}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Retained-arity"#####,
	),
	(
		r#####"arity/f64/extra"#####,
		r#####"pub fn main(){(1.0).cmp(1,2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Retained-arity"#####,
	),
	(
		r#####"arity/string/none"#####,
		r#####"pub fn main(){("x").cmp()}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Retained-arity"#####,
	),
	(
		r#####"arity/string/extra"#####,
		r#####"pub fn main(){("x").cmp(1,2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Retained-arity"#####,
	),
	(
		r#####"effects/order-success"#####,
		r#####"pub fn main(){{print!("R");1}.cmp({print!("A");2})}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"effects/order-invalid-rhs"#####,
		r#####"pub fn main(){{print!("R");1}.cmp({print!("A");false})}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"effects/order-missing"#####,
		r#####"pub fn main(){{print!("R");true}.cmp({print!("A");2})}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"effects/order-runtime-refusal"#####,
		r#####"pub fn main(){{print!("R");[1]}.cmp({print!("A");[2]})}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"effects/supported-rhs-mutates-receiver"#####,
		r#####"pub fn main(){let s="a";let alias=s;let result=s.cmp({print!("A");alias+="z";"b"});(result,s)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"effects/refused-rhs-mutates-receiver"#####,
		r#####"pub fn main(){let v=[1];let alias=v;v.cmp({print!("A");alias.push(2);print!("{:?}",v);[3]})}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"access/moved-receiver"#####,
		r#####"pub fn main(){let s="a";let alias=s;drop(s);alias.cmp("b")}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"access/moved-rhs"#####,
		r#####"pub fn main(){let s="a";let alias=s;drop(s);"b".cmp(alias)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"access/shared-string-read"#####,
		r#####"pub fn main(){let s="a";let alias=s;(s.cmp(alias),s,alias)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"access/leased-parent-string"#####,
		r#####"pub fn main(){let v=["a"];for x in v{print!("{:?}",x.cmp("b"));}7}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"access/leased-parent-mutate"#####,
		r#####"pub fn main(){let v=["a"];for x in v{x.cmp({v.push("c");"b"});}7}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"destination/rebind"#####,
		r#####"pub fn main(){let o=1.cmp(2);let o=o;let p=o;(p,o)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"clone/global"#####,
		r#####"pub fn main(){let o=1.cmp(2);(clone(o),o)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"frames/missing"#####,
		r#####"fn inner(){true.cmp(1)}fn outer(){inner()}pub fn main(){outer()}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"frames/invalid-rhs"#####,
		r#####"fn inner(){1.cmp(false)}fn outer(){inner()}pub fn main(){outer()}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"frames/runtime-refusal"#####,
		r#####"fn inner(){[1].cmp([2])}fn outer(){inner()}pub fn main(){outer()}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-runtime"#####,
	),
	(
		r#####"region/fallback"#####,
		r#####"pub fn main(){let o=1.cmp(2);let i=0;let n=0;while i<300{if o==std::cmp::Ordering::Less{n+=1;}i+=1;}n}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"region/order-phi"#####,
		r#####"pub fn main(){let o=std::cmp::Ordering::Less;let i=0;while i<300{o=i.cmp(150);i+=1;}o}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"refusal/partial-cmp"#####,
		r#####"pub fn main(){1.partial_cmp(2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"refusal/reverse"#####,
		r#####"pub fn main(){std::cmp::Ordering::Less.reverse()}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"refusal/then"#####,
		r#####"pub fn main(){std::cmp::Ordering::Less.then(std::cmp::Ordering::Equal)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"refusal/then-with"#####,
		r#####"pub fn main(){std::cmp::Ordering::Less.then_with(||std::cmp::Ordering::Equal)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"refusal/min"#####,
		r#####"pub fn main(){1.min(2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"refusal/max"#####,
		r#####"pub fn main(){1.max(2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"refusal/sort"#####,
		r#####"pub fn main(){let a=[2,1];a.sort();a}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"refusal/sort-by"#####,
		r#####"pub fn main(){let a=[2,1];a.sort_by(|a,b|a.cmp(b));a}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"refusal/iter-any"#####,
		r#####"pub fn main(){[1,2].iter().any(|v|v==1)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"refusal/cmp-function"#####,
		r#####"pub fn main(){let f=std::ops::cmp;f(1,2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"host-mismatch/unit"#####,
		r#####"pub fn main(){c9::id_unit(std::cmp::Ordering::Less)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-host"#####,
		r#####"Exact"#####,
	),
	(
		r#####"host-mismatch/bool"#####,
		r#####"pub fn main(){c9::id_bool(std::cmp::Ordering::Less)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-host"#####,
		r#####"Exact"#####,
	),
	(
		r#####"host-mismatch/i64"#####,
		r#####"pub fn main(){c9::id_i64(std::cmp::Ordering::Less)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-host"#####,
		r#####"Exact"#####,
	),
	(
		r#####"host-mismatch/u64"#####,
		r#####"pub fn main(){c9::id_u64(std::cmp::Ordering::Less)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-host"#####,
		r#####"Exact"#####,
	),
	(
		r#####"host-mismatch/f64"#####,
		r#####"pub fn main(){c9::id_f64(std::cmp::Ordering::Less)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-host"#####,
		r#####"Exact"#####,
	),
	(
		r#####"host-mismatch/char"#####,
		r#####"pub fn main(){c9::id_char(std::cmp::Ordering::Less)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-host"#####,
		r#####"Exact"#####,
	),
	(
		r#####"host-mismatch/string"#####,
		r#####"pub fn main(){c9::id_string(std::cmp::Ordering::Less)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-host"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Less/bare"#####,
		r#####"use std::cmp::Ordering::Less;pub fn main(){Less}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Equal/bare"#####,
		r#####"use std::cmp::Ordering::Equal;pub fn main(){Equal}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"constructor/Greater/bare"#####,
		r#####"use std::cmp::Ordering::Greater;pub fn main(){Greater}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"corpus/by-total/total"#####,
		r#####"fn by_total(a,b){if a[1]!=b[1]{b[1].cmp(a[1])}else{a[0].cmp(b[0])}}pub fn main(){by_total(("a",100),("b",200))}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"corpus/by-total/tie"#####,
		r#####"fn by_total(a,b){if a[1]!=b[1]{b[1].cmp(a[1])}else{a[0].cmp(b[0])}}pub fn main(){by_total(("b",100),("a",100))}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"projection/vec"#####,
		r#####"pub fn main(){[std::cmp::Ordering::Less,std::cmp::Ordering::Equal,std::cmp::Ordering::Greater]}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"projection/option"#####,
		r#####"pub fn main(){Some(std::cmp::Ordering::Less)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"projection/result"#####,
		r#####"pub fn main(){Ok(std::cmp::Ordering::Greater)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"projection/tuple"#####,
		r#####"pub fn main(){(std::cmp::Ordering::Less,std::cmp::Ordering::Equal,std::cmp::Ordering::Greater)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Exact"#####,
	),
	(
		r#####"refusal/direct-ops-cmp"#####,
		r#####"pub fn main(){std::ops::cmp(1,2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"refusal/direct-provider-cmp"#####,
		r#####"pub fn main(){std::i64::cmp(1,2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"refusal/custom-script-cmp"#####,
		r#####"struct S{x}impl S{fn cmp(self,rhs){99}}pub fn main(){S{x:1}.cmp(2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"shadow/other-receiver"#####,
		r#####"struct S{x}impl S{fn cmp(self,r){99}}pub fn main(){1.cmp(2)}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"shadow/untyped"#####,
		r#####"struct S{x}impl S{fn cmp(self,r){99}}fn f(a,b){a.cmp(b)}pub fn main(){(f(1,2),f(S{x:1},0))}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
	(
		r#####"shadow/unused-method"#####,
		r#####"struct S{x}impl S{fn cmp(self,r){99}}pub fn main(){"a".cmp("b")}
pub fn c9_reuse(){7}"#####,
		r#####"c9-main"#####,
		r#####"Refused-surface"#####,
	),
];
