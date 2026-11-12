use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::exec::*;
use super::fixture;

struct Rig {
    _root: tempfile::TempDir,
    bin: PathBuf,
    workspace: PathBuf,
    base: PathBuf,
}

impl Rig {
    fn new(version: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        let bin = fixture::install(version, &root.path().join("bin/fixturecli"));
        let base = root.path().join("sealed");
        Self {
            workspace,
            bin,
            base,
            _root: root,
        }
    }

    fn sealed(&self) -> SealedExecutable {
        let opened = open_and_identify(&self.bin).unwrap();
        let sha = opened.identity.sha256.clone();
        opened.seal(&self.base, &sha).unwrap()
    }

    fn spec(&self, sealed: SealedExecutable, args: &[&str], limits: Limits) -> LaunchSpec {
        LaunchSpec {
            sealed,
            display_name: "fixturecli".into(),
            argv: args.iter().map(OsString::from).collect(),
            cwd: self.workspace.clone(),
            env: fixed_env(),
            limits,
        }
    }
}

fn limits() -> Limits {
    Limits {
        timeout: Duration::from_secs(10),
        max_output: 4096,
    }
}

fn wake() -> Wake {
    Arc::new(|| {})
}

fn wait(op: &RunningOperation) -> Outcome {
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(15));
    loop {
        if let Some(outcome) = op.try_outcome() {
            return outcome;
        }
        assert!(Instant::now() < deadline, "operation never completed");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait_gone(pid: i32) -> bool {
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(3));
    while Instant::now() < deadline {
        if !pid_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn sha(path: &Path) -> String {
    open_and_identify(path).unwrap().identity.sha256
}

#[test]
fn echo_round_trip_records_one_launch() {
    let rig = Rig::new("a");
    let sealed = rig.sealed();
    let op = spawn_supervised(rig.spec(sealed, &["echo", "hello"], limits()), wake()).unwrap();
    let outcome = wait(&op);
    assert_eq!(outcome.completion, Completion::Exited(0), "{outcome:?}");
    let stdout = String::from_utf8(outcome.stdout).unwrap();
    assert!(stdout.contains(r#""argv":["hello"]"#), "{stdout}");
    assert_eq!(fixture::launches(&rig.workspace).len(), 1);
}

#[test]
fn launch_uses_fixed_cwd_and_minimal_environment() {
    let rig = Rig::new("a");
    let op = spawn_supervised(rig.spec(rig.sealed(), &["env"], limits()), wake()).unwrap();
    let outcome = wait(&op);
    let stdout = String::from_utf8(outcome.stdout).unwrap();
    let canonical = rig.workspace.canonicalize().unwrap();
    assert!(stdout.contains(canonical.to_str().unwrap()), "{stdout}");
    let keys: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let names: Vec<&str> = keys["env"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for name in &names {
        assert!(
            ["PATH", "LANG", "HOME"].contains(name),
            "unexpected env var {name}"
        );
    }
    assert!(names.contains(&"PATH"));
}

#[test]
fn nonzero_exit_is_reported() {
    let rig = Rig::new("a");
    let op = spawn_supervised(rig.spec(rig.sealed(), &["exit", "7"], limits()), wake()).unwrap();
    assert_eq!(wait(&op).completion, Completion::Exited(7));
}

#[test]
fn timeout_kills_the_program_and_its_children() {
    let rig = Rig::new("a");
    let short = Limits {
        timeout: Duration::from_millis(400),
        max_output: 4096,
    };
    let op = spawn_supervised(rig.spec(rig.sealed(), &["spawn-child"], short), wake()).unwrap();
    let outcome = wait(&op);
    assert_eq!(outcome.completion, Completion::TimedOut);
    let pid: i32 = std::fs::read_to_string(rig.workspace.join(".child-pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(wait_gone(pid), "grandchild {pid} survived the timeout");
}

#[test]
fn output_limit_kills_the_program() {
    let rig = Rig::new("a");
    let op = spawn_supervised(
        rig.spec(rig.sealed(), &["spray", "5000000"], limits()),
        wake(),
    )
    .unwrap();
    let outcome = wait(&op);
    assert_eq!(outcome.completion, Completion::OutputTooLarge);
    assert!(outcome.stdout.len() <= 4096, "{}", outcome.stdout.len());
}

#[test]
fn cancel_kills_the_program() {
    let rig = Rig::new("a");
    let op = spawn_supervised(
        rig.spec(rig.sealed(), &["sleep", "30000"], limits()),
        wake(),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(150));
    op.cancel();
    assert_eq!(wait(&op).completion, Completion::Cancelled);
}

#[test]
fn dropping_a_running_operation_kills_its_process_tree() {
    let rig = Rig::new("a");
    let op = spawn_supervised(rig.spec(rig.sealed(), &["spawn-child"], limits()), wake()).unwrap();
    let pid_file = rig.workspace.join(".child-pid");
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(5));
    while !pid_file.exists() {
        assert!(Instant::now() < deadline, "fixture never started its child");
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
    assert!(pid_alive(pid));
    drop(op);
    assert!(wait_gone(pid), "grandchild {pid} survived plugin close");
}

#[test]
fn replacement_after_the_identity_check_cannot_change_what_runs() {
    let rig = Rig::new("a");
    let opened = open_and_identify(&rig.bin).unwrap();
    let approved = opened.identity.sha256.clone();
    // Another process atomically replaces the program after validation.
    fixture::install("b", &rig.bin);
    assert_ne!(sha(&rig.bin), approved);
    let sealed = opened.seal(&rig.base, &approved).unwrap();
    let op = spawn_supervised(rig.spec(sealed, &["version"], limits()), wake()).unwrap();
    let outcome = wait(&op);
    let stdout = String::from_utf8(outcome.stdout).unwrap();
    assert!(
        stdout.contains(r#""version":"a""#),
        "the verified bytes must run, got {stdout}"
    );
    let launches = fixture::launches(&rig.workspace);
    assert_eq!(launches.len(), 1);
    assert!(launches[0].starts_with("a "), "{launches:?}");
}

#[test]
fn replacement_before_the_identity_check_launches_nothing() {
    let rig = Rig::new("a");
    let approved = sha(&rig.bin);
    fixture::install("b", &rig.bin);
    let opened = open_and_identify(&rig.bin).unwrap();
    let error = opened
        .seal(&rig.base, &approved)
        .err()
        .expect("changed content must not seal");
    assert!(
        matches!(error, ExecError::ContentMismatch { .. }),
        "{error}"
    );
    assert!(fixture::launches(&rig.workspace).is_empty());
}

#[test]
fn tampering_with_the_sealed_copy_is_caught_before_spawn() {
    let rig = Rig::new("a");
    let sealed = rig.sealed();
    let path = sealed.path().to_path_buf();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    std::fs::write(&path, &bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
    let error = spawn_supervised(rig.spec(sealed, &["version"], limits()), wake())
        .err()
        .expect("tampered copy must not launch");
    assert!(
        matches!(error, ExecError::ContentMismatch { .. }),
        "{error}"
    );
    assert!(fixture::launches(&rig.workspace).is_empty());
}

#[test]
fn swapping_the_sealed_file_for_another_inode_is_caught() {
    let rig = Rig::new("a");
    let sealed = rig.sealed();
    let path = sealed.path().to_path_buf();
    let identical = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, identical).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
    let error = spawn_supervised(rig.spec(sealed, &["version"], limits()), wake())
        .err()
        .expect("replaced inode must not launch");
    assert!(error.to_string().contains("replaced"), "{error}");
    assert!(fixture::launches(&rig.workspace).is_empty());
}

#[test]
fn unsafe_executables_are_refused() {
    let rig = Rig::new("a");
    let dir = rig.bin.parent().unwrap();

    let script = dir.join("script");
    std::fs::write(&script, "#!/bin/sh\necho hi\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(open_and_identify(&script)
        .err()
        .unwrap()
        .to_string()
        .contains("script"));

    let plain = dir.join("plain");
    std::fs::copy(&rig.bin, &plain).unwrap();
    std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(open_and_identify(&plain)
        .err()
        .unwrap()
        .to_string()
        .contains("not executable"));

    std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o775)).unwrap();
    assert!(open_and_identify(&plain)
        .err()
        .unwrap()
        .to_string()
        .contains("writable"));

    std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o4755)).unwrap();
    assert!(open_and_identify(&plain)
        .err()
        .unwrap()
        .to_string()
        .contains("setuid"));

    let empty = dir.join("empty");
    std::fs::write(&empty, b"").unwrap();
    std::fs::set_permissions(&empty, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(open_and_identify(&empty).is_err());

    let link = dir.join("link");
    std::os::unix::fs::symlink(&rig.bin, &link).unwrap();
    assert!(
        open_and_identify(&link).is_err(),
        "a final symlink must not be followed"
    );
    assert!(fixture::launches(&rig.workspace).is_empty());
}

#[test]
fn resolve_reads_metadata_only_and_never_probes_unknown_programs() {
    let rig = Rig::new("a");
    let dir = rig.bin.parent().unwrap().to_path_buf();
    let found = resolve(
        "fixturecli",
        &[PathBuf::from("/nonexistent-dir"), dir.clone()],
    )
    .unwrap();
    assert_eq!(found, rig.bin.canonicalize().unwrap());
    let error = resolve("missing-tool", &[dir]).unwrap_err();
    assert!(matches!(error, ExecError::NotFound { .. }), "{error}");
    assert!(
        fixture::launches(&rig.workspace).is_empty(),
        "resolution must not execute anything"
    );
}

#[test]
fn stderr_is_captured_and_bounded() {
    let rig = Rig::new("a");
    let op = spawn_supervised(
        rig.spec(rig.sealed(), &["read", "/nonexistent/file"], limits()),
        wake(),
    )
    .unwrap();
    let outcome = wait(&op);
    assert_eq!(outcome.completion, Completion::Exited(3));
    assert!(outcome.stderr.contains("read failed"), "{}", outcome.stderr);
    assert!(outcome.stderr.len() <= 2048);
}

#[test]
fn default_search_dirs_are_fixed_system_locations() {
    let dirs = default_search_dirs();
    assert!(dirs.contains(&PathBuf::from("/usr/bin")));
    assert!(dirs.iter().all(|d| d.is_absolute()));
}

fn cached_copies(base: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(names) = std::fs::read_dir(base) {
        for name in names.flatten() {
            if let Ok(digests) = std::fs::read_dir(name.path()) {
                found.extend(digests.flatten().map(|d| d.path()));
            }
        }
    }
    found
}

#[test]
fn a_verified_copy_is_built_once_and_reused() {
    let rig = Rig::new("a");
    let first = rig.sealed();
    let second = rig.sealed();
    assert_eq!(first.path(), second.path(), "same digest, same cached copy");
    assert_eq!(cached_copies(&rig.base).len(), 1);
    let op = spawn_supervised(rig.spec(second, &["version"], limits()), wake()).unwrap();
    assert_eq!(wait(&op).completion, Completion::Exited(0));
}

#[test]
fn replacement_after_the_identity_check_cannot_change_what_runs_on_a_cache_hit() {
    let rig = Rig::new("a");
    drop(rig.sealed()); // warm the cache
    let opened = open_and_identify(&rig.bin).unwrap();
    let approved = opened.identity.sha256.clone();
    fixture::install("b", &rig.bin);
    let sealed = opened.seal(&rig.base, &approved).unwrap();
    let op = spawn_supervised(rig.spec(sealed, &["version"], limits()), wake()).unwrap();
    let stdout = String::from_utf8(wait(&op).stdout).unwrap();
    assert!(stdout.contains(r#""version":"a""#), "{stdout}");
}

#[test]
fn a_corrupted_cached_copy_is_rebuilt_not_trusted() {
    let rig = Rig::new("a");
    let first = rig.sealed();
    let path = first.path().to_path_buf();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(&path, b"not the program").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
    let rebuilt = rig.sealed();
    rebuilt.verify().unwrap();
    let op = spawn_supervised(rig.spec(rebuilt, &["version"], limits()), wake()).unwrap();
    assert_eq!(wait(&op).completion, Completion::Exited(0));
    assert!(fixture::launches(&rig.workspace).len() == 1);
}

#[test]
fn a_superseded_digest_loses_its_copy() {
    let rig = Rig::new("a");
    drop(rig.sealed());
    fixture::install("b", &rig.bin);
    drop(rig.sealed());
    let copies = cached_copies(&rig.base);
    assert_eq!(copies.len(), 1, "{copies:?}");
    assert!(copies[0].ends_with(sha(&rig.bin)));
}
