use std::collections::HashSet;

use super::super::static_capability_check;

/// No declared capabilities, introspect script reports none required — should pass.
#[test]
fn static_capability_check_returns_ok_when_no_required_caps() {
    // Build a tiny Python script that mimics what --plexi-introspect produces:
    // prints an empty required_capabilities list and exits 0.
    let dir = tempfile::tempdir().expect("tempdir");
    let script_path = dir.path().join("app.py");
    std::fs::write(
        &script_path,
        r#"import sys, json
if "--plexi-introspect" in sys.argv:
    print(json.dumps({"required_capabilities": []}))
    sys.exit(0)
"#,
    )
    .expect("write script");

    let py_exe = std::ffi::OsString::from("python3");
    let declared: HashSet<crate::app_permissions::Capability> = HashSet::new();

    let result = static_capability_check(
        "test_app",
        &script_path,
        py_exe.as_ref(),
        "",
        &std::env::var("PATH").unwrap_or_default(),
        &declared,
    );
    assert!(result.is_ok(), "expected Ok(()), got {result:?}");
}

/// Introspect reports a capability that is NOT declared — should return Err.
#[test]
fn static_capability_check_fails_on_missing_declaration() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script_path = dir.path().join("app.py");
    std::fs::write(
        &script_path,
        r#"import sys, json
if "--plexi-introspect" in sys.argv:
    print(json.dumps({"required_capabilities": ["net.http"]}))
    sys.exit(0)
"#,
    )
    .expect("write script");

    let py_exe = std::ffi::OsString::from("python3");
    let declared: HashSet<crate::app_permissions::Capability> = HashSet::new();

    let result = static_capability_check(
        "test_app",
        &script_path,
        py_exe.as_ref(),
        "",
        &std::env::var("PATH").unwrap_or_default(),
        &declared,
    );
    assert!(result.is_err(), "expected Err, got Ok");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("net.http"),
        "error message should mention missing capability, got: {msg}"
    );
}

/// Introspect subprocess fails (bad script) — should return Ok (infra failure = skip).
#[test]
fn static_capability_check_skips_on_subprocess_failure() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script_path = dir.path().join("app.py");
    // Script exits non-zero when --plexi-introspect is passed.
    std::fs::write(
        &script_path,
        r#"import sys
if "--plexi-introspect" in sys.argv:
    sys.exit(1)
"#,
    )
    .expect("write script");

    let py_exe = std::ffi::OsString::from("python3");
    let declared: HashSet<crate::app_permissions::Capability> = HashSet::new();

    let result = static_capability_check(
        "test_app",
        &script_path,
        py_exe.as_ref(),
        "",
        &std::env::var("PATH").unwrap_or_default(),
        &declared,
    );
    assert!(
        result.is_ok(),
        "subprocess failure should be skipped, not propagated; got {result:?}"
    );
}
