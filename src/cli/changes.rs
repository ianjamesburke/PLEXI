//! `plexi changes` — preview, accept, refresh, and revert a prepared edit.
//!
//! Propose and accept both admit through the permission monitor before any
//! disk write. Without a grant the command prints `permission_required` and
//! leaves the file alone.

use std::path::Path;

use crate::host::changes::{self, GateStop};

fn stop(error: GateStop) -> i32 {
    match error {
        GateStop::Required { pending_request_id } => {
            println!("permission_required");
            println!("pending_request_id={pending_request_id}");
            2
        }
        GateStop::Failed(message) if message.starts_with("stale:") => {
            eprintln!("error: {message}");
            println!("status=stale");
            3
        }
        GateStop::Failed(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

pub fn changes_profile_cli() -> i32 {
    println!("{}", crate::config::config_dir().display());
    0
}

/// Refused (stint 1291). A terminal is not the person: only a click in the
/// Plexi window records an allow, and the host has no request route for this
/// command yet. Nothing is written.
pub fn changes_allow_cli(agent: &str, file: &Path, _old: &str, _new: &str) -> i32 {
    log::info!(
        "changes: allow refused in the CLI process agent={agent} path={}",
        file.display()
    );
    eprintln!(
        "error: plexi changes allow cannot grant from the terminal. Only a click in the Plexi window allows an edit; nothing was recorded"
    );
    1
}

pub fn changes_propose_cli(agent: &str, file: &Path, old: &str, new: &str) -> i32 {
    match changes::propose_gated(agent, file, old, new) {
        Ok(prepared) => {
            println!("change_set={}", prepared.id);
            println!("status=pending");
            println!("applied=false");
            0
        }
        Err(error) => stop(error),
    }
}

pub fn changes_preview_cli(id: &str) -> i32 {
    match changes::preview(id) {
        Ok(diff) => {
            print!("{diff}");
            0
        }
        Err(error) => {
            eprintln!("error: {error}");
            1
        }
    }
}

pub fn changes_accept_cli(id: &str) -> i32 {
    match changes::accept_gated(id) {
        Ok(set) => {
            println!("change_set={}", set.id);
            println!("status=committed");
            println!("agent={}", set.agent_id);
            0
        }
        Err(error) => stop(error),
    }
}

pub fn changes_refresh_cli(id: &str) -> i32 {
    match changes::refresh_gated(id) {
        Ok(set) => {
            println!("change_set={}", set.id);
            println!("status={}", set.status);
            0
        }
        Err(error) => stop(error),
    }
}

pub fn changes_revert_cli(id: &str) -> i32 {
    match changes::revert_gated(id) {
        Ok(set) => {
            println!("change_set={}", set.id);
            println!("status=reverted");
            println!("agent={}", set.agent_id);
            0
        }
        Err(error) => stop(error),
    }
}

#[cfg(test)]
mod tests {
    /// Stint 1291: `plexi changes allow` runs in the CLI process, which never
    /// writes a grant or opens the permission store. A leftover legacy
    /// `permissions.toml` is not imported by it either (that import seals).
    /// Stint 1291: the gated change commands need the broker, which only the
    /// host holds. From a CLI process they fail with the named error and write
    /// nothing.
    #[test]
    fn changes_propose_refuses_outside_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.clone());
        let file = dir.path().join("draft.txt");
        std::fs::write(&file, "alpha\n").unwrap();

        let result = crate::broker::custody::as_cli_process(|| {
            crate::host::changes::propose_gated("editor-bot", &file, "alpha", "beta")
        });

        match result {
            Err(crate::host::changes::GateStop::Failed(message)) => assert!(
                message.contains("this process is not the host"),
                "{message}"
            ),
            Err(other) => panic!("expected the not-host refusal, got {other:?}"),
            Ok(_) => panic!("propose must not run outside the host"),
        }
        assert!(!profile.join("grants.toml").exists());
        assert!(!profile.join("permission-audit.jsonl").exists());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "alpha\n");
    }

    #[test]
    fn changes_allow_writes_no_grant() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.clone());
        let legacy = profile.join("permissions.toml");
        std::fs::write(&legacy, "[entries]\n").unwrap();
        let file = dir.path().join("draft.txt");
        std::fs::write(&file, "alpha\n").unwrap();

        let code = super::changes_allow_cli("editor-bot", &file, "alpha", "beta");

        assert_ne!(code, 0, "changes allow must refuse in the CLI process");
        assert!(
            !profile.join("grants.toml").exists(),
            "changes allow must not write grants.toml"
        );
        assert_eq!(
            std::fs::read_to_string(&legacy).unwrap(),
            "[entries]\n",
            "the CLI must not import or retire permissions.toml"
        );
    }
}
