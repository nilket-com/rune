use std::process::Command;
fn main() {
	println!("cargo:rerun-if-changed=src");
	println!("cargo:rerun-if-changed=manifests");
	println!("cargo:rerun-if-changed=Cargo.toml");
	println!("cargo:rerun-if-changed=../../.git/HEAD");
	println!("cargo:rerun-if-changed=../../.git/refs/heads/main");
	let output = Command::new("git")
		.args(["rev-parse", "HEAD"])
		.output()
		.expect("git revision");
	assert!(
		output.status.success(),
		"xtask must be built in a Git checkout"
	);
	let revision = String::from_utf8(output.stdout).expect("UTF-8 revision");
	println!("cargo:rustc-env=XTASK_REVISION={}", revision.trim());
	let output = Command::new("git")
		.args(["status", "--porcelain"])
		.output()
		.expect("git status");
	assert!(output.status.success(), "git status");
	println!("cargo:rustc-env=XTASK_DIRTY={}", !output.stdout.is_empty());
}
