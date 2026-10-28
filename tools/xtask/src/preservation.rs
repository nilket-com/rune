//! Live comparison of the checked-in preservation corpus.
mod inputs;
mod live;

use crate::common::{Context, Result};
use serde_json::{json, Value};

pub fn check_l1_manifest(ctx: &Context) -> Result<Value> {
	Ok(json!({"input_modes": inputs::verify(&ctx.repo)?}))
}

pub fn run(ctx: &Context) -> Result<Value> {
	live::run(ctx)
}
