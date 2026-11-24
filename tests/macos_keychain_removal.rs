//! `secret rm --folder` through the real binary against a private keychain
//! file (stint 1635). `MacKeychain` is compiled out of test builds, so only
//! the binary exercises the production selector and delete.
//!
//! The test points a verified PR build's `PLEXI_KEYCHAIN_PATH` at a file
//! in its temp dir, with a fixture password, an isolated `HOME` and profile,
//! and every inherited `PLEXI_*` variable removed. The binary opens
//! that file by path; the user's search list and default keychain are never
//! read or changed. Ordered multi-file scope and duplicates are covered by
//! the fake-native tests in `src/workspace/secrets/mac_keychain.rs`. This
//! process seeds items itself with Keychain interaction disabled and
//! create-only adds, so the binary never writes them.
//!
//! An item this process creates trusts only this process to read its data.
//! The binary runs with interaction disabled, so a data request on it fails
//! instead of raising a dialog. The binary also traces every native data
//! copy, success included. Each `rm` must show zero copies and succeed on
//! such an item; the controls show both detectors firing.
#![cfg(target_os = "macos")]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};

const PROFILE: &str = "t1635";
const PASSWORD: &str = "t1635-throwaway-keychain";
const NAME: &str = "T1635_SECRET";
const LIMIT: Duration = Duration::from_secs(60);
/// Traces the production adapter writes at each native call.
const UI_ALLOWED: &str = "SecKeychainSetUserInteractionAllowed(true)";
const DELETE_CALL: &str = "SecKeychainItemDelete status=";
const DATA_CALL: &str = "SecKeychainItemCopyContent status=";
const KEYCHAIN: &str = "a.keychain-db";
const CARGO_BINARY: &str = env!("CARGO_BIN_EXE_plexi");
const BUILD_HINT: &str =
    "build a PLEXI_BUILD_TEST_CHANNEL=pr-<N> release build and install it as plexi-pr-<N>";
const HOOK_NAMES: [&str; 4] = [
    "PLEXI_KEYCHAIN_PATH",
    "PLEXI_KEYCHAIN_PASSWORD",
    "PLEXI_OPENROUTER_BASE_URL",
    "PLEXI_PICKER_SCRIPT",
];

fn check_test_hook_binary(path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    for name in HOOK_NAMES {
        // Mach-O images contain arbitrary bytes around these ASCII literals.
        let pattern = regex::bytes::Regex::new(name).expect("literal hook name");
        if !pattern.is_match(&bytes) {
            return Err(format!("missing {name}"));
        }
    }
    Ok(())
}

fn verified_cargo_channel() -> &'static str {
    // The integration cases run in parallel against the same Cargo artifact.
    static CHECK: std::sync::OnceLock<Result<&str, String>> = std::sync::OnceLock::new();
    CHECK.get_or_init(|| {
        check_test_hook_binary(Path::new(CARGO_BINARY))?;
        option_env!("PLEXI_BUILD_TEST_CHANNEL")
            .filter(|_| cfg!(plexi_test_channel))
            .ok_or_else(|| "missing compiled PR channel".to_string())
    })
    .as_ref()
    .unwrap_or_else(|reason| panic!("Refused {CARGO_BINARY}: {reason}; {BUILD_HINT}."))
}

