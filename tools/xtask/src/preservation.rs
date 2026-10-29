//! Live comparison of the checked-in preservation corpus.
mod inputs;
mod live;
mod metering;

use crate::common::{self, Context, Result};
use serde_json::{json, Value};

pub fn check_l1_manifest(ctx: &Context) -> Result<Value> {
	Ok(json!({"input_modes": inputs::verify(&ctx.repo)?}))
}

pub fn run(ctx: &Context) -> Result<Value> {
	let semantic = live::run(ctx)?;
	let mut native_ctx = ctx.clone();
	native_ctx.out = ctx.out.join("semantic-native");
	std::fs::create_dir(&native_ctx.out)?;
	let features = format!("{},native-code", common::EXECUTION_FEATURES);
	let binary = ctx.build_rune("metering-native", &features)?;
	let native_semantic = live::run_with_binary(&native_ctx, &binary)?;
	let metering = metering::run(ctx)?;
	Ok(json!({"semantic":semantic,"native_semantic":native_semantic,"metering":metering}))
}
