//! `plexi plugin list --json` prints JSON, not the human table (stint 1488).
//! Agents pass `--json` and parse the result; text on stdout breaks them in a
//! way that looks like success.

use std::process::{Command, Stdio};

#[test]
fn plugin_list_json_prints_a_json_array() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_plexi"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PLEXI_") {
            command.env_remove(key);
        }
    }
    let output = command
        .args(["plugin", "list", "--json"])
        .current_dir(cwd.path())
        .env("HOME", home.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|error| panic!("not JSON ({error}): {stdout}"));
    assert!(parsed.is_array(), "{parsed}");
}
