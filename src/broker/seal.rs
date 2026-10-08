//! Integrity for permission files.
//!
//! `grants.toml` is HMAC-SHA256 sealed. The audit log is an append-only hash
//! chain. The MAC key and the audit tip live in the host seal store
//! (`host_key`), never in `secrets.json` and never under a workspace id an
//! agent can pass to `plexi secret get`. Agent panes never receive the key.
//! A bad or missing MAC is not a grant.
//!
//! A profile that has never been sealed is the exception. Pre-seal builds
//! wrote `grants.toml` and `permission-audit.jsonl` with no MAC. Those files
//! are adopted once, then sealed. Adopt is allowed only when all three of
//! these host-store records are absent:
//!
//! - the permission MAC key (`permission-mac`)
//! - this profile's seal marker (`plexi:host:permission-seal:` plus the
//!   profile directory hash)
//! - this profile's audit tip
//!
//! Stripping `# plexi-mac:` clears none of them, so a sealed profile still
//! quarantines. The MAC key is one item for the whole host seal store: the
//! first profile that seals also closes legacy adopt for every other profile
//! on that store. Deleting the key is not enough on its own; the marker and
//! the tip still mean "sealed". A keychain or Secret Service read error
//! refuses adopt as well.
//!
//! That read error is not a bad MAC. When the key cannot be read, the grant
//! file and the audit log stay where they are and no integrity alert is filed.
//! Only a key that was actually read, and whose MAC does not match, is tampering.

use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use zeroize::Zeroizing;

/// Line prefix on a sealed TOML file. The HMAC covers every byte before it.
const MAC_PREFIX: &str = "# plexi-mac:";
/// Create-only secret-store account. Never written into a pane environment.
pub(crate) const KEY_ACCOUNT: &str = "plexi:host:permission-mac";
const AUDIT_TIP_PREFIX: &str = "plexi:host:permission-audit-tip:";
const SEAL_MARKER_PREFIX: &str = "plexi:host:permission-seal:";
const SEAL_MARKER_VALUE: &str = "1";
const GENESIS: &str = "genesis";
const UNAUTHENTICATED_AUDIT: &str = "audit log has no authenticated tip";

/// A grant, deny, or audit file the host refused to trust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityFault {
    pub file: String,
    pub reason: String,
}

/// Faults seen by a loader that ran before [`crate::broker::gate::PermissionMonitor`]
/// existed. That loader quarantines the file, so the monitor's own read finds
/// nothing and would otherwise file no Needs you.
fn noted_faults() -> &'static Mutex<Vec<(PathBuf, IntegrityFault)>> {
    static NOTED: OnceLock<Mutex<Vec<(PathBuf, IntegrityFault)>>> = OnceLock::new();
    NOTED.get_or_init(|| Mutex::new(Vec::new()))
}

pub(crate) fn note_integrity_fault(dir: &Path, fault: IntegrityFault) {
    let key = crate::platform::path::canonical_or_self(dir);
    log::info!(
        "permission_seal: noted integrity fault {} ({})",
        fault.file,
        fault.reason
    );
    noted_faults()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push((key, fault));
}

/// Faults noted for `dir` since the last take. The monitor raises each one.
pub(crate) fn take_integrity_faults(dir: &Path) -> Vec<IntegrityFault> {
    let key = crate::platform::path::canonical_or_self(dir);
    let mut guard = noted_faults()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut kept = Vec::new();
    let mut mine = Vec::new();
    for (noted_dir, fault) in guard.drain(..) {
        if noted_dir == key {
            mine.push(fault);
        } else {
            kept.push((noted_dir, fault));
        }
    }
    *guard = kept;
    mine
}

pub enum SealStatus {
    /// No file. A first run, not a tamper.
    Absent,
    /// MAC matched. `body` is the exact bytes the MAC covers.
    Trusted(String),
    /// The MAC key could not be read. The file stays. This is not tampering.
    KeyUnreadable(String),
    /// Missing or bad MAC. Fail closed and quarantine.
    Rejected(String),
}

#[derive(Debug)]
pub enum SealError {
    Untrusted(String),
    Io(String),
}

impl std::fmt::Display for SealError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Untrusted(message) | Self::Io(message) => f.write_str(message),
        }
    }
}

