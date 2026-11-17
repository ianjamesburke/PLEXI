//! A fresh profile's seeding (example and core packs, Python plugin venvs)
//! takes seconds; the logger must be up before it starts so a slow first
//! launch leaves a trace instead of a silent gap (stint 1567).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn find_log(dir: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == "plexi.log") {
            return Some(path);
        }
        if path.is_dir() && entry.file_name().to_string_lossy().starts_with(".plexi") {
            if let Some(found) = find_log(&path) {
                return Some(found);
            }
        }
    }
    None
}

#[test]
fn fresh_profile_seeding_is_logged_after_logger_start() {
    let home = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_plexi"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PLEXI_") {
            command.env_remove(key);
        }
    }
    // No `uv` on PATH: venv provisioning fails fast and offline, and its
    // warning must still reach the log.
    let output = command
        .args(["plugin", "list", "--json"])
        .current_dir(cwd.path())
        .env("HOME", home.path())
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log_path = find_log(home.path()).expect("fresh profile has a plexi.log");
    let log = std::fs::read_to_string(&log_path).unwrap();
    let position = |needle: &str| {
        log.find(needle)
            .unwrap_or_else(|| panic!("{needle:?} missing from {}:\n{log}", log_path.display()))
    };
    let ready = position("startup: logger ready ");
    let seeded = position("examples pack: seeded ");
    let done = position("startup: profile initialization took ");
    assert!(ready < seeded && seeded < done, "{log}");
    assert!(log[done..].contains("first_launch=true"), "{log}");
}