#[test]
fn binary_check_requires_every_hook_name() {
    let temp = tempfile::tempdir().unwrap();
    let binary = temp.path().join("plexi");
    assert!(check_test_hook_binary(&binary).is_err());
    for missing in HOOK_NAMES {
        let names: Vec<_> = HOOK_NAMES
            .into_iter()
            .filter(|name| *name != missing)
            .collect();
        std::fs::write(&binary, names.join("\0")).unwrap();
        assert_eq!(
            check_test_hook_binary(&binary),
            Err(format!("missing {missing}"))
        );
    }
    let mut bytes = vec![0xff, 0];
    bytes.extend(HOOK_NAMES.join("\0").bytes());
    std::fs::write(&binary, bytes).unwrap();
    assert_eq!(check_test_hook_binary(&binary), Ok(()));
}

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
    binary: PathBuf,
    folder: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let channel = verified_cargo_channel();
        let root = tempfile::tempdir().unwrap();
        // Cargo names its output "plexi". The runtime enables hooks only
        // under the matching compiled PR basename, even with --profile.
        let binary = root.path().join(format!("plexi-{channel}"));
        std::fs::copy(CARGO_BINARY, &binary).unwrap();
        interaction_off();
        let folder = root.path().join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        let sandbox = Self {
            root,
            binary,
            folder,
        };
        let (enable, _) = sandbox.run(KEYCHAIN, &["experiments", "enable", "folder_secrets"]);
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

    fn log_len(&self) -> usize {
        std::fs::metadata(self.profile().join("plexi.log")).map_or(0, |meta| meta.len() as usize)
    }

    /// Runs the binary with `keychain` as its whole domain, killed if it does
    /// not finish within [`LIMIT`]. Returns this command's slice of
    /// `plexi.log`; a missing log fails the test.
    fn run(&self, keychain: &str, args: &[&str]) -> (Output, String) {
        self.run_with(keychain, &[], args)
    }

    fn run_with(&self, keychain: &str, env: &[(&str, &str)], args: &[&str]) -> (Output, String) {
        let home = self.root.path().join("home");
        let path = self.file(keychain);
        assert!(
            path.starts_with(self.root.path()),
            "{path:?} escapes the sandbox"
        );
        let mut command = Command::new(&self.binary);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("PLEXI_") {
                command.env_remove(key);
            }
        }
        command
            .args(["--profile", PROFILE])
            .args(args)
            .current_dir(self.root.path())
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("PLEXI_KEYCHAIN_PATH", &path)
            .env("PLEXI_KEYCHAIN_PASSWORD", PASSWORD)
            .env(NAME, "t1635-value")
            .envs(env.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let before = self.log_len();
        let output = bounded(command);
        let log = std::fs::read(self.profile().join("plexi.log"))
            .unwrap_or_else(|error| panic!("plexi.log missing after {args:?}: {error}"));
        assert!(log.len() > before, "{args:?} wrote nothing to plexi.log");
        (output, String::from_utf8_lossy(&log[before..]).into_owned())
    }

    /// Every removal is checked: no data copy and no UI, whatever its result.
    fn rm(&self, keychain: &str) -> (Output, String) {
        let folder = self.folder.to_str().unwrap().to_string();
        let (output, log) = self.run(keychain, &["secret", "rm", NAME, "--folder", &folder]);
        assert_no_password_request(&log);
        (output, log)
    }
}

/// The removal-side guarantees on one command's log: no data copy, no UI.
fn assert_no_password_request(log: &str) {
    assert_eq!(
        log.matches(DATA_CALL).count(),
        0,
        "removal requested item data: {log}"
    );
    assert!(
        !log.contains(UI_ALLOWED),
        "removal allowed Keychain UI: {log}"
    );
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
/// entry is needed, no data copy happens and UI is never allowed. A repeat
/// removal reports the no-match and fails.
#[test]
#[ignore = "needs a PLEXI_BUILD_TEST_CHANNEL=pr-<N> build; run with --ignored"]
fn unindexed_item_is_removed_without_a_password_request() {
    let sandbox = Sandbox::new();
    let account = sandbox.account();
    sandbox.seed(KEYCHAIN, &account, "seeded-by-test");

    let (first, log) = sandbox.rm(KEYCHAIN);
    assert!(removed(&first), "{}", text(&first.stderr));
    assert!(!sandbox.holds(KEYCHAIN, &account));
    assert_eq!(log.matches(&format!("{DELETE_CALL}0")).count(), 1, "{log}");

    let (again, log) = sandbox.rm(KEYCHAIN);
    assert!(!again.status.success());
    let err = text(&again.stderr);
    assert!(err.contains("returned no match"), "{err}");
    assert!(!text(&again.stdout).contains("Removed"));
    assert!(!log.contains(DELETE_CALL), "{log}");
}

/// A stale index entry over an already-created empty file is never success.
#[test]
#[ignore = "needs a PLEXI_BUILD_TEST_CHANNEL=pr-<N> build; run with --ignored"]
fn stale_cached_name_in_an_empty_file_is_a_reported_no_match() {
    let sandbox = Sandbox::new();
    let account = sandbox.account();
    sandbox.keychain(KEYCHAIN);
    sandbox.index(&[&account]);
    let (out, _) = sandbox.rm(KEYCHAIN);
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(err.contains("returned no match"), "{err}");
    assert!(!text(&out.stdout).contains("Removed"));
}

/// A damaged file fails at its own stage, never as a no-match, and no other
/// keychain is consulted.
#[test]
#[ignore = "needs a PLEXI_BUILD_TEST_CHANNEL=pr-<N> build; run with --ignored"]
fn damaged_keychain_is_an_open_error_not_a_no_match() {
    let sandbox = Sandbox::new();
    std::fs::write(sandbox.file("damaged.keychain-db"), b"not a keychain").unwrap();
    let (out, _) = sandbox.rm("damaged.keychain-db");
    let err = text(&out.stderr);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains("open the keychain (status "), "{err}");
    assert!(!err.contains("no match"), "{err}");
    assert!(!text(&out.stdout).contains("Removed"));
}