pub struct LoadedToml<T> {
    pub data: T,
    pub fault: Option<IntegrityFault>,
    /// True when a signed file parsed. Absent and rejected files are false.
    pub trusted: bool,
    /// The MAC key could not be read. The file was not moved.
    pub key_unreadable: bool,
}

/// Read a sealed TOML file. Does not move it; the caller quarantines.
pub fn read_sealed(path: &Path) -> SealStatus {
    let raw = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return SealStatus::Absent,
        Err(error) => return SealStatus::Rejected(format!("unreadable: {error}")),
    };
    if raw.is_empty() {
        return SealStatus::Rejected("empty file".to_string());
    }
    let text = match String::from_utf8(raw) {
        Ok(text) => text,
        Err(_) => return SealStatus::Rejected("not utf-8".to_string()),
    };
    let Some(idx) = text.rfind(MAC_PREFIX) else {
        if let Err(error) = existing_mac_key() {
            if super::host_key::is_key_unreadable(&error) {
                return SealStatus::KeyUnreadable(error);
            }
        }
        return SealStatus::Rejected("missing mac".to_string());
    };
    if idx > 0 && text.as_bytes()[idx - 1] != b'\n' {
        return SealStatus::Rejected("mac is not on its own line".to_string());
    }
    let mac_line = text[idx..].lines().next().unwrap_or("");
    let mac_hex = mac_line.trim_start_matches(MAC_PREFIX).trim();
    let body = &text[..idx];
    let key = match key_bytes() {
        Ok(key) => key,
        Err(error) if super::host_key::is_key_unreadable(&error) => {
            return SealStatus::KeyUnreadable(error);
        }
        Err(error) => return SealStatus::Rejected(error),
    };
    let expected = hmac_sha256(key.as_slice(), body.as_bytes());
    let Ok(got) = decode_hex(mac_hex) else {
        return SealStatus::Rejected("mac is not hex".to_string());
    };
    if !ct_eq(&expected, &got) {
        return SealStatus::Rejected("bad mac".to_string());
    }
    SealStatus::Trusted(body.to_string())
}

pub fn load_toml<T: DeserializeOwned + Default>(path: &Path) -> LoadedToml<T> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    match read_sealed(path) {
        SealStatus::Absent => LoadedToml {
            data: T::default(),
            fault: None,
            trusted: false,
            key_unreadable: false,
        },
        SealStatus::KeyUnreadable(reason) => {
            log::info!("permission_seal: key unreadable; left {file_name} in place ({reason})");
            LoadedToml {
                data: T::default(),
                fault: None,
                trusted: false,
                key_unreadable: true,
            }
        }
        SealStatus::Trusted(body) => {
            if let Some(dir) = path.parent() {
                mark_profile_sealed(dir);
            }
            match toml::from_str::<T>(&body) {
                Ok(data) => LoadedToml {
                    data,
                    fault: None,
                    trusted: true,
                    key_unreadable: false,
                },
                Err(error) => {
                    backup_corrupt(path, &error.to_string());
                    LoadedToml {
                        data: T::default(),
                        fault: None,
                        trusted: false,
                        key_unreadable: false,
                    }
                }
            }
        }
        SealStatus::Rejected(reason) => {
            log::info!("permission_seal: rejected {file_name} ({reason})");
            quarantine(path);
            LoadedToml {
                data: T::default(),
                fault: Some(IntegrityFault {
                    file: file_name,
                    reason,
                }),
                trusted: false,
                key_unreadable: false,
            }
        }
    }
}

pub fn write_toml<T: Serialize>(path: &Path, label: &str, data: &T) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    let body = toml::to_string_pretty(data).map_err(|error| error.to_string())?;
    write_sealed(path, body.as_bytes())?;
    log::info!("{label}: saved {}", path.display());
    Ok(())
}

pub fn write_sealed(path: &Path, body: &[u8]) -> Result<(), String> {
    let mut bytes = body.to_vec();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    let key = key_bytes()?;
    let mac = hex_encode(&hmac_sha256(key.as_slice(), &bytes));
    bytes.extend(format!("{MAC_PREFIX}{mac}\n").into_bytes());
    crate::platform::fs::atomic_write_with_mode(path, &bytes, 0o600)?;
    if let Some(dir) = path.parent() {
        mark_profile_sealed(dir);
    }
    Ok(())
}

