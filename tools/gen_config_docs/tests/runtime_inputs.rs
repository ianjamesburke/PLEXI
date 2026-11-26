use std::process::{Command, Output};

#[expect(
    clippy::disallowed_methods,
    reason = "integration test: run the Cargo-built generator with a different working directory"
)]
fn generate_from(directory: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gen_config_docs"))
        .current_dir(directory)
        .output()
        .unwrap_or_else(|error| panic!("run gen_config_docs in {directory}: {error}"))
}

#[test]
fn generates_docs_from_the_invocation_checkout() {
    let output = generate_from("tests/fixtures/other-worktree");

    assert!(
        output.status.success(),
        "generator failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let docs = String::from_utf8(output.stdout).expect("generator emits UTF-8 Markdown");
    assert!(docs.contains("| `worktree_setting` |"));
    assert!(docs.contains("worktree_setting = true"));
}

#[test]
fn missing_input_points_to_the_workspace_root() {
    let output = generate_from("tests");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("src/config/mod.rs"), "{error}");
    assert!(error.contains("Plexi workspace root"), "{error}");
}
