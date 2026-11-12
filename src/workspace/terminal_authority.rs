//! Terminal authority across a host restart.
//!
//! The saved layout (`workspaces/default.json`) is plain JSON that any program
//! running as the person can rewrite, so it cannot say who opened a terminal
//! or which restrictions the terminal carried. Those facts are written to a
//! second file, `terminal-authority.toml`, sealed with the host permission MAC
//! (`broker::seal`) and bound to the exact layout bytes saved beside it.
//!
//! On restore a terminal is *verified* only when the ledger's MAC is good, the
//! ledger names the digest of the layout that was parsed, and it holds a
//! record for that pane. A verified terminal gets its sealed opener, parent,
//! spawner and posture; nothing in the JSON can change them.
//!
//! Every other terminal is *unverified*: no ledger (a layout from before this
//! file existed), an unreadable seal key, a rejected ledger, a layout that is
//! not the one that was sealed, or a missing record. An unverified terminal is
//! restored as a new principal. It gets a new pane id, so no grant stored for
//! `pane:<old id>` answers for it. It is never human-owned, receives no
//! injected secrets, and keeps no parent. Each brokered effect it attempts
//! asks the person in the Plexi window.
//!
//! The ledger does not make a pane the person: that distinction is removed by
//! the typed caller classification. Until then it only guarantees that a
//! restart cannot raise what a terminal held before it.
//!
//! Not covered here: replaying an older layout together with the ledger that
//! was sealed for it. Detecting that needs a monotonic generation in the host
//! key store, which belongs to the sealed-anchor work.

use crate::broker::seal::{self, SealStatus};
use crate::broker::PermissionPosture;
use crate::spatial::tiling::PaneId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const LEDGER_FILE: &str = "terminal-authority.toml";
const LEDGER_LABEL: &str = "terminal_authority";
const LEDGER_SCHEMA: u32 = 1;
const REJECTED_COPY: &str = "terminal-authority.toml.rejected";

/// How a terminal came to exist.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOpener {
    /// Opened from the Plexi window: a new page, a split, the first shell.
    Desktop,
    /// Opened for or by a request: an agent, a plugin, a command.
    Request,
}

/// The sealed authority facts for one terminal pane.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct TerminalAuthorityRecord {
    pub pane_id: PaneId,
    pub opener: TerminalOpener,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_pane_id: Option<PaneId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_by: Option<String>,
    // A table: keep it after every scalar field so the TOML stays valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posture: Option<PermissionPosture>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
struct Ledger {
    schema: u32,
    /// Digest of the layout bytes this ledger was sealed for.
    layout_sha256: String,
    #[serde(default)]
    terminals: Vec<TerminalAuthorityRecord>,
}

/// Why a restored terminal could not be tied to a sealed record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnverifiedReason {
    /// The caller restored a layout that was not read from disk.
    NoLayoutDigest,
    /// No ledger beside the layout: written before sealed records existed, or removed.
    NoLedger,
    /// The seal key could not be read, so the ledger could not be checked.
    KeyUnreadable,
    /// The ledger has no valid MAC, or its body is not a ledger.
    LedgerRejected,
    /// The ledger is authentic but was sealed for different layout bytes.
    LayoutChanged,
    /// The ledger matches the layout but has no record for this pane.
    NoRecord,
}

impl UnverifiedReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoLayoutDigest => "layout was not read from disk",
            Self::NoLedger => "no sealed record beside the layout",
            Self::KeyUnreadable => "seal key unreadable",
            Self::LedgerRejected => "sealed record rejected",
            Self::LayoutChanged => "layout changed after it was sealed",
            Self::NoRecord => "no sealed record for this pane",
        }
    }
}

/// What restore may assume about one terminal.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RestoredAuthority {
    Verified(TerminalAuthorityRecord),
    Unverified {
        former_pane_id: PaneId,
        reason: UnverifiedReason,
    },
}

/// The authority of every terminal in a layout being restored, keyed by the
/// pane id the terminal is restored under.
#[derive(Default, Debug)]
pub struct RestoreAuthority {
    by_pane: HashMap<PaneId, RestoredAuthority>,
}

impl RestoreAuthority {
    pub fn for_pane(&self, pane_id: PaneId) -> Option<&RestoredAuthority> {
        self.by_pane.get(&pane_id)
    }

