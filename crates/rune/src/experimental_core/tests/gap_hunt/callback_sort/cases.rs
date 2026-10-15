// C11 source-only catalogue; policies provisional until stock freeze.
const COUNT: usize = 131;
#[allow(clippy::type_complexity)]
const CASES: &[(&str, &str, &str, &str, &str, usize, Option<usize>)] = &[
	(
		"success/named/0/ascending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/ascending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/0/descending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/0/duplicates",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/duplicates",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/1/ascending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/ascending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/1/descending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/1/duplicates",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/duplicates",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/2/ascending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/ascending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/2/descending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/2/duplicates",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/duplicates",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/3/ascending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/3/ascending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/3/descending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/3/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/3/duplicates",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/3/duplicates",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/4/ascending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/4/ascending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/4/descending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/4/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/4/duplicates",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/4/duplicates",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/5/ascending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/5/ascending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/5/descending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/5/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/5/duplicates",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/5/duplicates",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/8/ascending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/ascending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/8/descending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/8/duplicates",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/duplicates",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/17/ascending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/17/ascending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/17/descending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/17/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/17/duplicates",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/17/duplicates",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/33/ascending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/33/ascending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/33/descending",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/33/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/named/33/duplicates",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/33/duplicates",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/noncapturing/0",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(|a,b|{c11::tick(a,b);a.cmp(b)});v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/noncapturing/1",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(|a,b|{c11::tick(a,b);a.cmp(b)});v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/noncapturing/2",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(|a,b|{c11::tick(a,b);a.cmp(b)});v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/noncapturing/5",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(|a,b|{c11::tick(a,b);a.cmp(b)});v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/5/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/noncapturing/17",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(|a,b|{c11::tick(a,b);a.cmp(b)});v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/17/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"success/noncapturing/33",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(|a,b|{c11::tick(a,b);a.cmp(b)});v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/33/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"schedule/equal",
		r########"fn compare(a,b){c11::tick(a,b);std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/64/duplicates",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"schedule/less",
		r########"fn compare(a,b){c11::tick(a,b);std::cmp::Ordering::Less}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/64/duplicates",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"schedule/greater",
		r########"fn compare(a,b){c11::tick(a,b);std::cmp::Ordering::Greater}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/64/duplicates",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"schedule/inconsistent",
		r########"fn compare(a,b){let n=c11::tick(a,b);if n%3==0{std::cmp::Ordering::Less}else if n%3==1{std::cmp::Ordering::Greater}else{std::cmp::Ordering::Equal}}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/64/duplicates",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"stability/key-and-id",
		r########"fn compare(a,b){c11::tick(a[1],b[1]);a[0].cmp(b[0])}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"tagged",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"alias/independent-child-mutation",
		r########"fn compare(a,b){c11::tick(a[0],b[0]);a[1].push(9);b[1].push(8);a[0].cmp(b[0])}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"children-distinct",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"alias/shared-child-mutation",
		r########"fn compare(a,b){c11::tick(a[0],b[0]);a[1].push(9);b[1].push(8);a[0].cmp(b[0])}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"children-shared",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"failure/first-call-failure",
		r########"fn compare(a,b){let n=c11::tick(a,b);if n==1{a/(b-b);};a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"failure/later-failure",
		r########"fn compare(a,b){let n=c11::tick(a,b);if n==3{a/(b-b);};a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"failure/all-failing",
		r########"fn compare(a,b){c11::tick(a,b);a/(b-b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"failure/non-ordering-return",
		r########"fn compare(a,b){c11::tick(a,b);7}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"failure/panic",
		r########"fn compare(a,b){c11::tick(a,b);panic!("c11 comparator")}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"return/0/()",
		r########"fn compare(a,b){c11::tick(a,b);()}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"return/1/()",
		r########"fn compare(a,b){c11::tick(a,b);()}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"return/2/()",
		r########"fn compare(a,b){c11::tick(a,b);()}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"return/0/true",
		r########"fn compare(a,b){c11::tick(a,b);true}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"return/1/true",
		r########"fn compare(a,b){c11::tick(a,b);true}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"return/2/true",
		r########"fn compare(a,b){c11::tick(a,b);true}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"return/0/\"x\"",
		r########"fn compare(a,b){c11::tick(a,b);"x"}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"return/1/\"x\"",
		r########"fn compare(a,b){c11::tick(a,b);"x"}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"return/2/\"x\"",
		r########"fn compare(a,b){c11::tick(a,b);"x"}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"return/0/(1,)",
		r########"fn compare(a,b){c11::tick(a,b);(1,)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"return/1/(1,)",
		r########"fn compare(a,b){c11::tick(a,b);(1,)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"return/2/(1,)",
		r########"fn compare(a,b){c11::tick(a,b);(1,)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"return/0/Some(1)",
		r########"fn compare(a,b){c11::tick(a,b);Some(1)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"return/1/Some(1)",
		r########"fn compare(a,b){c11::tick(a,b);Some(1)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"return/2/Some(1)",
		r########"fn compare(a,b){c11::tick(a,b);Some(1)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"return/0/Ok(1)",
		r########"fn compare(a,b){c11::tick(a,b);Ok(1)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"return/1/Ok(1)",
		r########"fn compare(a,b){c11::tick(a,b);Ok(1)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"return/2/Ok(1)",
		r########"fn compare(a,b){c11::tick(a,b);Ok(1)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"arity/zero/0",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(wrong);v}
fn wrong(){std::cmp::Ordering::Equal}
pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"arity/zero/1",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(wrong);v}
fn wrong(){std::cmp::Ordering::Equal}
pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"arity/zero/2",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(wrong);v}
fn wrong(){std::cmp::Ordering::Equal}
pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"arity/one/0",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(wrong);v}
fn wrong(a){std::cmp::Ordering::Equal}
pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"arity/one/1",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(wrong);v}
fn wrong(a){std::cmp::Ordering::Equal}
pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"arity/one/2",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(wrong);v}
fn wrong(a){std::cmp::Ordering::Equal}
pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"arity/three/0",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(wrong);v}
fn wrong(a,b,c){std::cmp::Ordering::Equal}
pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"arity/three/1",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(wrong);v}
fn wrong(a,b,c){std::cmp::Ordering::Equal}
pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"arity/three/2",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(wrong);v}
fn wrong(a,b,c){std::cmp::Ordering::Equal}
pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/descending",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"type/comparator/0",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(7);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/0/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"type/comparator/1",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(7);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/1/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"type/comparator/2",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(7);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/2/descending",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"parent/read-len",
		r########"fn compare(a,b){c11::tick(1,2);a.len();std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"self-cycle",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"parent/read-clone",
		r########"fn compare(a,b){c11::tick(1,2);let x=a.clone();std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"self-cycle",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"parent/read-index",
		r########"fn compare(a,b){c11::tick(1,2);let x=a[0];std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"self-cycle",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"parent/read-format",
		r########"fn compare(a,b){c11::tick(1,2);let x=format!("{:?}",a);std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"self-cycle",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"parent/read-iterate",
		r########"fn compare(a,b){c11::tick(1,2);for x in a{break;}std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"self-cycle",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"parent/write-push",
		r########"fn compare(a,b){c11::tick(1,2);a.push(3);std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"self-cycle",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"parent/write-index",
		r########"fn compare(a,b){c11::tick(1,2);a[0]=3;std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"self-cycle",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"parent/take-drop",
		r########"fn compare(a,b){c11::tick(1,2);drop(a);std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"self-cycle",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"parent/nested-same",
		r########"fn compare(a,b){c11::tick(1,2);a.sort_by(compare);std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"self-cycle",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"parent/inert-alias",
		r########"fn compare(a,b){let x=a;c11::tick(1,2);std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"self-cycle",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"parent/noncapturing-closure-read",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(|a,b|{c11::tick(1,2);a.len();std::cmp::Ordering::Equal});v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"self-cycle",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"nested/different",
		r########"fn compare(a,b){c11::tick(a,b);let w=[3,1,2];w.sort_by(inner);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}
