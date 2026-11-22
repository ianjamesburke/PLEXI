//! Help, version and shell completions never run profile initialization or
//! plugin migration (stint 1604). The installer generates completions with the
//! new binary while an older Plexi may still own the profile; a profile whose
//! migration would fail must not stop it.

use std::path::Path;
use std::process::{Command, Output, Stdio};

const PROFILE: &str = "t1604";

fn run(home: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_plexi"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PLEXI_") {
            command.env_remove(key);
        }
    }
    command
        .args(["--profile", PROFILE])
        .arg("--profile-root")
        .arg(home)
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// A migrated profile where `apps/` came back holding a package `plugins/`
/// lacks: plugin migration fails on it, so profile initialization would too.
fn broken_profile(home: &Path) -> std::path::PathBuf {
    let profile = home.join(format!(".plexi-{PROFILE}"));
    std::fs::create_dir_all(profile.join("migration-plugin-v2")).unwrap();
    std::fs::write(profile.join("migration-plugin-v2/complete"), "schema=2\n").unwrap();
    std::fs::create_dir_all(profile.join("plugins")).unwrap();
    std::fs::create_dir_all(profile.join("apps/mine")).unwrap();
    profile
}

#[test]
fn help_version_and_completions_never_initialize_the_profile() {
    let home = tempfile::tempdir().unwrap();
    let profile = broken_profile(home.path());
    let commands: &[&[&str]] = &[
        &["completions", "zsh"],
        &["completions", "bash"],
        &["completions", "fish"],
        &["--version"],
        &["help"],
        &["help", "pane"],
        &["pane", "--help"],
    ];
    for args in commands {
        let output = run(home.path(), args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{args:?}: {stderr}");
        assert!(!output.stdout.is_empty(), "{args:?} printed nothing");
        assert!(!stderr.contains("plugin_migration"), "{args:?}: {stderr}");
    }
    assert!(
        !profile.join("migration-plugin-v2.lock").exists(),
        "plugin migration was attempted"
    );
    assert!(
        !profile.join("initialization.lock").exists(),
        "profile initialization was attempted"
    );
    assert!(!profile.join("sdk").exists(), "profile was initialized");
    assert!(profile.join("apps/mine").is_dir());
}

/// Completions read nothing from the profile: an unreadable profile directory
/// changes neither the exit status nor a byte of the output.
#[cfg(unix)]
#[test]
fn completions_are_identical_with_an_unreadable_profile() {
    use std::os::unix::fs::PermissionsExt;
    let clean = tempfile::tempdir().unwrap();
    let expected = run(clean.path(), &["completions", "zsh"]);
    assert!(expected.status.success());

    let home = tempfile::tempdir().unwrap();
    let profile = broken_profile(home.path());
    std::fs::write(profile.join("config.toml"), "[experiments]\nbogus = true\n").unwrap();
    std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o000)).unwrap();
    let output = run(home.path(), &["completions", "zsh"]);
    std::fs::set_permissions(&profile, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, expected.stdout);
}
