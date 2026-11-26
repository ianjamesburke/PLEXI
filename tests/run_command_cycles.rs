#![cfg(unix)]

mod common;
#[path = "../src/testing/timeout.rs"]
mod timeout;

use plexi_distribution::bounded::{Exited, Helper};
use std::fs;
use std::path::Path;
use std::time::Duration;

fn run_fixture(commands: &str, name: &str) -> Exited {
    let original = Path::new(common::plexi_binary());
    let root = tempfile::tempdir_in(original.parent().unwrap()).unwrap();
    let binary = root.path().join("plexi-pr-1471");
    fs::hard_link(original, &binary).unwrap();
    let channel_dir = ".plexi-pr-1471";
    let profile = root.path().join("home").join(channel_dir);
    fs::create_dir_all(&profile).unwrap();
    fs::write(
        profile.join("config.toml"),
        "config_version = 1\ncore_only = true\n",
    )
    .unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir_all(workspace.join(channel_dir)).unwrap();
    fs::write(workspace.join(channel_dir).join("commands.toml"), commands).unwrap();

    Helper::new(binary)
        .args(["run", name])
        .current_dir(workspace)
        .env_clear()
        .env("HOME", profile.parent().unwrap())
        .env("PATH", "/usr/bin:/bin")
        .run_any_exit(
            "recursive run fixture",
            timeout::load_aware_timeout(Duration::from_secs(30)),
        )
        .unwrap()
}

// The mkdir step bounds the number of subprocesses even if the guard regresses.
#[test]
fn run_command_cycle_direct() {
    let output = run_fixture(
        r#"
[commands.direct]
steps = ["mkdir visit-1 || mkdir visit-2 || exit 73", "run direct"]
"#,
        "direct",
    );
    let stderr = String::from_utf8_lossy(&output.output.stderr);
    assert!(
        stderr.contains("recursive command cycle: direct -> direct."),
        "{stderr}"
    );
    assert!(
        stderr.contains("Edit '") && stderr.contains("workspace/.plexi-pr-1471/commands.toml'"),
        "{stderr}"
    );
    assert_eq!(output.status.code(), Some(2), "{stderr}");
}

#[test]
fn run_command_cycle_indirect() {
    let output = run_fixture(
        r#"
[commands.outer]
steps = ["run a"]
[commands.a]
steps = ["mkdir visit-1 || mkdir visit-2 || exit 73", "run b -- a"]
[commands.b]
steps = ["run $1"]
"#,
        "outer",
    );
    let stderr = String::from_utf8_lossy(&output.output.stderr);
    assert!(
        stderr.contains("recursive command cycle: a -> b -> a."),
        "{stderr}"
    );
    assert!(
        stderr.contains("Edit '") && stderr.contains("workspace/.plexi-pr-1471/commands.toml'"),
        "{stderr}"
    );
    assert_eq!(output.status.code(), Some(2), "{stderr}");
}

#[test]
fn run_command_allows_repeated_sibling_steps() {
    let output = run_fixture(
        r#"
[commands]
child = "true"
[commands.parent]
steps = ["run child", "run child"]
"#,
        "parent",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.output.stderr),
    );
}