fn inner(a,b){c11::mark("inner");a.cmp(b)}
pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"effects/receiver-then-comparator",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){{c11::mark("receiver");v}.sort_by({c11::mark("comparator");compare});v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"effects/ignored-result",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);7}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"frames/caller-chain",
		r########"fn compare(a,b){c11::tick(a,b);panic!("inner failure")}
pub fn main(v,state){outer(v);v}
fn inner(v){v.sort_by(compare);} fn outer(v){inner(v);}
pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"refused/captures-parent",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(|a,b|{v.len();a.cmp(b)});v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Refused-surface",
		"args",
		2000000,
		None,
	),
	(
		"refused/captures-scalar",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(|a,b|{let x=state;a.cmp(b)});v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Refused-surface",
		"args",
		2000000,
		None,
	),
	(
		"refused/move-capture",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(move |a,b|{let x=state;a.cmp(b)});v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Refused-surface",
		"args",
		2000000,
		None,
	),
	(
		"refused/captured-after-sort",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);let n=7;let f=|x|x+n;f(1)}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Refused-surface",
		"args",
		2000000,
		None,
	),
	(
		"refused/any",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.iter().any(|x|x==1)}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Refused-surface",
		"args",
		2000000,
		None,
	),
	(
		"refused/async-comparator",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(async_cmp);v}
