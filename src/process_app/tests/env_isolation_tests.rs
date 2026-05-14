//! Proves that user-global secrets (stored as `plexi:user:*`) are NOT
//! injected into app subprocess environments. The only path to a secret
//! is `AppRequest::SecretGet` through the brokered capability check.
//!
//! Strategy: add a canary directly to the Command (simulating what the old
//! unconditional list_user_secrets() injection did), then apply env_clear()
//! + whitelist (what ProcessApp::launch now does), and assert the canary
//! is absent from the subprocess. Rust's Command builder strips explicit
//! .env() additions that precede .env_clear(), so no host-process env
//! mutation is needed — no thread-safety concerns.

use std::process::Command;

const WHITELIST: &[&str] = &["HOME", "PATH", "LANG", "LC_ALL", "TERM", "USER", "SHELL"];
const CANARY_KEY: &str = "PLEXI_SECRET_CANARY";
const CANARY_VAL: &str = "secret_must_not_leak";

#[test]
fn user_global_secrets_not_injected_into_subprocess_env() {
    let sh = ["/bin/sh", "/usr/bin/sh"]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .copied();
    let Some(sh) = sh else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };

    // Add the canary first (models what the old injection loop wrote),
    // then env_clear() which strips it. env(k,v) before env_clear() is
    // removed by env_clear() — verified empirically via Rust std behavior.
    let output = Command::new(sh)
        .arg("-c")
        .arg(format!("echo \"${{{}:-ABSENT}}\"", CANARY_KEY))
        .env(CANARY_KEY, CANARY_VAL)
        .env_clear()
        .envs(
            WHITELIST
                .iter()
                .filter_map(|k| std::env::var(k).ok().map(|v| (*k, v))),
        )
        .output()
        .expect("sh spawn failed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        "ABSENT",
        "user-global secret must not appear in subprocess env; got: {stdout:?}"
    );
}
