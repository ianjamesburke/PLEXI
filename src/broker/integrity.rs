//! Death and tamper marker for the permission profile.
//!
//! The stamp records whether the last host process exited through
//! `exit_host` and the sha256 of the profile files it left behind. The next
//! start compares that stamp to the files on disk before grant migration can
//! rewrite them. An unexpected death or a hash mismatch is a Needs you
//! integrity item. The host's own writes refresh the stamp, so a grant save
//! or an audit append is not itself a tamper.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const STAMP_NAME: &str = "permission-integrity.toml";
const ABSENT: &str = "absent";

const PROFILE_FILES: [&str; 3] = ["grants.toml", "permission-audit.jsonl", "permissions.toml"];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct IntegrityStamp {
    clean_shutdown: bool,
    pid: u32,
    grants_sha256: String,
    audit_sha256: String,
    permissions_sha256: String,
}

/// What the next start should tell the human.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityFinding {
    pub unexpected_death: bool,
    pub profile_changed: bool,
    pub summary: String,
}

fn stamp_path(dir: &Path) -> PathBuf {
    dir.join(STAMP_NAME)
}

fn digest_file(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => format!("{:x}", Sha256::digest(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ABSENT.to_string(),
        Err(error) => {
            log::error!("integrity: could not read {}: {error}", path.display());
            format!("unreadable:{error}")
        }
    }
}

fn hashes(dir: &Path) -> (String, String, String) {
    (
        digest_file(&dir.join(PROFILE_FILES[0])),
        digest_file(&dir.join(PROFILE_FILES[1])),
        digest_file(&dir.join(PROFILE_FILES[2])),
    )
}

fn snapshot(dir: &Path, clean_shutdown: bool) -> IntegrityStamp {
    let (grants_sha256, audit_sha256, permissions_sha256) = hashes(dir);
    IntegrityStamp {
        clean_shutdown,
        pid: std::process::id(),
        grants_sha256,
        audit_sha256,
        permissions_sha256,
    }
}

fn profile_changed(previous: &IntegrityStamp, now: &IntegrityStamp) -> bool {
    previous.grants_sha256 != now.grants_sha256
        || previous.audit_sha256 != now.audit_sha256
        || previous.permissions_sha256 != now.permissions_sha256
}

fn summary(unexpected_death: bool, profile_changed: bool) -> String {
    match (unexpected_death, profile_changed) {
        (true, true) => "The host did not shut down cleanly and the permission profile changed while it was down.".to_string(),
        (true, false) => "The host did not shut down cleanly.".to_string(),
        (false, true) => "The permission profile changed while the host was down.".to_string(),
        (false, false) => String::new(),
    }
}

fn write_stamp(dir: &Path, stamp: &IntegrityStamp) {
    let path = stamp_path(dir);
    let body = match toml::to_string_pretty(stamp) {
        Ok(body) => body,
        Err(error) => {
            log::error!("integrity: could not serialize stamp: {error}");
            return;
        }
    };
    if let Err(error) = crate::platform::fs::atomic_write(&path, body.as_bytes()) {
        log::error!("integrity: could not write {}: {error}", path.display());
        return;
    }
    log::info!(
        "integrity: stamp clean_shutdown={} pid={} dir={}",
        stamp.clean_shutdown,
        stamp.pid,
        dir.display()
    );
}

/// Compare the on-disk stamp to the profile files. No stamp means first boot.
///
/// Call this before `GrantStore::load_or_default`. Migration rewrites
/// `grants.toml`, and a comparison after that rewrite hides an edit made
/// while the host was down.
pub fn inspect(dir: &Path) -> Option<IntegrityFinding> {
    let path = stamp_path(dir);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            log::error!("integrity: unreadable stamp {}: {error}", path.display());
            return Some(IntegrityFinding {
                unexpected_death: true,
                profile_changed: true,
                summary: summary(true, true),
            });
        }
    };
    let previous: IntegrityStamp = match toml::from_str(&raw) {
        Ok(stamp) => stamp,
        Err(error) => {
            log::error!("integrity: corrupt stamp {}: {error}", path.display());
            return Some(IntegrityFinding {
                unexpected_death: true,
                profile_changed: true,
                summary: summary(true, true),
            });
        }
    };
    let now = snapshot(dir, previous.clean_shutdown);
    let unexpected_death = !previous.clean_shutdown;
    let changed = profile_changed(&previous, &now);
    if !unexpected_death && !changed {
        log::info!("integrity: profile matches the last clean shutdown");
        return None;
    }
    let finding = IntegrityFinding {
        unexpected_death,
        profile_changed: changed,
        summary: summary(unexpected_death, changed),
    };
    log::info!(
        "integrity: unexpected_death={} profile_changed={} dir={}",
        finding.unexpected_death,
        finding.profile_changed,
        dir.display()
    );
    Some(finding)
}

