//! A new build's CLI under a live host of another build (stint 1609, ruling
//! B): commands run without initializing or migrating the profile, the
//! deferral is logged, and only commands that read or write the profile's
//! plugins or SDK are refused, naming the live host and the fix.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const PROFILE: &str = "t1609";
const REFUSAL: &str = "Quit and reopen Plexi";

fn run(home: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_plexi"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PLEXI_") {
            command.env_remove(key);
        }
    }
    command
        .args(["--profile", PROFILE])
        .args(args)
        .current_dir(home)
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// A profile an older build initialized: its SDK stamp names that build.
fn old_build_profile(home: &Path) -> PathBuf {
    let profile = home.join(format!(".plexi-{PROFILE}"));
    std::fs::create_dir_all(profile.join("plugins")).unwrap();
    std::fs::create_dir_all(profile.join("sdk/plexi_sdk")).unwrap();
    std::fs::write(profile.join("sdk/pyproject.toml"), "").unwrap();
    std::fs::write(profile.join("sdk/.sdk_version"), "old-build").unwrap();
    // A current config, so this test covers plugin and SDK migration only:
    // config, agent and secret migrations under a deferred CLI are known
    // gaps tracked in their own stint.
    std::fs::write(profile.join("config.toml"), "config_version = 1\n").unwrap();
    profile
}

/// Every file and directory under the profile with its bytes, except the
/// shared log and the initialization lock the CLI checks under.
fn snapshot(profile: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if relative == Path::new("plexi.log") || relative == Path::new("initialization.lock") {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                out.insert(relative, None);
                walk(root, &path, out);
            } else {
                out.insert(relative, Some(std::fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(profile, profile, &mut out);
    out
}

fn assert_profile_unchanged(
    before: &BTreeMap<PathBuf, Option<Vec<u8>>>,
    profile: &Path,
    args: &[&str],
) {
    let after = snapshot(profile);
    let changed: Vec<_> = before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .collect();
    assert!(
        changed.is_empty(),
        "{args:?} changed the profile: {changed:?}"
    );
}

/// A plugin checkout outside the profile, for `plugin init --adopt`.
fn plugin_checkout(home: &Path) -> PathBuf {
    let dir = home.join("checkout");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("manifest.toml"),
        "schema_version = 2\n\n[plugin]\nid = \"adopted\"\nname = \"Adopted\"\nversion = \"0.1.0\"\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"adopted.main\"\ndefault = true\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("main.py"),
        "def init(size, args): return []\ndef update(event): return []\ndef view(): return None\n",
    )
    .unwrap();
    dir
}

/// The old host's lock, held by this process through its own open file.
fn hold_host_lock(profile: &Path) -> std::fs::File {
    let path = profile.join("host.lock");
    std::fs::write(&path, "4312").unwrap();
    let file = std::fs::File::open(&path).unwrap();
    file.lock().unwrap();
    file
}

fn stamp(profile: &Path) -> String {
    std::fs::read_to_string(profile.join("sdk/.sdk_version")).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn cli_under_an_old_host_runs_without_migrating_and_refuses_only_profile_commands() {
    let home = tempfile::tempdir().unwrap();
    let profile = old_build_profile(home.path());
    let host = hold_host_lock(&profile);

    let before = snapshot(&profile);
    let checkout = plugin_checkout(home.path());

    // Each allowed command reaches its real work and leaves the profile alone.
    let output = run(home.path(), &["config", "list"], &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(String::from_utf8_lossy(&output.stdout).contains("config_version"));
    assert_profile_unchanged(&before, &profile, &["config", "list"]);

    let output = run(home.path(), &["agent", "list"], &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(String::from_utf8_lossy(&output.stdout).contains("default"));
    assert_profile_unchanged(&before, &profile, &["agent", "list"]);

    // No host socket in this sandbox: the command got past the profile gate
    // to its host request.
    let output = run(home.path(), &["pane", "list"], &[]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("no Plexi host to talk to"),
        "{}",
        stderr(&output)
    );
    assert_profile_unchanged(&before, &profile, &["pane", "list"]);

    let adopt = ["plugin", "init", "--adopt", checkout.to_str().unwrap()];
    let output = run(home.path(), &adopt, &[]);
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(checkout.join("plexi.scaffold.toml").is_file());
    assert_profile_unchanged(&before, &profile, &adopt);

    let log = std::fs::read_to_string(profile.join("plexi.log")).unwrap();
    assert!(
        log.contains("deferred to the next host start") && log.contains("pid=Some(4312)"),
        "the deferral was not logged:\n{log}"
    );

    for args in [&["plugin", "list"][..], &["view", "list"], &["doctor"]] {
        let output = run(home.path(), args, &[]);
        let message = stderr(&output);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {message}");
        for needle in [
            REFUSAL,
            "pid 4312",
            "old-build",
            &format!("host restart --profile {PROFILE}"),
        ] {
            assert!(
                message.contains(needle),
                "{args:?}: {needle:?} missing: {message}"
            );
        }
        assert_profile_unchanged(&before, &profile, args);
    }

    // Once the old host is gone, the next command migrates as before.
    drop(host);
    let output = run(home.path(), &["config", "list"], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stamp(&profile),
        env!("PLEXI_BUILD_ID"),
        "the profile was not upgraded"
    );
}

/// A bare launch typed in a Plexi pane prints help and never touches the
/// profile, even one that needs migrating.
#[test]
fn bare_launch_inside_plexi_never_initializes_the_profile() {
    let home = tempfile::tempdir().unwrap();
    let profile = old_build_profile(home.path());
    let output = run(home.path(), &[], &[("PLEXI_RUNNING", "1")]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!output.stdout.is_empty(), "no help printed");
    assert_eq!(stamp(&profile), "old-build");
    assert!(!profile.join("initialization.lock").exists());
}
