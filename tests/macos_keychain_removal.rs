//! `secret rm --folder` through the real binary against private keychain
//! files (stint 1635). `MacKeychain` is compiled out of test builds, so only
//! the binary exercises the production selector and delete.
//!
//! Every run points `PLEXI_KEYCHAIN_PATH` at files inside this test's temp
//! dir, with a fixture password, an isolated `HOME` and profile, and every
//! inherited `PLEXI_*` variable removed. The binary opens those files by
//! path; the user's search list and default keychain are never read or
//! changed. This process seeds items itself with Keychain interaction
//! disabled and create-only adds, so the binary never writes them.
//!
//! An item this process creates trusts only this process to read its data.
//! The binary runs with interaction disabled, so if it requested that data
//! the request would fail instead of raising a dialog. A removal that
//! succeeds on such an item therefore never requested the password. The
//! negative control shows the same item refusing the binary's data request.
#![cfg(target_os = "macos")]

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};

const PROFILE: &str = "t1635";
const PASSWORD: &str = "t1635-throwaway-keychain";
const NAME: &str = "T1635_SECRET";
const LIMIT: Duration = Duration::from_secs(60);
const UI_ALLOWED: &str = "allowing Keychain authentication";

/// Interaction stays off for this whole test process.
fn interaction_off() {
    static OFF: std::sync::Once = std::sync::Once::new();
    OFF.call_once(|| {
        let lock = SecKeychain::disable_user_interaction().expect("disable Keychain UI");
        std::mem::forget(lock);
    });
}

