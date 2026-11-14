//! Stint 1487 (e): the one-host claim in `main` runs before the host binds its
//! socket. With another process holding the profile's `host.lock`, a host
//! launch must exit `3` with the refusal, and never create `notify.sock`.
//! Removing the claim block, or moving it after the socket bind, fails this
//! test: the launch then starts a second host, which this test kills at the
//! deadline. The launch is a background (accessory) host so a failure never
//! raises a window, and it carries the `PLEXI_KEYCHAIN_PATH` /
//! `PLEXI_KEYCHAIN_PASSWORD` test hook (src/broker/host_key.rs,
//! src/workspace/secrets/store.rs) pointed at a throwaway file in the temp
//! home, so even a host that boots never reaches the login keychain.
#![cfg(unix)]

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn host_launch_on_a_held_profile_refuses_before_the_socket() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let profile = home.join(".plexi-pr-1487");
    std::fs::create_dir(&profile).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(profile.join("host.lock"))
        .unwrap();
    lock.try_lock().expect("the test holds the profile lock");

    // The channel comes from the binary name, as for an installed channel shim.
    let binary = home.join("plexi-pr-1487");
    std::fs::copy(env!("CARGO_BIN_EXE_plexi"), &binary).unwrap();
    let mut command = Command::new(&binary);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("PLEXI_") {
            command.env_remove(key);
        }
    }
    let keychain = home.join("throwaway.keychain-db");
    command
        .current_dir(home)
        .env("HOME", home)
        .env("PLEXI_BACKGROUND_HOST", "1")
        .env("PLEXI_KEYCHAIN_PATH", &keychain)
        .env("PLEXI_KEYCHAIN_PASSWORD", "host-claim-refusal-test");
    // The keychain isolation must be on the child before it can run at all.
    let envs: Vec<(String, Option<String>)> = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|v| v.to_string_lossy().into_owned()),
            )
        })
        .collect();
    assert!(
        envs.iter().any(|(key, value)| key == "PLEXI_KEYCHAIN_PATH"
            && value.as_deref() == Some(keychain.to_str().unwrap())),
        "the child must use a throwaway keychain inside the temp home"
    );
    assert!(envs
        .iter()
        .any(|(key, value)| key == "PLEXI_KEYCHAIN_PASSWORD"
            && value.as_deref().is_some_and(|v| !v.is_empty())));
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().ok();
            child.wait().ok();
            panic!("a second host started on a held profile: the claim in main did not refuse");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut stderr = String::new();
    std::io::Read::read_to_string(child.stderr.as_mut().unwrap(), &mut stderr).unwrap();
    assert_eq!(
        status.code(),
        Some(3),
        "refusal exit code; stderr: {stderr}"
    );
    assert!(
        stderr.contains("already running on this profile"),
        "{stderr}"
    );
    assert!(
        !profile.join("notify.sock").exists(),
        "the refused host must not bind the profile's socket"
    );
}