/// Record that this process is running and the profile hashes are the ones it
/// currently sees. A kill leaves `clean_shutdown = false`.
pub fn mark_running(dir: &Path) {
    write_stamp(dir, &snapshot(dir, false));
}

/// Record that this process is exiting through the host's quit path.
pub fn mark_clean_shutdown(dir: &Path) {
    write_stamp(dir, &snapshot(dir, true));
}

/// Refresh the running stamp after the host itself wrote a profile file.
pub fn note_profile_write(dir: &Path) {
    mark_running(dir);
}

/// `path` is a profile file the host just saved. Detached test stores no-op.
pub fn note_saved_file(path: &Path) {
    if path.as_os_str().is_empty() {
        return;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    if !PROFILE_FILES.contains(&name) {
        return;
    }
    let Some(dir) = path.parent() else {
        return;
    };
    if dir.as_os_str().is_empty() {
        return;
    }
    note_profile_write(dir);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn first_boot_has_no_finding() {
        let dir = dir();
        assert!(inspect(dir.path()).is_none());
    }

    #[test]
    fn clean_shutdown_with_matching_hashes_files_nothing() {
        let dir = dir();
        std::fs::write(dir.path().join("grants.toml"), "records = []\n").unwrap();
        mark_running(dir.path());
        mark_clean_shutdown(dir.path());
        assert!(inspect(dir.path()).is_none());
    }

    #[test]
    fn unexpected_death_without_an_edit_is_not_a_profile_change() {
        let dir = dir();
        std::fs::write(dir.path().join("grants.toml"), "records = []\n").unwrap();
        mark_running(dir.path());
        let finding = inspect(dir.path()).expect("death");
        assert!(finding.unexpected_death);
        assert!(!finding.profile_changed);
        assert!(!finding.summary.contains("permission profile changed"));
    }

    #[test]
    fn kill_then_edit_names_the_profile_change() {
        let dir = dir();
        std::fs::write(dir.path().join("grants.toml"), "records = []\n").unwrap();
        mark_running(dir.path());
        std::fs::write(dir.path().join("grants.toml"), "records = []\n# tampered\n").unwrap();
        let finding = inspect(dir.path()).expect("tamper");
        assert!(finding.unexpected_death);
        assert!(finding.profile_changed);
        assert!(finding.summary.contains("permission profile changed"));
    }

    #[test]
    fn host_write_refreshes_the_stamp() {
        let dir = dir();
        mark_running(dir.path());
        std::fs::write(dir.path().join("grants.toml"), "records = []\n").unwrap();
        note_profile_write(dir.path());
        let finding = inspect(dir.path()).expect("still running");
        assert!(finding.unexpected_death);
        assert!(!finding.profile_changed);
    }

    #[test]
    fn corrupt_stamp_is_an_integrity_finding() {
        let dir = dir();
        std::fs::write(dir.path().join(STAMP_NAME), "not toml").unwrap();
        let finding = inspect(dir.path()).expect("corrupt");
        assert!(finding.profile_changed);
        assert!(finding.summary.contains("permission profile changed"));
    }

    #[test]
    fn edited_profile_while_down_files_an_integrity_item() {
        let dir = dir();
        std::fs::write(dir.path().join("grants.toml"), "records = []\n").unwrap();
        mark_running(dir.path());
        std::fs::write(dir.path().join("grants.toml"), "records = []\n# tampered\n").unwrap();

        let monitor = crate::broker::gate::PermissionMonitor::for_profile(dir.path());
        let items = monitor.list_needs_you();
        let item = items
            .iter()
            .find(|row| row.kind == crate::broker::gate::NeedsYouKind::Integrity)
            .expect("integrity item");
        assert!(
            item.summary.contains("permission profile changed"),
            "{item:?}"
        );
        assert!(item.expires_at.is_none());
        assert!(item.resolution.is_none());

        let receipt = monitor.resolve_needs_you(&item.id, true).unwrap();
        assert_eq!(
            receipt.resolution,
            crate::broker::gate::NeedsYouResolution::Approved
        );
        assert!(monitor.list_needs_you().is_empty());
        assert!(
            monitor.store().records().is_empty(),
            "acknowledging integrity must not mint a grant"
        );
    }
}