struct Sandbox {
    root: tempfile::TempDir,
    folder: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        interaction_off();
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        let sandbox = Self { root, folder };
        let enable = sandbox.run(&[], None, &["experiments", "enable", "folder_secrets"]);
        assert!(enable.status.success(), "{}", text(&enable.stderr));
        sandbox
    }

    fn file(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    /// A valid, unlocked, empty private keychain file.
    fn keychain(&self, name: &str) -> SecKeychain {
        let path = self.file(name);
        let mut keychain = if path.exists() {
            SecKeychain::open(&path).unwrap()
        } else {
            CreateOptions::new()
                .password(PASSWORD)
                .prompt_user(false)
                .create(&path)
                .unwrap()
        };
        keychain.unlock(Some(PASSWORD)).unwrap();
        keychain
    }

    fn account(&self) -> String {
        let canonical = std::fs::canonicalize(&self.folder).unwrap();
        let hex: String = canonical
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        format!("plexi:folder:{hex}:{NAME}")
    }

    /// Create-only add by this process, with interaction disabled.
    fn seed(&self, keychain: &str, account: &str, value: &str) {
        self.keychain(keychain)
            .add_generic_password("plexi", account, value.as_bytes())
            .unwrap();
    }

    /// Attributes-only: both password outputs null.
    fn holds(&self, keychain: &str, account: &str) -> bool {
        use core_foundation::base::TCFType;
        let keychain = self.keychain(keychain);
        let mut item = std::ptr::null_mut();
        let status = unsafe {
            security_framework_sys::keychain::SecKeychainFindGenericPassword(
                keychain.as_CFTypeRef(),
                5,
                "plexi".as_ptr().cast(),
                account.len() as u32,
                account.as_ptr().cast(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut item,
            )
        };
        if !item.is_null() {
            unsafe { core_foundation::base::CFRelease(item.cast()) };
        }
        match status {
            0 => true,
            -25300 => false,
            other => panic!("attribute lookup failed: {other}"),
        }
    }

    fn profile(&self) -> PathBuf {
        self.root
            .path()
            .join("home")
            .join(format!(".plexi-{PROFILE}"))
    }

    fn index(&self, accounts: &[&str]) {
        std::fs::create_dir_all(self.profile()).unwrap();
        std::fs::write(
            self.profile().join("secrets-index.json"),
            serde_json::to_string(accounts).unwrap(),
        )
        .unwrap();
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.profile().join("plexi.log")).unwrap_or_default()
    }

    /// Runs the binary with only these keychain files in its domain, killed
    /// if it does not finish within [`LIMIT`].
    fn run(&self, search: &[&str], default: Option<&str>, args: &[&str]) -> Output {
        let home = self.root.path().join("home");
        let mut command = Command::new(env!("CARGO_BIN_EXE_plexi"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("PLEXI_") {
                command.env_remove(key);
            }
        }
        let paths: Vec<PathBuf> = if search.is_empty() {
            vec![self.file("unused.keychain-db")]
        } else {
            search.iter().map(|name| self.file(name)).collect()
        };
        for path in paths
            .iter()
            .chain(default.map(|name| self.file(name)).iter())
        {
            assert!(
                path.starts_with(self.root.path()),
                "{path:?} escapes the sandbox"
            );
        }
        let joined = std::env::join_paths(&paths).unwrap();
        assert!(!joined.is_empty());
        command
            .args(["--profile", PROFILE])
            .args(args)
            .current_dir(self.root.path())
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("PLEXI_KEYCHAIN_PATH", &joined)
            .env("PLEXI_KEYCHAIN_PASSWORD", PASSWORD)
            .env(NAME, "t1635-value")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(default) = default {
            command.env("PLEXI_KEYCHAIN_DEFAULT_PATH", self.file(default));
        }
        bounded(command)
    }

    fn rm(&self, search: &[&str], default: Option<&str>) -> Output {
        let folder = self.folder.to_str().unwrap().to_string();
        self.run(
            search,
            default,
            &["secret", "rm", NAME, "--folder", &folder],
        )
    }
}

fn bounded(mut command: Command) -> Output {
    let mut child = command.spawn().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > LIMIT {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("plexi did not finish within {LIMIT:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Output {
        status,
        stdout: out.join().unwrap(),
        stderr: err.join().unwrap(),
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn removed(output: &Output) -> bool {
    output.status.success() && text(&output.stdout).contains("Removed")
}

/// An unindexed item this process wrote is removed by reference: no index
/// entry is needed, no password is requested and UI is never allowed. A
/// repeat removal reports the no-match and fails.
#[test]
fn unindexed_item_is_removed_without_a_password_request() {
    let sandbox = Sandbox::new();
    let account = sandbox.account();
    sandbox.seed("a.keychain-db", &account, "seeded-by-test");

    let first = sandbox.rm(&["a.keychain-db"], None);
    assert!(removed(&first), "{}", text(&first.stderr));
    assert!(!sandbox.holds("a.keychain-db", &account));
    let log = sandbox.log();
    assert!(
        log.contains(&format!("keychain delete account={account} status=0")),
        "{log}"
    );
    assert!(!log.contains(UI_ALLOWED), "{log}");

    let again = sandbox.rm(&["a.keychain-db"], None);
    assert!(!again.status.success());
    let err = text(&again.stderr);
    assert!(err.contains("returned no match"), "{err}");
    assert!(!text(&again.stdout).contains("Removed"));
}

/// A stale index entry over an already-created empty file is never success.
#[test]
fn stale_cached_name_in_an_empty_file_is_a_reported_no_match() {
    let sandbox = Sandbox::new();
    let account = sandbox.account();
    sandbox.keychain("empty.keychain-db");
    sandbox.index(&[&account]);
    let out = sandbox.rm(&["empty.keychain-db"], None);
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(err.contains("returned no match"), "{err}");
    assert!(!text(&out.stdout).contains("Removed"));
}

/// A damaged file fails at its own stage, never as a no-match, and no other
/// keychain is consulted.
#[test]
fn damaged_keychain_is_an_open_error_not_a_no_match() {
    let sandbox = Sandbox::new();
    std::fs::write(sandbox.file("damaged.keychain-db"), b"not a keychain").unwrap();
    let out = sandbox.rm(&["damaged.keychain-db"], None);
    let err = text(&out.stderr);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains("open the keychain (status "), "{err}");
    assert!(!err.contains("no match"), "{err}");
    assert!(!text(&out.stdout).contains("Removed"));
}

/// R2.1: an item only in a searched non-default keychain is removed; an
/// item only in a default keychain excluded from the search is not, and
/// nothing reports success for it.
#[test]
fn removal_uses_the_search_domain_not_the_default_keychain() {
    let sandbox = Sandbox::new();
    let account = sandbox.account();
    sandbox.keychain("a.keychain-db");
    sandbox.seed("b.keychain-db", &account, "in-b");
    let out = sandbox.rm(&["a.keychain-db", "b.keychain-db"], Some("a.keychain-db"));
    assert!(removed(&out), "{}", text(&out.stderr));
    assert!(!sandbox.holds("b.keychain-db", &account));

    sandbox.seed("c.keychain-db", &account, "in-excluded-default");
    let out = sandbox.rm(&["a.keychain-db"], Some("c.keychain-db"));
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("returned no match"));
    assert!(sandbox.holds("c.keychain-db", &account));
}

/// Duplicates: each removal deletes the first visible copy only.
#[test]
fn one_removal_deletes_one_duplicate() {
    let sandbox = Sandbox::new();
    let account = sandbox.account();
    sandbox.seed("a.keychain-db", &account, "first");
    sandbox.seed("b.keychain-db", &account, "second");
    let search = ["a.keychain-db", "b.keychain-db"];

    let first = sandbox.rm(&search, None);
    assert!(removed(&first), "{}", text(&first.stderr));
    assert!(!sandbox.holds("a.keychain-db", &account));
    assert!(sandbox.holds("b.keychain-db", &account));
    let second = sandbox.rm(&search, None);
    assert!(removed(&second), "{}", text(&second.stderr));
    assert!(!sandbox.holds("b.keychain-db", &account));
    assert!(!sandbox.rm(&search, None).status.success());
}

/// Negative controls for the detectors above. A background data request
/// on an item this process wrote is refused without a dialog, so the trap
/// that proves removal never requested a password is live. And a write
/// that does allow UI leaves the trace removal must never leave.
#[test]
fn negative_controls_show_the_detectors_fire() {
    let sandbox = Sandbox::new();
    // Alias migration reads the legacy value in the background.
    let legacy = "plexi:user:openrouter-api-key";
    sandbox.seed("a.keychain-db", legacy, "trap-value");
    sandbox.index(&[legacy]);
    let list = sandbox.run(&["a.keychain-db"], None, &["secret", "list", "--global"]);
    let log = sandbox.log();
    assert!(
        log.contains(&format!("could not read '{legacy}' to migrate")),
        "the binary's data request was not refused: {}\n{log}",
        text(&list.stderr)
    );
    assert!(
        sandbox.holds("a.keychain-db", legacy),
        "the trap item must survive"
    );
    assert!(!sandbox.holds("a.keychain-db", "plexi:user:OPENROUTER_API_KEY"));
    assert!(!log.contains(UI_ALLOWED), "{log}");

    let folder = sandbox.folder.to_str().unwrap().to_string();
    let set = sandbox.run(
        &["a.keychain-db"],
        None,
        &["secret", "set", NAME, "--from-env", "--folder", &folder],
    );
    assert!(set.status.success(), "{}", text(&set.stderr));
    assert!(sandbox.log().contains(UI_ALLOWED), "{}", sandbox.log());
}
