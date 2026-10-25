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
fn legacy_probe() {
	probe(cases(), COUNT);
}
#[test]
#[ignore = "C4 stock replay: pinned GAP_FROZEN, explicit peer GO and new GAP_OUT"]
fn legacy_replay() {
	let frozen =
		fs::read_to_string(std::env::var_os("GAP_FROZEN").expect("pinned frozen TSV required"))
			.unwrap();
	replay(&frozen);
}

#[test]
#[ignore = "C4 exact frozen stock contract; bounded children, new GAP_OUT"]
fn legacy_contract() {
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

// Expectations are label-exact; no wildcard nominal/refusal exemption.
fn refusal(label: &str) -> Option<&'static str> {
	match label {
		"c4/shape/Ok/Ok()" | "c4/shape/Err/Err()" => Some("Some(FrontendDiagnostic)"),
		"c4/shape/Ok/Ok(x,y)" | "c4/shape/Err/Err(x,y)" => Some("Some(Match)"),
		"c4/let/deferred/Ok/match"
		| "c4/let/deferred/Ok/mismatch"
		| "c4/let/deferred/Err/match"
		| "c4/let/deferred/Err/mismatch" => Some("Some(Pattern)"),
		"c4/nominal/shadow-type" | "c4/nominal/user-enum" | "c4/nominal/tuple-struct" => {
			Some("Some(Match)")
		}
		_ => None,
	}
}
#[test]
#[ignore = "C4 execution supplement contract; bounded children, new GAP_OUT"]
fn contract() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT"));
	fs::create_dir(&out).unwrap();
	let scratch = Scratch(
		std::env::temp_dir().join(format!("rune-c4-execution-contract-{}", std::process::id())),
	);
	fs::create_dir(&scratch.0).unwrap();
	let frozen: Vec<_> = include_str!("../../result_patterns_stock.tsv")
		.lines()
		.skip(1)
		.collect();
	assert_eq!(frozen.len(), COUNT);
	let (mut exact, mut refused, mut full_budget) = (0, 0, 0);
	let mut presentation = 0;
	for (n, ((label, source, mode), row)) in CASES.iter().zip(frozen).enumerate() {
		let row: Vec<_> = row.split('\t').collect();
		assert_eq!(row.len(), 9);
		assert_eq!(row[0], *label);
		assert_eq!(row[1], *mode);
		assert_eq!(row[2], hex(source.as_bytes()));
		assert_eq!(row[8], "Exact");
		let id = format!("{n:05}");
		let path = out.join(format!("{id}.source.rn"));
		fs::write(&path, source).unwrap();
		let old = scratch.0.join("result.site");
		if old.exists() {
			fs::remove_file(old).unwrap();
		}
		let run = launch(&path, &path, mode, "fresh", &scratch.0, &out, &id);
		assert!(run.complete, "{label}: {}", run.record);
		let actual: Vec<_> = run.record.split('\t').collect();
		assert_eq!(actual.len(), 5);
		let admissions: serde_json::Value =
			serde_json::from_str(include_str!("c15_admissions.json")).unwrap();
		let c15b: serde_json::Value =
			serde_json::from_str(include_str!("c15b_admissions.json")).unwrap();
		let c16e: serde_json::Value =
			serde_json::from_str(include_str!("c16e_admissions.json")).unwrap();
		let c16e_rule = c16e.get(*label);
		let admitted = c16e_rule
			.or_else(|| c15b.get(*label))
			.or_else(|| admissions.get(*label));
		if let Some(rule) = c16e_rule {
			use sha2::{Digest, Sha256};
			assert_eq!(
				rule["stock_row_sha256"],
				hex(&Sha256::digest(row.join("\t").as_bytes()))
			);
			assert_eq!(rule["previous_refusal"], refusal(label).unwrap());
		}
		if let Some(rule) = c15b.get(*label) {
			use sha2::{Digest, Sha256};
			assert_eq!(
				rule["stock_row_sha256"],
				hex(&Sha256::digest(row.join("\t").as_bytes()))
			);
		}
		if let Some(rule) = admitted {
			use sha2::Digest as _;
			assert_eq!(
				rule["source_sha256"],
				hex(&sha2::Sha256::digest(source.as_bytes()))
			);
			assert!(refusal(label).is_some());
		}
		if let Some(class) = refusal(label).filter(|_| admitted.is_none()) {
			assert_eq!((actual[0], actual[1]), ("refused", class), "{label}");
			assert_eq!(run.stdout, b"\nrunning 1 test\n");
			assert!(run.stderr.is_empty());
			refused += 1;
			continue;
		}
		if label.starts_with("c4/fuel/") {
			// Stock N is deliberately not used as new fuel: this is full-budget source behavior.
			let (value, stdout) = match label.split('/').nth(2).unwrap() {
				"guards" => ("i64:7:7", b"\nrunning 1 test\nfirst\nsecond\n".as_slice()),
				"nested" => ("i64:15:15", b"\nrunning 1 test\n".as_slice()),
				"loop" => ("i64:1:1", b"\nrunning 1 test\n".as_slice()),
				_ => panic!("unknown family"),
			};
			assert_eq!(actual[0], "value", "{label}");
			assert_eq!(unhex(actual[4]), value, "{label}");
			assert_eq!(run.stdout, stdout, "{label}");
			assert!(run.stderr.is_empty());
			full_budget += 1;
			continue;
		}
		let expected = unhex(row[3]);
		let expected: Vec<_> = expected.split('\t').collect();
		let expected_class =
			if let Some(rule) = c16e_rule.filter(|r| r["policy"] == "Policy-presentation") {
				assert_eq!(expected[0], "error");
				assert_eq!(expected[1], rule["stock_class"].as_str().unwrap());
				assert_eq!(actual[1], rule["fresh_class"].as_str().unwrap());
				rule["fresh_class"].as_str().unwrap()
			} else {
				if let Some(rule) = c16e_rule {
					assert_eq!(rule["policy"], "Exact");
				}
				expected[1]
			};
		assert_eq!(
			(actual[0], actual[1]),
			(expected[0], expected_class),
			"{label}: class"
		);
		assert_eq!(actual[3..], expected[3..], "{label}: message/value");
		assert_eq!(hex(&run.stdout), row[4], "{label}: stdout");
		assert_eq!(hex(&run.stderr), row[5], "{label}: stderr");
		let site = if actual[0] == "error" {
			fs::read(scratch.0.join("result.site")).unwrap()
		} else {
			Vec::new()
		};
		fs::write(out.join(format!("{id}.execution.site")), &site).unwrap();
		assert_eq!(hex(&site), row[6], "{label}: site");
		if c16e_rule.is_some_and(|r| r["policy"] == "Policy-presentation") {
			presentation += 1;
		} else {
			exact += 1;
		}
	}
	assert_eq!((exact, presentation, refused, full_budget), (183, 2, 4, 75));
	fs::write(out.join("summary.txt"),"183 exact;2 source-pinned policy-presentation;4 explicit refusals;75 full-execution-budget source checks (no numeric stock fuel parity)\n").unwrap();
	contract_completed("fresh", COUNT);
}
#[test]
#[ignore = "C4 both-engine frozen contract; bounded children, new GAP_OUT"]
fn check_contract() {
	let out = PathBuf::from(std::env::var_os("GAP_OUT").expect("new GAP_OUT"));
	fs::create_dir(&out).unwrap();
	for (engine, test) in [("stock", "legacy_contract"), ("fresh", "contract")] {
		let result = Command::new(std::env::current_exe().unwrap())
			.args([
				"--exact",
				&format!("experimental_core::tests::gap_hunt::patterns::result_supplement::{test}"),
				"--ignored",
				"--nocapture",
			])
			.env("GAP_OUT", out.join(engine))
			.output()
			.unwrap();
		fs::write(out.join(format!("{engine}.stdout")), &result.stdout).unwrap();
		fs::write(out.join(format!("{engine}.stderr")), &result.stderr).unwrap();
		assert!(
			result.status.success(),
			"{engine}: {}",
			String::from_utf8_lossy(&result.stdout)
		);
		let text = String::from_utf8_lossy(&result.stdout);
		assert!(text.contains("1 passed; 0 failed"));
		assert!(text.contains(&format!("CONTRACT ENGINE {engine}")));
		assert!(text.contains("CONTRACT ROWS 264"));
		assert!(result.stderr.is_empty());
	}
	contract_completed("both", COUNT);
}

