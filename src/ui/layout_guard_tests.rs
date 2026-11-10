#[test]
fn registered_surfaces_have_fixtures_or_explicit_gaps() {
    let result = std::process::Command::new("python3")
        .arg("scripts/check-ui-layout.py")
        .arg("--test")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run UI surface coverage guard");
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
