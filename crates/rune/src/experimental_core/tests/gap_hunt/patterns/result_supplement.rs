//! C4 tests-first stock Result-pattern supplement. Production is unchanged.
use super::*;
const COUNT: usize = 264;
const CASES: &[(&str, &str, &str)] = &[
	("c4/match/ok-binding/ok-int", "pub fn main(){match Ok(7){Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-binding/err-int", "pub fn main(){match Err(7){Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-binding/ok-unit", "pub fn main(){match Ok(()){Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-binding/err-unit", "pub fn main(){match Err(()){Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-binding/ok-heap", "pub fn main(){match Ok([7]){Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-binding/err-heap", "pub fn main(){match Err([7]){Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-binding/some", "pub fn main(){match Some(7){Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-binding/none", "pub fn main(){match None{Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-binding/scalar", "pub fn main(){match 7{Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-binding/vec", "pub fn main(){match [7]{Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-binding/tuple", "pub fn main(){match (7,){Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/match/err-binding/ok-int", "pub fn main(){match Ok(7){Err(x)=>11,_=>22}}", "c3-main"),
	("c4/match/err-binding/err-int", "pub fn main(){match Err(7){Err(x)=>11,_=>22}}", "c3-main"),
	("c4/match/err-binding/ok-unit", "pub fn main(){match Ok(()){Err(x)=>11,_=>22}}", "c3-main"),
	("c4/match/err-binding/err-unit", "pub fn main(){match Err(()){Err(x)=>11,_=>22}}", "c3-main"),
	("c4/match/err-binding/ok-heap", "pub fn main(){match Ok([7]){Err(x)=>11,_=>22}}", "c3-main"),
	("c4/match/err-binding/err-heap", "pub fn main(){match Err([7]){Err(x)=>11,_=>22}}", "c3-main"),
	("c4/match/err-binding/some", "pub fn main(){match Some(7){Err(x)=>11,_=>22}}", "c3-main"),
	("c4/match/err-binding/none", "pub fn main(){match None{Err(x)=>11,_=>22}}", "c3-main"),
	("c4/match/err-binding/scalar", "pub fn main(){match 7{Err(x)=>11,_=>22}}", "c3-main"),
	("c4/match/err-binding/vec", "pub fn main(){match [7]{Err(x)=>11,_=>22}}", "c3-main"),
	("c4/match/err-binding/tuple", "pub fn main(){match (7,){Err(x)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-ignore/ok-int", "pub fn main(){match Ok(7){Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-ignore/err-int", "pub fn main(){match Err(7){Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-ignore/ok-unit", "pub fn main(){match Ok(()){Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-ignore/err-unit", "pub fn main(){match Err(()){Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-ignore/ok-heap", "pub fn main(){match Ok([7]){Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-ignore/err-heap", "pub fn main(){match Err([7]){Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-ignore/some", "pub fn main(){match Some(7){Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-ignore/none", "pub fn main(){match None{Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-ignore/scalar", "pub fn main(){match 7{Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-ignore/vec", "pub fn main(){match [7]{Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-ignore/tuple", "pub fn main(){match (7,){Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/match/err-ignore/ok-int", "pub fn main(){match Ok(7){Err(_)=>11,_=>22}}", "c3-main"),
	("c4/match/err-ignore/err-int", "pub fn main(){match Err(7){Err(_)=>11,_=>22}}", "c3-main"),
	("c4/match/err-ignore/ok-unit", "pub fn main(){match Ok(()){Err(_)=>11,_=>22}}", "c3-main"),
	("c4/match/err-ignore/err-unit", "pub fn main(){match Err(()){Err(_)=>11,_=>22}}", "c3-main"),
	("c4/match/err-ignore/ok-heap", "pub fn main(){match Ok([7]){Err(_)=>11,_=>22}}", "c3-main"),
	("c4/match/err-ignore/err-heap", "pub fn main(){match Err([7]){Err(_)=>11,_=>22}}", "c3-main"),
	("c4/match/err-ignore/some", "pub fn main(){match Some(7){Err(_)=>11,_=>22}}", "c3-main"),
	("c4/match/err-ignore/none", "pub fn main(){match None{Err(_)=>11,_=>22}}", "c3-main"),
	("c4/match/err-ignore/scalar", "pub fn main(){match 7{Err(_)=>11,_=>22}}", "c3-main"),
	("c4/match/err-ignore/vec", "pub fn main(){match [7]{Err(_)=>11,_=>22}}", "c3-main"),
	("c4/match/err-ignore/tuple", "pub fn main(){match (7,){Err(_)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-literal/ok-int", "pub fn main(){match Ok(7){Ok(7)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-literal/err-int", "pub fn main(){match Err(7){Ok(7)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-literal/ok-unit", "pub fn main(){match Ok(()){Ok(7)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-literal/err-unit", "pub fn main(){match Err(()){Ok(7)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-literal/ok-heap", "pub fn main(){match Ok([7]){Ok(7)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-literal/err-heap", "pub fn main(){match Err([7]){Ok(7)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-literal/some", "pub fn main(){match Some(7){Ok(7)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-literal/none", "pub fn main(){match None{Ok(7)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-literal/scalar", "pub fn main(){match 7{Ok(7)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-literal/vec", "pub fn main(){match [7]{Ok(7)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-literal/tuple", "pub fn main(){match (7,){Ok(7)=>11,_=>22}}", "c3-main"),
	("c4/match/err-literal/ok-int", "pub fn main(){match Ok(7){Err(7)=>11,_=>22}}", "c3-main"),
	("c4/match/err-literal/err-int", "pub fn main(){match Err(7){Err(7)=>11,_=>22}}", "c3-main"),
	("c4/match/err-literal/ok-unit", "pub fn main(){match Ok(()){Err(7)=>11,_=>22}}", "c3-main"),
	("c4/match/err-literal/err-unit", "pub fn main(){match Err(()){Err(7)=>11,_=>22}}", "c3-main"),
	("c4/match/err-literal/ok-heap", "pub fn main(){match Ok([7]){Err(7)=>11,_=>22}}", "c3-main"),
	("c4/match/err-literal/err-heap", "pub fn main(){match Err([7]){Err(7)=>11,_=>22}}", "c3-main"),
	("c4/match/err-literal/some", "pub fn main(){match Some(7){Err(7)=>11,_=>22}}", "c3-main"),
	("c4/match/err-literal/none", "pub fn main(){match None{Err(7)=>11,_=>22}}", "c3-main"),
	("c4/match/err-literal/scalar", "pub fn main(){match 7{Err(7)=>11,_=>22}}", "c3-main"),
	("c4/match/err-literal/vec", "pub fn main(){match [7]{Err(7)=>11,_=>22}}", "c3-main"),
	("c4/match/err-literal/tuple", "pub fn main(){match (7,){Err(7)=>11,_=>22}}", "c3-main"),
	("c4/match/ok-unit/ok-int", "pub fn main(){match Ok(7){Ok(())=>11,_=>22}}", "c3-main"),
	("c4/match/ok-unit/err-int", "pub fn main(){match Err(7){Ok(())=>11,_=>22}}", "c3-main"),
	("c4/match/ok-unit/ok-unit", "pub fn main(){match Ok(()){Ok(())=>11,_=>22}}", "c3-main"),
	("c4/match/ok-unit/err-unit", "pub fn main(){match Err(()){Ok(())=>11,_=>22}}", "c3-main"),
	("c4/match/ok-unit/ok-heap", "pub fn main(){match Ok([7]){Ok(())=>11,_=>22}}", "c3-main"),
	("c4/match/ok-unit/err-heap", "pub fn main(){match Err([7]){Ok(())=>11,_=>22}}", "c3-main"),
	("c4/match/ok-unit/some", "pub fn main(){match Some(7){Ok(())=>11,_=>22}}", "c3-main"),
	("c4/match/ok-unit/none", "pub fn main(){match None{Ok(())=>11,_=>22}}", "c3-main"),
	("c4/match/ok-unit/scalar", "pub fn main(){match 7{Ok(())=>11,_=>22}}", "c3-main"),
	("c4/match/ok-unit/vec", "pub fn main(){match [7]{Ok(())=>11,_=>22}}", "c3-main"),
	("c4/match/ok-unit/tuple", "pub fn main(){match (7,){Ok(())=>11,_=>22}}", "c3-main"),
	("c4/match/err-unit/ok-int", "pub fn main(){match Ok(7){Err(())=>11,_=>22}}", "c3-main"),
	("c4/match/err-unit/err-int", "pub fn main(){match Err(7){Err(())=>11,_=>22}}", "c3-main"),
	("c4/match/err-unit/ok-unit", "pub fn main(){match Ok(()){Err(())=>11,_=>22}}", "c3-main"),
	("c4/match/err-unit/err-unit", "pub fn main(){match Err(()){Err(())=>11,_=>22}}", "c3-main"),
	("c4/match/err-unit/ok-heap", "pub fn main(){match Ok([7]){Err(())=>11,_=>22}}", "c3-main"),
	("c4/match/err-unit/err-heap", "pub fn main(){match Err([7]){Err(())=>11,_=>22}}", "c3-main"),
	("c4/match/err-unit/some", "pub fn main(){match Some(7){Err(())=>11,_=>22}}", "c3-main"),
	("c4/match/err-unit/none", "pub fn main(){match None{Err(())=>11,_=>22}}", "c3-main"),
	("c4/match/err-unit/scalar", "pub fn main(){match 7{Err(())=>11,_=>22}}", "c3-main"),
	("c4/match/err-unit/vec", "pub fn main(){match [7]{Err(())=>11,_=>22}}", "c3-main"),
	("c4/match/err-unit/tuple", "pub fn main(){match (7,){Err(())=>11,_=>22}}", "c3-main"),
	("c4/flow/err-first-ok", "pub fn main(){match Ok(7){Err(x)=>x,Ok(x)=>x,_=>22}}", "c3-main"),
	("c4/flow/err-first-err", "pub fn main(){match Err(7){Err(x)=>x,Ok(x)=>x,_=>22}}", "c3-main"),
	("c4/flow/mixed-option-result", "pub fn main(){match Some(7){Err(x)=>x,Some(x)=>x,Ok(x)=>x,_=>22}}", "c3-main"),
	("c4/flow/mixed-result-option", "pub fn main(){match Ok(7){Some(x)=>x,Err(x)=>x,Ok(x)=>x,_=>22}}", "c3-main"),
	("c4/flow/nested-ok-some", "pub fn main(){match Ok(Some(7)){Ok(Some(x))=>x,_=>22}}", "c3-main"),
	("c4/flow/nested-err-ok", "pub fn main(){match Err(Ok(7)){Err(Ok(x))=>x,_=>22}}", "c3-main"),
	("c4/flow/nested-mismatch", "pub fn main(){match Ok(Err(7)){Ok(Ok(x))=>x,Ok(Err(x))=>x,_=>22}}", "c3-main"),
	("c4/flow/tuple-payload", "pub fn main(){match Ok((7,8)){Ok((x,y))=>x+y,_=>22}}", "c3-main"),
	("c4/flow/vec-payload", "pub fn main(){match Err([7,8]){Err([x,y])=>x+y,_=>22}}", "c3-main"),
	("c4/flow/result-in-tuple", "pub fn main(){match (Ok(7),Err(8)){(Ok(x),Err(y))=>x+y,_=>22}}", "c3-main"),
	("c4/flow/result-in-vec", "pub fn main(){match [Ok(7),Err(8)]{[Ok(x),Err(y)]=>x+y,_=>22}}", "c3-main"),
	("c4/flow/miss-partial-binding", "pub fn main(){match Ok((7,8)){Ok((x,9))=>x,Ok((x,y))=>x+y,_=>22}}", "c3-main"),
	("c4/flow/miss-nested-heap", "pub fn main(){match Ok(([7],[8])){Ok(([x],[9]))=>x,Ok(([x],[y]))=>x+y,_=>22}}", "c3-main"),
	("c4/flow/guard-false", "pub fn main(){match Ok([7]){Ok(x) if {println!(\"first\");false}=>11,Ok(y)=>{println!(\"second\");y[0]},_=>22}}", "c3-main"),
	("c4/flow/guard-mutation", "pub fn main(){let r=Ok([7]);match r{Ok(x) if {x.push(8);false}=>11,Ok(y)=>y.len(),_=>22}}", "c3-main"),
	("c4/flow/guard-true", "pub fn main(){match Err(7){Err(x) if {println!(\"guard\");true}=>x,_=>22}}", "c3-main"),
	("c4/flow/variable-rebind", "pub fn main(){let r=Ok([7]);match r{Ok(x) if {r=Err([8]);false}=>11,Err(y)=>y[0],_=>22}}", "c3-main"),
	("c4/flow/block-snapshot", "pub fn main(){let r=Ok([7]);match {r}{Ok(x) if {r=Err([8]);false}=>11,Ok(y)=>y[0],_=>22}}", "c3-main"),
	("c4/flow/condition-order", "pub fn main(){let a=[];match {a.push(1);Ok(7)}{Err(x)=>x,Ok(x) if {a.push(2);false}=>x,Ok(x)=>{a.push(3);a.len()},_=>22}}", "c3-main"),
	("c4/flow/payload-reference", "pub fn main(){let a=[7];let r=Ok(a);match r{Ok(x)=>{x.push(8);a.len()},_=>22}}", "c3-main"),
	("c4/flow/if-let-ok", "pub fn main(){if let Ok(x)=Ok(7){x}else{22}}", "c3-main"),
	("c4/flow/if-let-err", "pub fn main(){if let Err(x)=Err(7){x}else{22}}", "c3-main"),
	("c4/flow/if-let-miss", "pub fn main(){if let Ok(x)=Err(7){x}else{22}}", "c3-main"),
	("c4/flow/while-let-ok", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;if x<2{r=Ok(x+1);}else{r=Err(7);}}n}", "c3-main"),
	("c4/flow/while-let-err", "pub fn main(){let r=Err(0);let n=0;while let Err(x)=r{n+=1;if x<2{r=Err(x+1);}else{r=Ok(7);}}n}", "c3-main"),
	("c4/flow/while-let-miss", "pub fn main(){let r=Err(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-main"),
	("c4/flow/while-let-break", "pub fn main(){let r=Ok([7]);while let Ok(x)=r{break x[0];}}", "c3-main"),
	("c4/flow/while-let-continue", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{r=Err(x);n+=1;continue;}n}", "c3-main"),
	("c4/flow/if-let-tuple-root", "pub fn main(){let t=(Ok(7),);if let Ok(x)=t.0{x}else{22}}", "c3-main"),
	("c4/access/moved-parent/Ok/Ok(_)/variable", "pub fn main(){let a=Ok([7]);let b=a;drop(a);match b{Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Ok/Ok(_)/block", "pub fn main(){let a=Ok([7]);let b=a;drop(a);match {b}{Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Ok/Err(_)/variable", "pub fn main(){let a=Ok([7]);let b=a;drop(a);match b{Err(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Ok/Err(_)/block", "pub fn main(){let a=Ok([7]);let b=a;drop(a);match {b}{Err(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Ok/Ok(x)/variable", "pub fn main(){let a=Ok([7]);let b=a;drop(a);match b{Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Ok/Ok(x)/block", "pub fn main(){let a=Ok([7]);let b=a;drop(a);match {b}{Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Ok/Err(x)/variable", "pub fn main(){let a=Ok([7]);let b=a;drop(a);match b{Err(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Ok/Err(x)/block", "pub fn main(){let a=Ok([7]);let b=a;drop(a);match {b}{Err(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Ok/Ok([_])/variable", "pub fn main(){let a=Ok([7]);let b=a;drop(a);match b{Ok([_])=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Ok/Ok([_])/block", "pub fn main(){let a=Ok([7]);let b=a;drop(a);match {b}{Ok([_])=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Ok/Err([_])/variable", "pub fn main(){let a=Ok([7]);let b=a;drop(a);match b{Err([_])=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Ok/Err([_])/block", "pub fn main(){let a=Ok([7]);let b=a;drop(a);match {b}{Err([_])=>11,_=>22}}", "c3-main"),
	("c4/access/moved-payload/Ok/Ok(_)", "pub fn main(){let p=[7];let r=Ok(p);drop(p);match r{Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-payload/Ok/Err(_)", "pub fn main(){let p=[7];let r=Ok(p);drop(p);match r{Err(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-payload/Ok/Ok(x)", "pub fn main(){let p=[7];let r=Ok(p);drop(p);match r{Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-payload/Ok/Err(x)", "pub fn main(){let p=[7];let r=Ok(p);drop(p);match r{Err(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-payload/Ok/Ok([_])", "pub fn main(){let p=[7];let r=Ok(p);drop(p);match r{Ok([_])=>11,_=>22}}", "c3-main"),
	("c4/access/moved-payload/Ok/Err([_])", "pub fn main(){let p=[7];let r=Ok(p);drop(p);match r{Err([_])=>11,_=>22}}", "c3-main"),
	("c4/access/moved-unrelated/Ok/Ok(_)", "pub fn main(){let a=[7];let b=a;drop(a);match b{Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-unrelated/Ok/Err(_)", "pub fn main(){let a=[7];let b=a;drop(a);match b{Err(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-unrelated/Ok/Ok(x)", "pub fn main(){let a=[7];let b=a;drop(a);match b{Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-unrelated/Ok/Err(x)", "pub fn main(){let a=[7];let b=a;drop(a);match b{Err(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Err/Ok(_)/variable", "pub fn main(){let a=Err([7]);let b=a;drop(a);match b{Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Err/Ok(_)/block", "pub fn main(){let a=Err([7]);let b=a;drop(a);match {b}{Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Err/Err(_)/variable", "pub fn main(){let a=Err([7]);let b=a;drop(a);match b{Err(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Err/Err(_)/block", "pub fn main(){let a=Err([7]);let b=a;drop(a);match {b}{Err(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Err/Ok(x)/variable", "pub fn main(){let a=Err([7]);let b=a;drop(a);match b{Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Err/Ok(x)/block", "pub fn main(){let a=Err([7]);let b=a;drop(a);match {b}{Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Err/Err(x)/variable", "pub fn main(){let a=Err([7]);let b=a;drop(a);match b{Err(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Err/Err(x)/block", "pub fn main(){let a=Err([7]);let b=a;drop(a);match {b}{Err(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Err/Ok([_])/variable", "pub fn main(){let a=Err([7]);let b=a;drop(a);match b{Ok([_])=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Err/Ok([_])/block", "pub fn main(){let a=Err([7]);let b=a;drop(a);match {b}{Ok([_])=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Err/Err([_])/variable", "pub fn main(){let a=Err([7]);let b=a;drop(a);match b{Err([_])=>11,_=>22}}", "c3-main"),
	("c4/access/moved-parent/Err/Err([_])/block", "pub fn main(){let a=Err([7]);let b=a;drop(a);match {b}{Err([_])=>11,_=>22}}", "c3-main"),
	("c4/access/moved-payload/Err/Ok(_)", "pub fn main(){let p=[7];let r=Err(p);drop(p);match r{Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-payload/Err/Err(_)", "pub fn main(){let p=[7];let r=Err(p);drop(p);match r{Err(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-payload/Err/Ok(x)", "pub fn main(){let p=[7];let r=Err(p);drop(p);match r{Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-payload/Err/Err(x)", "pub fn main(){let p=[7];let r=Err(p);drop(p);match r{Err(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-payload/Err/Ok([_])", "pub fn main(){let p=[7];let r=Err(p);drop(p);match r{Ok([_])=>11,_=>22}}", "c3-main"),
	("c4/access/moved-payload/Err/Err([_])", "pub fn main(){let p=[7];let r=Err(p);drop(p);match r{Err([_])=>11,_=>22}}", "c3-main"),
	("c4/access/moved-unrelated/Err/Ok(_)", "pub fn main(){let a=[7];let b=a;drop(a);match b{Ok(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-unrelated/Err/Err(_)", "pub fn main(){let a=[7];let b=a;drop(a);match b{Err(_)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-unrelated/Err/Ok(x)", "pub fn main(){let a=[7];let b=a;drop(a);match b{Ok(x)=>11,_=>22}}", "c3-main"),
	("c4/access/moved-unrelated/Err/Err(x)", "pub fn main(){let a=[7];let b=a;drop(a);match b{Err(x)=>11,_=>22}}", "c3-main"),
	("c4/access/compound-lease/Ok/wildcard", "pub fn main(){let r=Ok(7);r.0 += match r{Ok(_)=>1,_=>0};r.0}", "c3-main"),
	("c4/access/compound-lease/Ok/binding", "pub fn main(){let r=Ok(7);r.0 += match r{Ok(x)=>x,_=>0};r.0}", "c3-main"),
	("c4/access/compound-lease/Ok/opposite", "pub fn main(){let r=Ok(7);r.0 += match r{Err(_)=>1,_=>0};r.0}", "c3-main"),
	("c4/access/compound-lease/Ok/block-snapshot", "pub fn main(){let r=Ok(7);r.0 += match {r}{Ok(_)=>1,_=>0};r.0}", "c3-main"),
	("c4/access/compound-lease/Err/wildcard", "pub fn main(){let r=Err(7);r.0 += match r{Err(_)=>1,_=>0};r.0}", "c3-main"),
	("c4/access/compound-lease/Err/binding", "pub fn main(){let r=Err(7);r.0 += match r{Err(x)=>x,_=>0};r.0}", "c3-main"),
	("c4/access/compound-lease/Err/opposite", "pub fn main(){let r=Err(7);r.0 += match r{Ok(_)=>1,_=>0};r.0}", "c3-main"),
	("c4/access/compound-lease/Err/block-snapshot", "pub fn main(){let r=Err(7);r.0 += match {r}{Err(_)=>1,_=>0};r.0}", "c3-main"),
	("c4/shape/Ok/Ok(..)", "pub fn main(){match Ok(7){Ok(..)=>11,_=>22}}", "c3-main"),
	("c4/shape/Ok/Ok()", "pub fn main(){match Ok(7){Ok()=>11,_=>22}}", "c3-main"),
	("c4/shape/Ok/Ok(x, ..)", "pub fn main(){match Ok(7){Ok(x, ..)=>11,_=>22}}", "c3-main"),
	("c4/shape/Ok/Ok(x,y)", "pub fn main(){match Ok(7){Ok(x,y)=>11,_=>22}}", "c3-main"),
	("c4/shape/Ok/Ok", "pub fn main(){match Ok(7){Ok=>11,_=>22}}", "c3-main"),
	("c4/let/deferred/Ok/match", "pub fn main(){let Ok(x)=Ok(7);x}", "c3-main"),
	("c4/let/deferred/Ok/mismatch", "pub fn main(){let Ok(x)=Err(7);x}", "c3-main"),
	("c4/shape/Err/Err(..)", "pub fn main(){match Err(7){Err(..)=>11,_=>22}}", "c3-main"),
	("c4/shape/Err/Err()", "pub fn main(){match Err(7){Err()=>11,_=>22}}", "c3-main"),
	("c4/shape/Err/Err(x, ..)", "pub fn main(){match Err(7){Err(x, ..)=>11,_=>22}}", "c3-main"),
	("c4/shape/Err/Err(x,y)", "pub fn main(){match Err(7){Err(x,y)=>11,_=>22}}", "c3-main"),
	("c4/shape/Err/Err", "pub fn main(){match Err(7){Err=>11,_=>22}}", "c3-main"),
	("c4/let/deferred/Err/match", "pub fn main(){let Err(x)=Err(7);x}", "c3-main"),
	("c4/let/deferred/Err/mismatch", "pub fn main(){let Err(x)=Ok(7);x}", "c3-main"),
	("c4/nominal/alias-ok", "use std::result::Result::Ok as Good;pub fn main(){match Good(7){Good(x)=>x,_=>22}}", "c3-main"),
	("c4/nominal/qualified", "pub fn main(){match ::std::result::Result::Err(7){::std::result::Result::Err(x)=>x,_=>22}}", "c3-main"),
	("c4/nominal/shadow-type", "struct Ok(v);pub fn main(){match Ok(7){Ok(x)=>x,_=>22}}", "c3-main"),
	("c4/nominal/user-enum", "enum E{Ok(v),Err(v)}pub fn main(){match E::Ok(7){E::Ok(x)=>x,_=>22}}", "c3-main"),
	("c4/nominal/tuple-struct", "struct P(v);pub fn main(){match P(7){P(x)=>x,_=>22}}", "c3-main"),
	("c4/nominal/object-pattern", "pub fn main(){match Ok(#{x:7}){Ok(#{x})=>x,_=>22}}", "c3-main"),
	("c4/fuel/guards/0", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-0"),
	("c4/fuel/nested/0", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-0"),
	("c4/fuel/loop/0", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-0"),
	("c4/fuel/guards/1", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-1"),
	("c4/fuel/nested/1", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-1"),
	("c4/fuel/loop/1", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-1"),
	("c4/fuel/guards/2", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-2"),
	("c4/fuel/nested/2", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-2"),
	("c4/fuel/loop/2", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-2"),
	("c4/fuel/guards/3", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-3"),
	("c4/fuel/nested/3", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-3"),
	("c4/fuel/loop/3", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-3"),
	("c4/fuel/guards/4", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-4"),
	("c4/fuel/nested/4", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-4"),
	("c4/fuel/loop/4", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-4"),
	("c4/fuel/guards/5", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-5"),
	("c4/fuel/nested/5", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-5"),
	("c4/fuel/loop/5", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-5"),
	("c4/fuel/guards/6", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-6"),
	("c4/fuel/nested/6", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-6"),
	("c4/fuel/loop/6", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-6"),
	("c4/fuel/guards/7", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-7"),
	("c4/fuel/nested/7", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-7"),
	("c4/fuel/loop/7", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-7"),
	("c4/fuel/guards/8", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-8"),
	("c4/fuel/nested/8", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-8"),
	("c4/fuel/loop/8", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-8"),
	("c4/fuel/guards/9", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-9"),
	("c4/fuel/nested/9", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-9"),
	("c4/fuel/loop/9", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-9"),
	("c4/fuel/guards/10", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-10"),
	("c4/fuel/nested/10", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-10"),
	("c4/fuel/loop/10", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-10"),
	("c4/fuel/guards/11", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-11"),
	("c4/fuel/nested/11", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-11"),
	("c4/fuel/loop/11", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-11"),
	("c4/fuel/guards/12", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-12"),
	("c4/fuel/nested/12", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-12"),
	("c4/fuel/loop/12", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-12"),
	("c4/fuel/guards/13", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-13"),
	("c4/fuel/nested/13", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-13"),
	("c4/fuel/loop/13", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-13"),
	("c4/fuel/guards/14", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-14"),
	("c4/fuel/nested/14", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-14"),
	("c4/fuel/loop/14", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-14"),
	("c4/fuel/guards/15", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-15"),
	("c4/fuel/nested/15", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-15"),
	("c4/fuel/loop/15", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-15"),
	("c4/fuel/guards/16", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-16"),
	("c4/fuel/nested/16", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-16"),
	("c4/fuel/loop/16", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-16"),
	("c4/fuel/guards/17", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-17"),
	("c4/fuel/nested/17", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-17"),
	("c4/fuel/loop/17", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-17"),
	("c4/fuel/guards/18", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-18"),
	("c4/fuel/nested/18", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-18"),
	("c4/fuel/loop/18", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-18"),
	("c4/fuel/guards/19", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-19"),
	("c4/fuel/nested/19", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-19"),
	("c4/fuel/loop/19", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-19"),
	("c4/fuel/guards/20", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-20"),
	("c4/fuel/nested/20", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-20"),
	("c4/fuel/loop/20", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-20"),
	("c4/fuel/guards/21", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-21"),
	("c4/fuel/nested/21", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-21"),
	("c4/fuel/loop/21", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-21"),
	("c4/fuel/guards/22", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-22"),
	("c4/fuel/nested/22", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-22"),
	("c4/fuel/loop/22", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-22"),
	("c4/fuel/guards/23", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-23"),
	("c4/fuel/nested/23", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-23"),
	("c4/fuel/loop/23", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-23"),
	("c4/fuel/guards/24", "pub fn main(){match Ok(7){Ok(x) if {println!(\"first\");false}=>x,Ok(y)=>{println!(\"second\");y},_=>22}}", "c3-fuel-24"),
	("c4/fuel/nested/24", "pub fn main(){match Err((7,8)){Ok(x)=>11,Err((x,y))=>x+y,_=>22}}", "c3-fuel-24"),
	("c4/fuel/loop/24", "pub fn main(){let r=Ok(0);let n=0;while let Ok(x)=r{n+=1;r=Err(x);}n}", "c3-fuel-24"),
];
fn cases() -> Vec<Case> {
	CASES
		.iter()
		.map(|(label, source, mode)| Case {
			label: (*label).into(),
			source: (*source).into(),
			mode: (*mode).into(),
		})
		.collect()
}
#[test]
fn matrix_labels_are_unique_and_disjoint_from_frozen_c3() {
	assert_eq!(CASES.len(), COUNT);
	let old: std::collections::BTreeSet<_> = include_str!("../../patterns_stock.tsv")
		.lines()
		.skip(1)
		.map(|r| r.split('\t').next().unwrap())
		.collect();
	let mut seen = std::collections::BTreeSet::new();
	for (label, _, mode) in CASES {
		assert!(label.starts_with("c4/"));
		assert!(seen.insert(label));
		assert!(!old.contains(label));
		assert!(*mode == "c3-main" || mode.starts_with("c3-fuel-"));
	}
}
#[test]
#[ignore = "C4 stock supplement source checkpoint: explicit peer GO and new GAP_OUT"]
fn stock_probe() {
	probe(cases(), COUNT);
}
#[test]
#[ignore = "C4 stock replay: pinned GAP_FROZEN, explicit peer GO and new GAP_OUT"]
fn stock_replay() {
	let frozen =
		fs::read_to_string(std::env::var_os("GAP_FROZEN").expect("pinned frozen TSV required"))
			.unwrap();
	replay(&frozen);
}

#[test]
#[ignore = "C4 exact frozen stock contract; bounded children, new GAP_OUT"]
fn stock_contract() {
	replay(include_str!("../../result_patterns_stock.tsv"));
}

fn replay(frozen: &str) {
	let rows: Vec<_> = frozen.lines().skip(1).collect();
	assert_eq!(rows.len(), COUNT);
	for ((label, source, mode), row) in CASES.iter().zip(&rows) {
		let c: Vec<_> = row.split('\t').collect();
		assert_eq!(c.len(), 9);
		assert_eq!(c[0], *label);
		assert_eq!(c[1], *mode);
		assert_eq!(c[2], hex(source.as_bytes()));
	}
	probe(cases(), COUNT);
	let out = PathBuf::from(std::env::var_os("GAP_OUT").unwrap());
	let replay = fs::read_to_string(out.join("stock-patterns.tsv")).unwrap();
	assert_eq!(frozen.lines().next(), replay.lines().next());
	let repeated: Vec<_> = replay.lines().skip(1).collect();
	assert_eq!(repeated.len(), COUNT);
	let mut exact = 0;
	let mut observations = 0;
	for (old, new) in rows.iter().zip(&repeated) {
		let a: Vec<_> = old.split('\t').collect();
		let b: Vec<_> = new.split('\t').collect();
		assert_eq!(a[..3], b[..3]);
		assert_eq!(a[8], b[8]);
		match a[8] {
			"Exact" => {
				assert_eq!(old, new, "exact replay {}", a[0]);
				exact += 1;
			}
			"Observation(address-unmodified)" => {
				observations += 1;
				let x = unhex(a[3]);
				let y = unhex(b[3]);
				assert_eq!(
					x.split('\t').take(2).collect::<Vec<_>>(),
					y.split('\t').take(2).collect::<Vec<_>>()
				);
				assert_eq!(a[6], b[6]);
			}
			_ => panic!("unknown frozen policy"),
		}
	}
	fs::write(out.join("REPLAY.txt"),format!("{exact} exact replays; {observations} raw address observations, phase/class/site only (not exact)\n")).unwrap();
}