    /// Decide every terminal in `layout` and reissue the pane id of each one
    /// that cannot be verified. Call before anything reads a pane id from
    /// `layout`.
    ///
    /// `layout_sha256` is the digest of the bytes `layout` was parsed from.
    /// `pane_id_floor` is a lower bound for reissued ids taken from state the
    /// layout cannot influence (stored grants).
    pub fn resolve(
        profile_dir: &Path,
        layout_sha256: Option<&str>,
        pane_id_floor: PaneId,
        layout: &mut super::WorkspaceFile,
    ) -> Self {
        let sealed = read_ledger(profile_dir, layout_sha256);
        let mut by_pane = HashMap::new();
        let mut unverified: Vec<(PaneId, UnverifiedReason)> = Vec::new();
        for window in &layout.windows {
            for pane in &window.panes {
                if !matches!(pane.kind, super::SavedPaneKind::Terminal) {
                    continue;
                }
                match &sealed {
                    Ok(records) => match records.get(&pane.id) {
                        Some(record) => {
                            by_pane.insert(pane.id, RestoredAuthority::Verified(record.clone()));
                        }
                        None => unverified.push((pane.id, UnverifiedReason::NoRecord)),
                    },
                    Err(reason) => unverified.push((pane.id, *reason)),
                }
            }
        }
        let verified_count = by_pane.len();
        let former_ids: Vec<PaneId> = unverified.iter().map(|(id, _)| *id).collect();
        let reissued = layout.reissue_pane_ids(&former_ids, pane_id_floor);
        for (former_pane_id, reason) in &unverified {
            let Some(pane_id) = reissued.get(former_pane_id).copied() else {
                // Unreachable: every id came from this layout. Leaving the
                // pane without an entry keeps it restricted.
                log::error!(
                    "terminal_authority: pane {former_pane_id} was not reissued; it stays restricted"
                );
                continue;
            };
            log::info!(
                "terminal_authority: pane {former_pane_id} restored as pane {pane_id} without a verified owner ({}). \
                 It is restricted: no injected secrets and no stored grants. \
                 Approve its requests in Needs you, or open a new terminal from the Plexi window.",
                reason.as_str()
            );
            by_pane.insert(
                pane_id,
                RestoredAuthority::Unverified {
                    former_pane_id: *former_pane_id,
                    reason: *reason,
                },
            );
        }
        log::info!(
            "terminal_authority: restore verified={verified_count} unverified={} profile={}",
            unverified.len(),
            profile_dir.display()
        );
        Self { by_pane }
    }
}

pub fn ledger_path_in(profile_dir: &Path) -> PathBuf {
    profile_dir.join(LEDGER_FILE)
}

/// Digest that binds a ledger to one saved layout.
pub fn layout_digest(layout_bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(layout_bytes))
}

/// Seal `terminals` for the layout whose bytes hash to `layout_sha256`.
///
/// Call after the layout itself is on disk. A failure leaves the previous
/// ledger in place; it no longer matches the layout, so the next restore
/// treats every terminal as unverified.
pub fn seal_layout(
    profile_dir: &Path,
    layout_sha256: &str,
    mut terminals: Vec<TerminalAuthorityRecord>,
) -> Result<(), String> {
    terminals.sort_by_key(|record| record.pane_id);
    let count = terminals.len();
    let ledger = Ledger {
        schema: LEDGER_SCHEMA,
        layout_sha256: layout_sha256.to_string(),
        terminals,
    };
    let path = ledger_path_in(profile_dir);
    seal::write_toml(&path, LEDGER_LABEL, &ledger).map_err(|error| {
        format!(
            "could not seal terminal authority in {}: {error}. \
             Terminals will be restored restricted until a save succeeds; \
             check that the Plexi seal key in the system keychain is readable.",
            path.display()
        )
    })?;
    log::info!("terminal_authority: sealed {count} terminals for layout {layout_sha256}");
    Ok(())
}

fn read_ledger(
    profile_dir: &Path,
    layout_sha256: Option<&str>,
) -> Result<HashMap<PaneId, TerminalAuthorityRecord>, UnverifiedReason> {
    let Some(layout_sha256) = layout_sha256 else {
        log::info!("terminal_authority: layout has no on-disk digest; no terminal is verified");
        return Err(UnverifiedReason::NoLayoutDigest);
    };
    let path = ledger_path_in(profile_dir);
    let body = match seal::read_sealed(&path) {
        SealStatus::Trusted(body) => body,
        SealStatus::Absent => {
            log::info!(
                "terminal_authority: no ledger at {}; no terminal is verified",
                path.display()
            );
            return Err(UnverifiedReason::NoLedger);
        }
        SealStatus::KeyUnreadable(reason) => {
            log::error!(
                "terminal_authority: seal key unreadable, {} left in place and no terminal is verified ({reason})",
                path.display()
            );
            return Err(UnverifiedReason::KeyUnreadable);
        }
        SealStatus::Rejected(reason) => {
            return Err(reject(&path, &reason));
        }
    };
    let ledger: Ledger = match toml::from_str(&body) {
        Ok(ledger) => ledger,
        Err(error) => return Err(reject(&path, &format!("not a ledger: {error}"))),
    };
    if ledger.schema != LEDGER_SCHEMA {
        return Err(reject(
            &path,
            &format!("schema {} is not {LEDGER_SCHEMA}", ledger.schema),
        ));
    }
    if ledger.layout_sha256 != layout_sha256 {
        log::error!(
            "terminal_authority: {} was sealed for layout {} but the layout on disk is {layout_sha256}; no terminal is verified",
            path.display(),
            ledger.layout_sha256
        );
        return Err(UnverifiedReason::LayoutChanged);
    }
    let mut records = HashMap::new();
    for record in ledger.terminals {
        let pane_id = record.pane_id;
        if records.insert(pane_id, record).is_some() {
            return Err(reject(&path, &format!("two records for pane {pane_id}")));
        }
    }
    Ok(records)
}

