const OBSERVER_NODES: usize = 4096;
struct RuntimeGraph { memo: std::collections::HashMap<(u8, usize), usize>, pending: Vec<crate::runtime::Value> }
fn identity(value: &crate::runtime::Value) -> Option<(u8, usize)> {
	macro_rules! key { ($tag:expr,$ty:ty) => { if let Ok(v) = value.borrow_ref::<$ty>() { return Some(($tag, (&*v as *const $ty) as usize)); } }; }
	key!(0, crate::alloc::String); key!(1, crate::runtime::Vec); key!(2, crate::runtime::OwnedTuple); key!(3, crate::runtime::Object);
	key!(4, Option<crate::runtime::Value>); key!(5, Result<crate::runtime::Value, crate::runtime::Value>);
	None
}
impl RuntimeGraph {
	fn intern(&mut self, value: crate::runtime::Value) -> usize {
		if let Some(id) = identity(&value).and_then(|k| self.memo.get(&k).copied()) { return id; }
		assert!(self.pending.len() < OBSERVER_NODES, "C11b observation graph bound");
		let id = self.pending.len(); if let Some(key) = identity(&value) { self.memo.insert(key, id); }
		self.pending.push(value); id
	}
	fn node(&mut self, value: &crate::runtime::Value) -> Json {
		use crate::runtime::{Inline, Repr}; let hash = value.type_hash().to_string();
		if let Some(v) = value.as_inline() { return match v {
			Inline::Empty => json!(["Empty",hash]), Inline::Unit => json!(["Unit",hash]), Inline::Bool(v) => json!(["Bool",hash,v]),
			Inline::Char(v) => json!(["Char",hash,*v as u32]), Inline::Signed(v) => json!(["I64",hash,v.to_string()]),
			Inline::Unsigned(v) => json!(["U64",hash,v.to_string()]), Inline::Float(v) => json!(["F64",hash,format!("{:016x}",v.to_bits())]),
			Inline::Type(v) => json!(["Type",hash,v.into_hash().to_string()]), Inline::Hash(v) => json!(["Hash",hash,v.to_string()]),
			Inline::Ordering(v) => json!(["Ordering",hash,format!("{v:?}")]),
		}; }
		if let Ok(v) = value.borrow_ref::<crate::alloc::String>() { return json!(["String",hash,hex(v.as_bytes())]); }
		if let Ok(v) = value.borrow_ref::<crate::runtime::Vec>() { return json!(["Vec",hash,v.iter().map(|v|self.intern(v.clone())).collect::<Vec<_>>()]); }
		if let Ok(v) = value.borrow_ref::<crate::runtime::OwnedTuple>() { return json!(["Tuple",hash,v.iter().map(|v|self.intern(v.clone())).collect::<Vec<_>>()]); }
		if let Ok(v) = value.borrow_ref::<crate::runtime::Object>() { return json!(["Object",hash,v.iter().map(|(k,v)|(hex(k.as_bytes()),self.intern(v.clone()))).collect::<Vec<_>>()]); }
		if let Ok(v) = value.borrow_ref::<Option<crate::runtime::Value>>() { return json!(["Option",hash,(*v).as_ref().map(|v|self.intern(v.clone()))]); }
		if let Ok(v) = value.borrow_ref::<Result<crate::runtime::Value,crate::runtime::Value>>() { return match &*v { Ok(v)=>json!(["Result",hash,"Ok",self.intern(v.clone())]), Err(v)=>json!(["Result",hash,"Err",self.intern(v.clone())]) }; }
		match value.as_ref() { Repr::Dynamic(v)=>json!(["Dynamic",hash,format!("{:?}",v.rtti().kind),value.is_readable(),value.is_writable()]), Repr::Any(_)=>json!(["OpaqueAny",hash,value.is_readable(),value.is_writable()]), Repr::Inline(_)=>unreachable!() }
	}
}
fn runtime_graph(value: &crate::runtime::Value) -> Json {
	let mut g=RuntimeGraph{memo:std::collections::HashMap::new(),pending:Vec::new()}; let root=g.intern(value.clone()); let mut nodes=Vec::new(); let mut at=0;
	while at<g.pending.len(){let value=g.pending[at].clone();nodes.push(g.node(&value));at+=1;} json!({"root":root,"nodes":nodes})
}
fn hex(bytes:&[u8])->String{bytes.iter().map(|b|format!("{b:02x}")).collect()}
