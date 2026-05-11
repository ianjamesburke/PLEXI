//! Standalone `--help` parser that converts any CLI's help output into a
//! descriptor JSON string compatible with `descriptor-renderer`.
//!
//! Unlike `cli_crawl`, this module never caches — it always re-runs the
//! subprocess. Use it when you need a one-shot, fresh parse (e.g. `plexi open
//! --cli`). Use `cli_crawl` when you want disk caching between calls.

use std::time::Duration;

// ── public error type ─────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum CliParseError {
    #[error("binary `{binary}` not found or could not be spawned: {source}")]
    SpawnFailed {
        binary: String,
        #[source]
        source: std::io::Error,
    },
    #[error("`{binary} --help` timed out after {secs}s")]
    Timeout { binary: String, secs: u64 },
    #[error("failed to serialize descriptor: {0}")]
    Serialize(#[from] serde_json::Error),
}

// ── public entry point ────────────────────────────────────────────────────────

const TIMEOUT_SECS: u64 = 5;

/// Run `<binary> --help`, parse the output, and return a descriptor JSON string
/// matching the schema expected by `descriptor-renderer`. Never caches.
pub fn parse_help_to_descriptor(binary: &str) -> Result<String, CliParseError> {
    log::info!("cli_help_parser: running `{binary} --help`");

    let help_text = run_with_timeout(binary, "--help")?;
    let version = run_version(binary);

    let descriptor = crate::cli_crawl::parse_help(binary, &help_text, version);
    let count = descriptor.commands.len();

    let json = serde_json::to_string_pretty(&descriptor)?;
    log::info!("cli_help_parser: extracted {count} commands from `{binary} --help`");
    Ok(json)
}

// ── subprocess helpers ────────────────────────────────────────────────────────

fn run_with_timeout(binary: &str, arg: &str) -> Result<String, CliParseError> {
    use std::sync::mpsc;

    let binary_owned = binary.to_string();
    let arg_owned = arg.to_string();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let result = std::process::Command::new(&binary_owned)
            .arg(&arg_owned)
            .output();
        let _ = tx.send(result);
    });

    let timeout = Duration::from_secs(TIMEOUT_SECS);
    let output = rx
        .recv_timeout(timeout)
        .map_err(|_| CliParseError::Timeout {
            binary: binary.to_string(),
            secs: TIMEOUT_SECS,
        })?
        .map_err(|source| CliParseError::SpawnFailed {
            binary: binary.to_string(),
            source,
        })?;

    // Many CLIs print help to stderr; prefer stdout, fall back to stderr.
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let text = if stdout.trim().is_empty() {
        String::from_utf8_lossy(&output.stderr).into_owned()
    } else {
        stdout
    };

    if text.trim().is_empty() {
        log::warn!("cli_help_parser: `{binary} {arg}` produced no output");
    }

    Ok(text)
}

fn run_version(binary: &str) -> Option<String> {
    use std::sync::mpsc;

    let binary_owned = binary.to_string();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = std::process::Command::new(&binary_owned)
            .arg("--version")
            .output();
        let _ = tx.send(result);
    });

    let timeout = Duration::from_secs(TIMEOUT_SECS);
    let output = rx.recv_timeout(timeout).ok()?.ok()?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.lines().next().map(|l| l.trim().to_string())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the returned string is valid JSON with the expected top-level keys.
    #[test]
    fn parse_help_to_descriptor_returns_valid_json() {
        // `echo` is a safe binary guaranteed to exist; its --help may be minimal
        // but parse_help_to_descriptor must not error — it accepts sparse output.
        // We test with a known-good binary that always succeeds.
        let result = parse_help_to_descriptor("cargo");
        assert!(
            result.is_ok(),
            "expected Ok, got: {:?}",
            result.err()
        );
        let json = result.unwrap();
        let value: serde_json::Value = serde_json::from_str(&json)
            .expect("returned string must be valid JSON");
        assert!(value.get("name").is_some(), "descriptor must have `name`");
        assert!(
            value.get("commands").is_some(),
            "descriptor must have `commands`"
        );
        assert_eq!(
            value["name"].as_str(),
            Some("cargo"),
            "name must match binary"
        );
    }

    #[test]
    fn error_on_nonexistent_binary() {
        let result = parse_help_to_descriptor("__plexi_test_nonexistent_binary_xyz__");
        assert!(
            matches!(result, Err(CliParseError::SpawnFailed { .. })),
            "expected SpawnFailed, got: {:?}",
            result
        );
    }
}
