use sha2::{Digest, Sha256};
use std::process::Command;

fn git(args: &[&str]) -> String {
    Command::new("git").args(args).output().ok().filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default()
}
fn main() {
    let channel = std::fs::read_to_string(".channel").unwrap_or_default();
    let title = match channel.trim() { "alpha" => "Plexi Alpha", "beta" => "Plexi Beta", _ => "Plexi" };
    println!("cargo:rustc-env=PLEXI_APP_TITLE={title}");
    println!("cargo:rerun-if-changed=.channel");
    println!("cargo:rerun-if-env-changed=PLEXI_BUILD_TAG");
    let commit = git(&["rev-parse", "HEAD"]);
    for name in ["HEAD", &git(&["symbolic-ref", "-q", "HEAD"])] {
        if !name.is_empty() { println!("cargo:rerun-if-changed={}", git(&["rev-parse", "--git-path", name])); }
    }
    let tag = std::env::var("PLEXI_BUILD_TAG").unwrap_or_else(|_| git(&["describe", "--tags", "--exact-match", "HEAD"]));
    let mut hash = Sha256::new();
    // Include unstaged and newly added sources: two dogfood builds from the
    // same commit must not claim to contain the same SDK or core apps.
    let output = Command::new("git").args(["ls-files", "--cached", "--others", "--exclude-standard", "-z"]).output().expect("git ls-files for build identity");
    assert!(output.status.success(), "git ls-files for build identity failed");
    let mut files: Vec<_> = output.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()).collect();
    files.sort_unstable(); files.dedup();
    for file in files {
        let path = std::str::from_utf8(file).expect("UTF8 source path");
        println!("cargo:rerun-if-changed={path}");
        hash.update(file); hash.update([0]);
        if let Ok(bytes) = std::fs::read(path) { hash.update(bytes); }
        hash.update([0]);
    }
    let build_id = format!("{commit}-{:x}", hash.finalize());
    println!("cargo:rustc-env=PLEXI_SOURCE_COMMIT={commit}");
    println!("cargo:rustc-env=PLEXI_BUILD_ID={build_id}");
    println!("cargo:rustc-env=PLEXI_BUILD_TAG={tag}");
}
