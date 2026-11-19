//! `MacKeychain::contains` is compiled out of test builds, so this drives the
//! real binary through `secret rm --folder`, which asks `contains` before it
//! deletes (stint 1635). Every run points `PLEXI_KEYCHAIN_PATH` at a file
//! inside a temp dir: the binary opens that file by path and never touches
//! the default keychain or the search list. Keychain UI is off in the binary,
//! so a would-prompt call fails instead of showing a dialog.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const NAME: &str = "T1635_SECRET";
const PASSWORD: &str = "t1635-throwaway-keychain";

struct Sandbox {
    root: tempfile::TempDir,
    folder: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        let sandbox = Self { root, folder };
        let enable = sandbox.run(
            &sandbox.keychain(),
            &["experiments", "enable", "folder_secrets"],
        );
        assert!(enable.status.success(), "{}", stderr(&enable));
        sandbox
    }

    fn keychain(&self) -> PathBuf {
        self.root.path().join("t1635.keychain-db")
    }

    fn run(&self, keychain: &Path, args: &[&str]) -> Output {
        let home = self.root.path().join("home");
        let mut command = Command::new(env!("CARGO_BIN_EXE_plexi"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("PLEXI_") {
                command.env_remove(key);
            }
        }
        command
            .args(args)
            .current_dir(self.root.path())
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("PLEXI_KEYCHAIN_PATH", keychain)
            .env("PLEXI_KEYCHAIN_PASSWORD", PASSWORD)
            .env(NAME, "t1635-value")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn rm(&self, keychain: &Path) -> Output {
        let folder = self.folder.to_str().unwrap();
        self.run(keychain, &["secret", "rm", NAME, "--folder", folder])
    }

    fn set(&self) {
        let folder = self.folder.to_str().unwrap();
        let out = self.run(
            &self.keychain(),
            &["secret", "set", NAME, "--from-env", "--folder", folder],
        );
        assert!(out.status.success(), "{}", stderr(&out));
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn present_item_is_found_and_removed() {
    let sandbox = Sandbox::new();
    sandbox.set();
    let out = sandbox.rm(&sandbox.keychain());
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Removed"));
}

#[test]
fn missing_item_is_reported_not_found() {
    let sandbox = Sandbox::new();
    let out = sandbox.rm(&sandbox.keychain());
    assert!(!out.status.success());
    assert!(stderr(&out).contains("not found"), "{}", stderr(&out));
}

/// A damaged keychain file is a lookup failure, never "not found". A locked
/// keychain cannot be forced from here: securityd keeps a keychain unlocked
/// across processes, and an attributes-only lookup answers while locked.
#[test]
fn damaged_keychain_is_an_error_not_absence() {
    let sandbox = Sandbox::new();
    let damaged = sandbox.root.path().join("damaged.keychain-db");
    std::fs::write(&damaged, b"not a keychain").unwrap();
    let out = sandbox.rm(&damaged);
    let err = stderr(&out);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains("lookup failed"), "{err}");
    assert!(!err.contains("not found"), "{err}");
}