/// Controls for the detectors. A background data request on an item this
/// process wrote is refused and traced; a background read of an item the
/// binary wrote itself is traced with status 0; and a write that allows UI
/// leaves the UI trace. Removal must leave none of these.
#[test]
#[ignore = "needs a PLEXI_BUILD_TEST_CHANNEL=pr-<N> build; run with --ignored"]
fn controls_show_each_detector_fire() {
    let sandbox = Sandbox::new();
    // Alias migration reads the legacy value in the background.
    let legacy = "plexi:user:openrouter-api-key";
    sandbox.seed(KEYCHAIN, legacy, "trap-value");
    sandbox.index(&[legacy]);
    let (list, log) = sandbox.run(KEYCHAIN, &["secret", "list", "--global"]);
    assert!(
        log.contains(&format!("could not read '{legacy}' to migrate")),
        "the binary's data request was not refused: {}\n{log}",
        text(&list.stderr)
    );
    let refused: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(DATA_CALL))
        .collect();
    assert_eq!(refused.len(), 1, "{log}");
    assert!(
        !refused[0].ends_with(&format!("{DATA_CALL}0")),
        "{}",
        refused[0]
    );
    assert!(
        sandbox.holds(KEYCHAIN, legacy),
        "the trap item must survive"
    );
    assert!(!log.contains(UI_ALLOWED), "{log}");

    // An item the binary writes itself is readable by it: the migration's
    // two background reads each trace status 0.
    let mine = "plexi:user:openrouter-api-key";
    let fresh = "b.keychain-db";
    let (set, log) = sandbox.run_with(
        fresh,
        &[("openrouter-api-key", "binary-owned")],
        &[
            "secret",
            "set",
            "openrouter-api-key",
            "--global",
            "--from-env",
        ],
    );
    assert!(set.status.success(), "{}", text(&set.stderr));
    assert!(log.contains(UI_ALLOWED), "set must allow UI: {log}");
    sandbox.index(&[mine]);
    let (list, log) = sandbox.run(fresh, &["secret", "list", "--global"]);
    assert!(list.status.success(), "{}", text(&list.stderr));
    assert!(
        log.matches(&format!("{DATA_CALL}0")).count() >= 1,
        "a successful data copy must be traced: {log}"
    );
    assert!(!log.contains(UI_ALLOWED), "{log}");
}
/// A native delete refusal after a successful selection reaches the CLI as
/// a failure with its status, and the item survives. The selected file sits
/// in a read-only directory, so selection succeeds and
/// `SecKeychainItemDelete` cannot write the change.
#[test]
#[ignore = "needs a PLEXI_BUILD_TEST_CHANNEL=pr-<N> build; run with --ignored"]
fn native_delete_refusal_fails_with_its_status_and_keeps_the_item() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new();
    let account = sandbox.account();
    std::fs::create_dir(sandbox.file("ro")).unwrap();
    sandbox.seed("ro/a.keychain-db", &account, "kept");
    let set_mode = |path: PathBuf, mode: u32| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap()
    };
    set_mode(sandbox.file("ro/a.keychain-db"), 0o444);
    set_mode(sandbox.file("ro"), 0o555);
    let (out, log) = sandbox.rm("ro/a.keychain-db");
    set_mode(sandbox.file("ro"), 0o755);
    set_mode(sandbox.file("ro/a.keychain-db"), 0o644);

    let err = text(&out.stderr);
    assert!(!out.status.success(), "{err}");
    assert!(!text(&out.stdout).contains("Removed"));
    assert!(err.contains("delete the selected item (status "), "{err}");
    let deletes: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(DELETE_CALL))
        .collect();
    assert_eq!(
        deletes.len(),
        1,
        "expected exactly one native delete: {log}"
    );
    let call = deletes[0];
    let status = call.rsplit('=').next().unwrap().trim();
    assert_ne!(status, "0", "{call}");
    assert!(err.contains(&format!("(status {status})")), "{err}\n{call}");
    assert!(sandbox.holds("ro/a.keychain-db", &account));
}