async fn async_cmp(a,b){a.cmp(b)}
pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Refused-surface",
		"args",
		2000000,
		None,
	),
	(
		"failure/child-mutation",
		r########"fn compare(a,b){c11::tick(a[0],b[0]);a[1].push(9);a[0]/(b[0]-b[0])}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"children-distinct",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"failure/shared-child-mutation",
		r########"fn compare(a,b){c11::tick(a[0],b[0]);a[1].push(9);a[0]/(b[0]-b[0])}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"children-shared",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"nested/different-error",
		r########"fn compare(a,b){c11::tick(a,b);let w=[3,1,2];w.sort_by(inner);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}
fn inner(a,b){c11::mark("inner");panic!("nested")}
pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Intentional-divergence",
		"args",
		2000000,
		None,
	),
	(
		"type/non-vector",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"non-vector",
		"Refused-runtime",
		"args",
		2000000,
		None,
	),
	(
		"depth/8",
		r########"fn compare(a,b){c11::tick(a,b);if a>0{let w=[a-1,a-1];w.sort_by(compare);};std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"depth/8",
		"Retained-depth",
		"args",
		2000000,
		None,
	),
	(
		"depth/31",
		r########"fn compare(a,b){c11::tick(a,b);if a>0{let w=[a-1,a-1];w.sort_by(compare);};std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"depth/31",
		"Retained-depth",
		"args",
		2000000,
		None,
	),
	(
		"depth/32",
		r########"fn compare(a,b){c11::tick(a,b);if a>0{let w=[a-1,a-1];w.sort_by(compare);};std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"depth/32",
		"Retained-depth",
		"args",
		2000000,
		None,
	),
	(
		"depth/33",
		r########"fn compare(a,b){c11::tick(a,b);if a>0{let w=[a-1,a-1];w.sort_by(compare);};std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"depth/33",
		"Retained-depth",
		"args",
		2000000,
		None,
	),
	(
		"depth/127",
		r########"fn compare(a,b){c11::tick(a,b);if a>0{let w=[a-1,a-1];w.sort_by(compare);};std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"depth/127",
		"Retained-depth",
		"args",
		2000000,
		None,
	),
	(
		"depth/510",
		r########"fn compare(a,b){c11::tick(a,b);if a>0{let w=[a-1,a-1];w.sort_by(compare);};std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"depth/510",
		"Retained-depth",
		"args",
		2000000,
		None,
	),
	(
		"depth/511",
		r########"fn compare(a,b){c11::tick(a,b);if a>0{let w=[a-1,a-1];w.sort_by(compare);};std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"depth/511",
		"Retained-depth",
		"args",
		2000000,
		None,
	),
	(
		"depth/512",
		r########"fn compare(a,b){c11::tick(a,b);if a>0{let w=[a-1,a-1];w.sort_by(compare);};std::cmp::Ordering::Equal}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"depth/512",
		"Retained-depth",
		"args",
		2000000,
		None,
	),
	(
		"fuel/0",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-fuel",
		"args",
		0,
		None,
	),
	(
		"fuel/1",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-fuel",
		"args",
		1,
		None,
	),
	(
		"fuel/2",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-fuel",
		"args",
		2,
		None,
	),
	(
		"fuel/8",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-fuel",
		"args",
		8,
		None,
	),
	(
		"fuel/16",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-fuel",
		"args",
		16,
		None,
	),
	(
		"fuel/32",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-fuel",
		"args",
		32,
		None,
	),
	(
		"fuel/64",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-fuel",
		"args",
		64,
		None,
	),
	(
		"fuel/128",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-fuel",
		"args",
		128,
		None,
	),
	(
		"fuel/256",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-fuel",
		"args",
		256,
		None,
	),
	(
		"fuel/1024",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-fuel",
		"args",
		1024,
		None,
	),
	(
		"cancel/0",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-cancel",
		"args",
		2000000,
		Some(0),
	),
	(
		"cancel/1",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-cancel",
		"args",
		2000000,
		Some(1),
	),
	(
		"cancel/2",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-cancel",
		"args",
		2000000,
		Some(2),
	),
	(
		"cancel/8",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-cancel",
		"args",
		2000000,
		Some(8),
	),
	(
		"cancel/16",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-cancel",
		"args",
		2000000,
		Some(16),
	),
	(
		"cancel/32",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-cancel",
		"args",
		2000000,
		Some(32),
	),
	(
		"cancel/64",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){v.sort_by(compare);v}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints/8/descending",
		"Retained-cancel",
		"args",
		2000000,
		Some(64),
	),
	(
		"function/noncapturing/empty-args",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){let f=||42;f()}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"function/noncapturing/one-arg",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){let f=|a|a+1;f(6)}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"function/noncapturing/ordering-args",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){let f=|a,b|a.cmp(b);f(1,2)}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"function/noncapturing/nested",
		r########"fn compare(a,b){c11::tick(a,b);a.cmp(b)}
pub fn main(v,state){let f=||{let g=||7;g()};f()}

pub fn c11_reuse(){c11::mark("reuse");7}"########,
		"ints",
		"Exact",
		"args",
		2000000,
		None,
	),
	(
		"old-c9/refusal/sort-by",
		r########"pub fn main(){let a=[2,1];a.sort_by(|a,b|a.cmp(b));a}
pub fn c9_reuse(){7}"########,
		"ints",
		"Exact",
		"c9-noargs",
		2000000,
		None,
	),
	(
		"old-c9/refusal/iter-any",
		r########"pub fn main(){[1,2].iter().any(|v|v==1)}
pub fn c9_reuse(){7}"########,
		"ints",
		"Refused-surface",
		"c9-noargs",
		2000000,
		None,
	),
];