/// Append one fact. Verifies the chain first. A bad chain is quarantined and
/// the tip is cleared before this returns [`SealError::Untrusted`].
pub fn append_audit(path: &Path, fact: &impl Serialize) -> Result<(), SealError> {
    if let Err(reason) = verify_audit(path) {
        if super::host_key::is_key_unreadable(&reason) {
            return Err(SealError::Io(reason));
        }
        reject_untrusted_audit(path, &reason);
        return Err(SealError::Untrusted(reason));
    }
    let key = key_bytes().map_err(SealError::Io)?;
    let (seq, prev) = next_link(path).map_err(SealError::Io)?;
    let fact_json =
        serde_json::to_string(fact).map_err(|error| SealError::Io(error.to_string()))?;
    let mac = hex_encode(&hmac_sha256(
        key.as_slice(),
        &payload(seq, &prev, &fact_json),
    ));
    let line = serde_json::to_string(&AuditEnvelope {
        seq,
        prev,
        fact: fact_json,
        mac,
    })
    .map_err(|error| SealError::Io(error.to_string()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| SealError::Io(error.to_string()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| SealError::Io(error.to_string()))?;
    use std::io::Write;
    writeln!(file, "{line}").map_err(|error| SealError::Io(error.to_string()))?;
    file.sync_all()
        .map_err(|error| SealError::Io(error.to_string()))?;
    let tip = format!("{seq}:{}", sha256_hex(line.as_bytes()));
    store_tip(path, &tip).map_err(SealError::Io)?;
    if let Some(dir) = path.parent() {
        mark_profile_sealed(dir);
    }
    Ok(())
}

/// Walk the chain and compare the authenticated tip. Does not move the file.
pub fn verify_audit(path: &Path) -> Result<(), String> {
    existing_mac_key()?;
    let tip = match tip_lookup(path) {
        Ok(value) => value,
        Err(error) if super::host_key::is_key_unreadable(&error) => return Err(error),
        Err(error) => {
            log::error!("permission_seal: could not read audit tip: {error}");
            None
        }
    };
    if !path.exists() {
        return if tip.is_some() {
            Err("audit log deleted".to_string())
        } else {
            Ok(())
        };
    }
    let raw = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let lines = content_lines(&raw);
    if lines.is_empty() {
        return if tip.is_some() {
            Err("audit log emptied".to_string())
        } else {
            Ok(())
        };
    }
    let Some(tip) = tip else {
        return Err(UNAUTHENTICATED_AUDIT.to_string());
    };
    let key = key_bytes()?;
    let mut prev = GENESIS.to_string();
    let mut last_seq = 0u64;
    let mut last_hash = String::new();
    for (idx, line) in lines.iter().enumerate() {
        if line.is_empty() {
            return Err("blank audit line".to_string());
        }
        let envelope: AuditEnvelope = serde_json::from_str(line)
            .map_err(|_| format!("audit line {} is not sealed", idx + 1))?;
        let expected_seq = (idx as u64) + 1;
        if envelope.seq != expected_seq {
            return Err(format!(
                "audit seq {} does not match position {expected_seq}",
                envelope.seq
            ));
        }
        if envelope.prev != prev {
            return Err(format!("audit prev mismatch at seq {}", envelope.seq));
        }
        let expected = hmac_sha256(
            key.as_slice(),
            &payload(envelope.seq, &envelope.prev, &envelope.fact),
        );
        let got = decode_hex(&envelope.mac)
            .map_err(|_| format!("audit mac is not hex at seq {}", envelope.seq))?;
        if !ct_eq(&expected, &got) {
            return Err(format!("audit mac mismatch at seq {}", envelope.seq));
        }
        last_hash = sha256_hex(line.as_bytes());
        last_seq = envelope.seq;
        prev = last_hash.clone();
    }
    let expected_tip = format!("{last_seq}:{last_hash}");
    if tip != expected_tip {
        return Err("audit tip mismatch".to_string());
    }
    Ok(())
}

pub fn reject_untrusted_audit(path: &Path, reason: &str) {
    log::info!(
        "permission_seal: audit rejected {} ({reason})",
        path.display()
    );
    if path.exists() {
        quarantine(path);
    }
    clear_audit_tip(path);
}

/// True when `reason` is the pre-seal audit failure: a log with lines and no tip.
pub(crate) fn unauthenticated_audit(reason: &str) -> bool {
    reason == UNAUTHENTICATED_AUDIT
}

/// Body of a file that has no MAC line. Absent, empty, and sealed files are `None`.
pub(crate) fn legacy_unsealed_body(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    if text.rfind(MAC_PREFIX).is_some() {
        return None;
    }
    Some(text)
}

/// No MAC key, no seal marker, and no audit tip for `dir`.
///
/// A read error is sealed. Legacy adopt must not run when the host store
/// cannot prove the profile is untouched.
pub(crate) fn profile_never_sealed(dir: &Path) -> bool {
    match existing_mac_key() {
        Ok(None) => {}
        Ok(Some(_)) => return false,
        Err(error) => {
            log::error!(
                "permission_seal: host mac key unreadable; not adopting legacy files in {}: {error}",
                dir.display()
            );
            return false;
        }
    }
    match super::host_key::get(&marker_account(dir)) {
        Ok(None) => {}
        Ok(Some(_)) => return false,
        Err(error) => {
            log::error!(
                "permission_seal: seal marker unreadable; not adopting legacy files in {}: {error}",
                dir.display()
            );
            return false;
        }
    }
    match tip_lookup(&dir.join("permission-audit.jsonl")) {
        Ok(None) => true,
        Ok(Some(_)) => false,
        Err(error) => {
            log::error!(
                "permission_seal: audit tip unreadable; not adopting legacy files in {}: {error}",
                dir.display()
            );
            false
        }
    }
}

/// Replace a pre-seal audit log with one authenticated migration line.
///
/// Does not quarantine. A profile that already has a tip is left untouched
/// and returns an error so the caller keeps the tamper path.
pub(crate) fn adopt_legacy_audit(path: &Path) -> Result<(), String> {
    if tip_lookup(path)?.is_some() {
        return Err("audit tip already exists".to_string());
    }
    if path.exists() {
        let raw = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let lines = content_lines(&raw);
        if !lines.is_empty() {
            log::info!(
                "permission_seal: replaced legacy unsealed audit {} ({} lines)",
                path.display(),
                lines.len()
            );
        }
        std::fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    let fact = serde_json::json!({
        "kind": "legacy_migration",
        "actor": "host",
        "decision": "migrated legacy unsealed permission-audit.jsonl",
    });
    append_audit(path, &fact).map_err(|error| error.to_string())?;
    log::info!(
        "permission_seal: started authenticated audit tip for {}",
        path.display()
    );
    Ok(())
}

/// Drop the MAC key from every pane. Agent panes also lose any value that
/// contains the profile directory, including the Unix command socket.
pub fn scrub_pane_env(env: &mut HashMap<String, String>, agent_pane: bool) {
    // Read a key that already exists. Creating one here opens the macOS
    // keychain while the first pane is being built, before `host start` can
    // report ready.
    let key_hex = match existing_mac_key() {
        Ok(Some(bytes)) => Some(Zeroizing::new(hex_encode(bytes.as_slice()))),
        Ok(None) => None,
        Err(error) => {
            log::error!("permission_seal: pane env scrub could not read the mac key: {error}");
            None
        }
    };
    let profile = crate::config::config_dir();
    let profile_text = profile.display().to_string();
    let canonical = crate::platform::path::canonical_or_self(&profile);
    let canonical_text = canonical.display().to_string();
    let mut removed = Vec::new();
    env.retain(|name, value| {
        let key_hit = name == KEY_ACCOUNT
            || name == "PLEXI_PERMISSION_MAC"
            || key_hex
                .as_ref()
                .is_some_and(|key| value.contains(key.as_str()) || name.contains(key.as_str()));
        let profile_hit = agent_pane
            && (name == "PLEXI_PROFILE"
                || name == "PLEXI_CONFIG_DIR"
                || value_has_profile(value, &profile_text)
                || value_has_profile(value, &canonical_text));
        if key_hit || profile_hit {
            removed.push(name.clone());
            false
        } else {
            true
        }
    });
    if removed.is_empty() {
        return;
    }
    let key_text = key_hex.as_ref().map(|key| key.as_str()).unwrap_or("");
    let safe: Vec<&str> = removed
        .iter()
        .map(String::as_str)
        .filter(|name| key_text.is_empty() || !name.contains(key_text))
        .collect();
    log::info!(
        "permission_seal: redacted pane env {safe:?} agent_pane={agent_pane} count={}",
        removed.len()
    );
}

fn value_has_profile(value: &str, profile: &str) -> bool {
    profile_is_specific(profile) && value.contains(profile)
}

fn profile_is_specific(profile: &str) -> bool {
    let trimmed = profile.trim_end_matches('/');
    trimmed.len() > 1 && trimmed != "."
}

#[derive(Serialize, serde::Deserialize)]
struct AuditEnvelope {
    seq: u64,
    prev: String,
    fact: String,
    mac: String,
}

fn payload(seq: u64, prev: &str, fact: &str) -> Vec<u8> {
    format!("{seq}\n{prev}\n{fact}").into_bytes()
}

fn content_lines(raw: &str) -> Vec<&str> {
    let mut text = raw;
    if let Some(stripped) = text.strip_suffix('\n') {
        text = stripped;
    }
    if let Some(stripped) = text.strip_suffix('\r') {
        text = stripped;
    }
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').collect()
    }
}

fn next_link(path: &Path) -> Result<(u64, String), String> {
    if !path.exists() {
        return Ok((1, GENESIS.to_string()));
    }
    let raw = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let lines = content_lines(&raw);
    let Some(last) = lines.last() else {
        return Ok((1, GENESIS.to_string()));
    };
    let envelope: AuditEnvelope =
        serde_json::from_str(last).map_err(|_| "audit tail is not sealed".to_string())?;
    Ok((envelope.seq + 1, sha256_hex(last.as_bytes())))
}

fn profile_dir_hash(dir: &Path) -> String {
    let canon = crate::platform::path::canonical_or_self(dir);
    sha256_hex(canon.to_string_lossy().as_bytes())
}

fn tip_account(path: &Path) -> String {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    format!("{AUDIT_TIP_PREFIX}{}", profile_dir_hash(dir))
}

fn marker_account(dir: &Path) -> String {
    format!("{SEAL_MARKER_PREFIX}{}", profile_dir_hash(dir))
}

fn tip_lookup(path: &Path) -> Result<Option<String>, String> {
    super::host_key::get(&tip_account(path)).map(|tip| tip.map(|value| value.to_string()))
}

fn mark_profile_sealed(dir: &Path) {
    if dir.as_os_str().is_empty() {
        return;
    }
    let account = marker_account(dir);
    match super::host_key::get(&account) {
        Ok(Some(_)) => {}
        Ok(None) => match super::host_key::set(&account, SEAL_MARKER_VALUE) {
            Ok(()) => log::info!(
                "permission_seal: recorded seal marker for {}",
                dir.display()
            ),
            Err(error) => log::error!(
                "permission_seal: could not record seal marker for {}: {error}",
                dir.display()
            ),
        },
        Err(error) => log::error!(
            "permission_seal: could not read seal marker for {}: {error}",
            dir.display()
        ),
    }
}

fn store_tip(path: &Path, tip: &str) -> Result<(), String> {
    super::host_key::set(&tip_account(path), tip)
}

fn clear_audit_tip(path: &Path) {
    let account = tip_account(path);
    if let Err(error) = super::host_key::delete(&account) {
        log::error!(
            "permission_seal: could not clear audit tip for {}: {error}",
            path.display()
        );
    } else {
        log::info!("permission_seal: cleared audit tip for {}", path.display());
    }
}

fn quarantine(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy();
    let dest = path.with_file_name(format!(
        "{name}.untrusted-{}-{}",
        crate::platform::clock::now_secs(),
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ));
    match std::fs::rename(path, &dest) {
        Ok(()) => {
            log::info!(
                "permission_seal: quarantined {} to {}",
                path.display(),
                dest.display()
            );
            Some(dest)
        }
        Err(error) => {
            log::error!(
                "permission_seal: could not quarantine {}: {error}",
                path.display()
            );
            None
        }
    }
}

fn backup_corrupt(path: &Path, error: &str) {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let dest = path.with_file_name(format!(
        "{name}.corrupt-{}",
        crate::platform::clock::now_secs()
    ));
    log::error!(
        "permission_seal: signed {} did not parse ({error}); backing up to {}",
        path.display(),
        dest.display()
    );
    if let Err(rename_error) = std::fs::rename(path, &dest) {
        log::error!(
            "permission_seal: could not rename corrupt file to {}: {rename_error}",
            dest.display()
        );
    }
}

/// Permission MAC if the host seal store already has one. Does not create a key.
///
/// Callers that seal with this key, including the Needs you journal, use this
/// instead of reading `secrets.json`.
pub(crate) fn existing_mac_key() -> Result<Option<Zeroizing<Vec<u8>>>, String> {
    super::host_key::scrub_user_secret_host_namespace();
    match super::host_key::get(super::host_key::MAC_ITEM)? {
        Some(existing) => Ok(Some(Zeroizing::new(decode_hex(existing.trim())?))),
        None => Ok(None),
    }
}

/// Permission MAC, creating it in the host seal store when this is the first use.
pub(crate) fn mac_key_bytes() -> Result<Zeroizing<Vec<u8>>, String> {
    let hex = key_hex()?;
    let bytes = decode_hex(hex.as_str())?;
    Ok(Zeroizing::new(bytes))
}

fn key_hex() -> Result<Zeroizing<String>, String> {
    if let Some(existing) = existing_mac_key()? {
        return Ok(Zeroizing::new(hex_encode(&existing)));
    }
    let hex = fresh_key_hex();
    match super::host_key::add_new(super::host_key::MAC_ITEM, &hex) {
        Ok(()) => {
            log::info!("permission_seal: created host permission mac key");
            Ok(Zeroizing::new(hex))
        }
        Err(error) => match existing_mac_key()? {
            Some(existing) => Ok(Zeroizing::new(hex_encode(&existing))),
            None => Err(error),
        },
    }
}

fn key_bytes() -> Result<Zeroizing<Vec<u8>>, String> {
    mac_key_bytes()
}

fn fresh_key_hex() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut key_block = [0u8; BLOCK];
    if key.len() > BLOCK {
        let digest = Sha256::digest(key);
        key_block[..digest.len()].copy_from_slice(&digest);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    key_block.fill(0);
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(data);
    let inner_hash = inner.finalize();
    ipad.fill(0);
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_hash);
    opad.fill(0);
    let mut out = [0u8; 32];
    out.copy_from_slice(&outer.finalize());
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("odd hex length".to_string());
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_val(bytes[i])?;
        let lo = hex_val(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Ok(out)
}

fn hex_val(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("mac is not hex".to_string()),
    }
}

fn ct_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha256_matches_rfc4231_case_1() {
        let key = [0x0bu8; 20];
        let mac = hmac_sha256(&key, b"Hi There");
        assert_eq!(
            hex_encode(&mac),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn scrub_removes_mac_key_and_agent_profile_path() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_profile_dir(dir.path().to_path_buf());
        let key = key_hex().unwrap();
        let mut env = HashMap::new();
        env.insert(
            "PLEXI_SOCKET".into(),
            format!("{}/notify.sock", dir.path().display()),
        );
        env.insert("LEAK".into(), key.to_string());
        env.insert("PLEXI_PROFILE".into(), dir.path().display().to_string());
        env.insert("PLEXI_CHANNEL".into(), "pr-test".into());
        env.insert("OK".into(), "fine".into());
        scrub_pane_env(&mut env, true);
        assert!(!env.contains_key("PLEXI_SOCKET"));
        assert!(!env.contains_key("PLEXI_PROFILE"));
        assert!(!env.contains_key("LEAK"));
        assert_eq!(
            env.get("PLEXI_CHANNEL").map(String::as_str),
            Some("pr-test")
        );
        assert_eq!(env.get("OK").map(String::as_str), Some("fine"));
        for value in env.values() {
            assert!(!value.contains(key.as_str()));
        }

        let mut human = HashMap::new();
        human.insert(
            "PLEXI_SOCKET".into(),
            format!("{}/notify.sock", dir.path().display()),
        );
        human.insert("LEAK".into(), key.to_string());
        scrub_pane_env(&mut human, false);
        assert!(human.contains_key("PLEXI_SOCKET"));
        assert!(!human.contains_key("LEAK"));
    }
}