/// Keep a copy of a ledger that was refused, then report it. The next save
/// replaces the original.
fn reject(path: &Path, reason: &str) -> UnverifiedReason {
    log::error!(
        "terminal_authority: rejected {} ({reason}); no terminal is verified. A copy is kept as {REJECTED_COPY}.",
        path.display()
    );
    seal::copy_aside(path, REJECTED_COPY);
    UnverifiedReason::LedgerRejected
}

/// Seal the layout currently on disk in `profile_dir`, taking each terminal's
/// authority from the layout itself. For fixtures that write a layout by hand
/// and then restore it as if the host had saved it.
#[cfg(test)]
pub(crate) fn seal_layout_for_test(profile_dir: &Path) {
    let bytes = std::fs::read(super::workspace_path_in(profile_dir)).expect("read saved layout");
    let layout: super::WorkspaceFile = serde_json::from_slice(&bytes).expect("parse saved layout");
    let terminals = layout
        .windows
        .iter()
        .flat_map(|window| window.panes.iter())
        .filter(|pane| matches!(pane.kind, super::SavedPaneKind::Terminal))
        .map(|pane| {
            let session = pane.agent_session.as_ref();
            TerminalAuthorityRecord {
                pane_id: pane.id,
                opener: if session.is_some() {
                    TerminalOpener::Request
                } else {
                    TerminalOpener::Desktop
                },
                parent_pane_id: session.and_then(|s| s.parent_pane_id),
                spawned_by: session.and_then(|s| s.spawned_by.clone()),
                posture: session.and_then(|s| s.permission_posture.clone()),
            }
        })
        .collect();
    seal_layout(profile_dir, &layout_digest(&bytes), terminals).expect("seal test layout");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pane_id: PaneId, opener: TerminalOpener) -> TerminalAuthorityRecord {
        TerminalAuthorityRecord {
            pane_id,
            opener,
            parent_pane_id: None,
            spawned_by: None,
            posture: None,
        }
    }

    #[test]
    fn sealed_ledger_round_trips_records_and_posture() {
        let profile = tempfile::tempdir().unwrap();
        let digest = layout_digest(b"layout");
        let mut child = record(7, TerminalOpener::Request);
        child.parent_pane_id = Some(3);
        child.spawned_by = Some("agent:worker".to_string());
        child.posture = Some(PermissionPosture {
            default_posture: crate::broker::Decision::Deny,
            allow: vec!["fs.read".to_string()],
            ask: Vec::new(),
            deny: vec!["pane.send".to_string()],
        });
        let desktop = record(3, TerminalOpener::Desktop);
        seal_layout(
            profile.path(),
            &digest,
            vec![child.clone(), desktop.clone()],
        )
        .unwrap();

        let records = read_ledger(profile.path(), Some(&digest)).unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(records[&7], child);
        assert_eq!(records[&3], desktop);
    }

    #[test]
    fn ledger_for_other_layout_bytes_verifies_nothing() {
        let profile = tempfile::tempdir().unwrap();
        seal_layout(
            profile.path(),
            &layout_digest(b"layout one"),
            vec![record(3, TerminalOpener::Desktop)],
        )
        .unwrap();

        assert_eq!(
            read_ledger(profile.path(), Some(&layout_digest(b"layout two"))).unwrap_err(),
            UnverifiedReason::LayoutChanged
        );
        assert_eq!(
            read_ledger(profile.path(), None).unwrap_err(),
            UnverifiedReason::NoLayoutDigest
        );
    }

    #[test]
    fn missing_unsigned_and_altered_ledgers_verify_nothing() {
        let profile = tempfile::tempdir().unwrap();
        let digest = layout_digest(b"layout");
        assert_eq!(
            read_ledger(profile.path(), Some(&digest)).unwrap_err(),
            UnverifiedReason::NoLedger
        );

        seal_layout(
            profile.path(),
            &digest,
            vec![record(3, TerminalOpener::Request)],
        )
        .unwrap();
        let path = ledger_path_in(profile.path());
        let sealed = std::fs::read_to_string(&path).unwrap();

        std::fs::write(&path, sealed.replace("\"request\"", "\"desktop\"")).unwrap();
        assert_eq!(
            read_ledger(profile.path(), Some(&digest)).unwrap_err(),
            UnverifiedReason::LedgerRejected
        );
        assert!(
            profile.path().join(REJECTED_COPY).is_file(),
            "a refused ledger is kept for inspection"
        );

        let unsigned = format!(
            "schema = 1\nlayout_sha256 = \"{digest}\"\n\n[[terminals]]\npane_id = 3\nopener = \"desktop\"\n"
        );
        std::fs::write(&path, unsigned).unwrap();
        assert_eq!(
            read_ledger(profile.path(), Some(&digest)).unwrap_err(),
            UnverifiedReason::LedgerRejected
        );
    }
}