#[test]
#[cfg_attr(
	miri,
	ignore = "Miri: source compilation; pure intrinsic controls cover ownership"
)]
fn emission_uses_result_flag_before_children_and_keeps_option_recipe() {
	use crate::experimental_core::{
		code::{Op, VARIANT_SUCCESS},
		frontend,
	};
	for (pattern, child) in [
		("Ok", false),
		("Ok(..)", false),
		("Ok(x)", true),
		("Ok(x,..)", true),
		("Err", false),
		("Err(..)", false),
		("Err(x)", true),
	] {
		let text = format!("pub fn main(){{match Ok(7){{{pattern}=>11,_=>22}}}}");
		let context = crate::Context::with_default_modules().unwrap();
		let mut sources = crate::Sources::new();
		sources
			.insert(crate::Source::memory(&text).unwrap())
			.unwrap();
		let mut diagnostics = crate::Diagnostics::new();
		let p = frontend::compile(&mut sources, &context, &mut diagnostics, 7).unwrap();
		let f = p
			.code
			.function(p.function(crate::Hash::type_hash(["main"])).unwrap())
			.unwrap();
		let ops: Vec<_> = f
			.instructions()
			.iter()
			.map(|i| f.decode(*i).unwrap())
			.collect();
		let flags: Vec<_> = ops
			.iter()
			.enumerate()
			.filter_map(|(pc, op)| {
				matches!(op,Op::Call{function,..} if *function==VARIANT_SUCCESS).then_some(pc)
			})
			.collect();
		let bare_binding = matches!(pattern, "Ok" | "Err");
		assert_eq!(flags.len(), usize::from(!bare_binding), "{pattern}");
		if !bare_binding {
			assert!(matches!(ops[flags[0] + 1], Op::Branch { .. }));
		}
		let fields: Vec<_> = ops
			.iter()
			.enumerate()
			.filter_map(|(pc, op)| matches!(op, Op::TupleField { .. }).then_some(pc))
			.collect();
		assert_eq!(fields.len(), usize::from(child), "{pattern}");
		if child {
			assert!(fields[0] > flags[0] + 1);
			let Op::TupleField { index, .. } = ops[fields[0]] else {
				unreachable!()
			};
			assert_eq!(ops[..fields[0]].iter().filter(|op| matches!(op, Op::Literal { out, value: crate::experimental_core::value::Value::I64(0) } if *out == index)).count(), 1);
		}
	}
	let context = crate::Context::with_default_modules().unwrap();
	let mut sources = crate::Sources::new();
	sources
		.insert(crate::Source::memory("pub fn main(){match Some(7){Some(x)=>x,_=>22}}").unwrap())
		.unwrap();
	let mut diagnostics = crate::Diagnostics::new();
	let p = frontend::compile(&mut sources, &context, &mut diagnostics, 7).unwrap();
	let f = p
		.code
		.function(p.function(crate::Hash::type_hash(["main"])).unwrap())
		.unwrap();
	assert!(f.instructions().iter().all(
		|i| !matches!(f.decode(*i).unwrap(),Op::Call{function,..} if function==VARIANT_SUCCESS)
	));
}
