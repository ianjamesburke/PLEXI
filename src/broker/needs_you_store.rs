//! Host-only persistence for the Needs you queue and its pending approvals.
//!
//! The queue lives under `<profile>/host`, mode `0700` on Unix. That directory
//! is not the grant file and not `secrets.json`, and pane environment
//! construction does not receive its path. The journal is HMAC-SHA256 sealed
//! with the host permission MAC (`seal::mac_key_bytes`), the same key that
//! seals grants. The signed profile seal state remembers the last MAC.
//! Invalid MACs and pre-resolved items trigger quarantine. A missing journal
//! with a remembered MAC raises an integrity finding. Loading never writes
//! a grant and ignores any leftover `host/seal.key`.

use super::custody::HostProcess;
use super::gate::{ContinuationRecord, NeedsYouDetails, NeedsYouKind};
use super::principal::{PanePrincipal, RecordOwner};
use super::ExactBinding;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const SCHEMA: u32 = 2;
const MAC_LEN: usize = 32;

/// `<profile>/host`. The host writes it; a pane is not given this path.
pub(crate) fn host_private_dir(profile: &Path) -> PathBuf {
    profile.join("host")
}

pub(crate) fn queue_file_exists(profile: &Path) -> bool {
    let path = journal_path(profile);
    fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file())
}

#[derive(Debug)]
pub(crate) enum LoadedQueue {
    Empty,
    Items(Vec<RestoredItem>),
    Untrusted(String),
}

#[derive(Debug, Clone)]
pub(crate) struct RestoredItem {
    pub id: String,
    pub kind: NeedsYouKind,
    pub actor: RecordOwner,
    pub resource: String,
    pub summary: String,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub run_tag: Option<String>,
    pub origin_session: String,
    pub pending: Option<RestoredPending>,
    pub details: Option<NeedsYouDetails>,
}

#[derive(Debug, Clone)]
pub(crate) struct RestoredPending {
    pub tool: String,
    pub input_summary: String,
    pub binding: ExactBinding,
}

#[derive(serde::Serialize)]
struct DiskQueue<'a> {
    schema: u32,
    items: &'a [DiskItem],
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DiskQueueOwned {
    schema: u32,
    items: Vec<DiskItem>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DiskItem {
    id: String,
    kind: NeedsYouKind,
    actor: RecordOwner,
    resource: String,
    summary: String,
    created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_tag: Option<String>,
    origin_session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<DiskPending>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    details: Option<NeedsYouDetails>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DiskPending {
    tool: String,
    input_summary: String,
    binding: ExactBinding,
}

#[derive(serde::Serialize)]
struct DiskRun<'a> {
    #[serde(flatten)]
    row: &'a ContinuationRecord,
    owner: &'a RecordOwner,
}

#[derive(serde::Serialize)]
struct DiskContinuations<'a> {
    schema: u32,
    records: Vec<DiskRun<'a>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DiskContinuationsOwned {
    schema: u32,
    records: Vec<ContinuationRecord>,
}

const TIP_PREFIX: &str = "plexi:host:needs-you-journal-tip:";

pub(crate) fn load(host: &HostProcess, profile: &Path) -> LoadedQueue {
    let journal = journal_path(profile);
    if is_symlink(&journal) || is_symlink(&host_private_dir(profile)) {
        return LoadedQueue::Untrusted("host queue path is a symlink".to_string());
    }
    let tip = match read_tip(host, profile) {
        Ok(tip) => tip,
        Err(error) => return LoadedQueue::Untrusted(error),
    };
    if !journal_is_file(&journal) {
        if tip_is_set(&tip) {
            if let Err(error) = super::seal::existing_mac_key(host, profile) {
                return LoadedQueue::Untrusted(error);
            }
            return missing_journal();
        }
        return LoadedQueue::Empty;
    }
    let host_key = match super::seal::existing_mac_key(host, profile) {
        Ok(key) => key,
        Err(error) => return LoadedQueue::Untrusted(error),
    };
    let raw = match fs::read(&journal) {
        Ok(raw) => raw,
        Err(error) => {
            return LoadedQueue::Untrusted(format!("read {}: {error}", journal.display()));
        }
    };
    let Some((body, mac)) = split_mac(&raw) else {
        return LoadedQueue::Untrusted("queue file has no seal".to_string());
    };
    if host_key
        .as_ref()
        .is_some_and(|key| ct_eq(&hmac_sha256(key, body), &mac))
    {
        return parse_journal(body);
    }
    if host_key.is_none() {
        LoadedQueue::Untrusted("queue file has no host seal key".to_string())
    } else {
        LoadedQueue::Untrusted("queue seal does not match".to_string())
    }
}

fn missing_journal() -> LoadedQueue {
    LoadedQueue::Untrusted(
        "needs-you queue file is missing; the host seal still remembers one".to_string(),
    )
}

// Called only after verifying the MAC over the original bytes.
fn decode_sealed<T: serde::de::DeserializeOwned>(
    body: &[u8],
    collection: &str,
) -> Result<T, String> {
    let mut data: serde_json::Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let old = match data["schema"].as_u64() {
        Some(1) => true,
        Some(2) => false,
        _ => return Err("unsupported sealed record schema".into()),
    };
    for row in data[collection]
        .as_array_mut()
        .ok_or("missing sealed record list")?
    {
        if old {
            let label = row["actor"]
                .as_str()
                .ok_or("missing legacy actor")?
                .to_owned();
            let binding_owner = row.pointer("/pending/binding").and_then(|binding| {
                let kind = serde_json::from_value(binding["actor_type"].clone()).ok()?;
                Some(RecordOwner::stored(kind, binding["actor_id"].as_str()?))
            });
            let details = if collection == "items" {
                &mut row["details"]
            } else {
                &mut *row
            };
            let number = details["pane_id"]
                .as_u64()
                .or_else(|| label.strip_prefix("pane:")?.parse().ok());
            let key = details
                .as_object_mut()
                .and_then(|obj| obj.remove("pane_incarnation"));
            let owner = match (number, key.as_ref().and_then(|v| v.as_str())) {
                (Some(n), Some(key)) => RecordOwner::Pane(
                    PanePrincipal::from_stored(&format!("pane:{n}#{key}"))
                        .ok_or("invalid sealed incarnation")?,
                ),
                _ if binding_owner.is_some() => binding_owner.expect("matched binding"),
                _ if number.is_some() || label.starts_with("pane:") => RecordOwner::Legacy(label),
                _ if label == super::principal::EXTERNAL_ID => {
                    RecordOwner::requester(&super::principal::Principal::External)
                }
                _ => RecordOwner::Host(label),
            };
            row["actor"] = serde_json::to_value(owner).map_err(|e| e.to_string())?;
        } else if collection == "records" {
            let owner = row
                .as_object_mut()
                .and_then(|obj| obj.remove("owner"))
                .ok_or("missing sealed run owner")?;
            row["actor"] = owner;
        }
    }
    data["schema"] = SCHEMA.into();
    serde_json::from_value(data).map_err(|e| e.to_string())
}

fn parse_journal(body: &[u8]) -> LoadedQueue {
    let parsed: DiskQueueOwned = match decode_sealed(body, "items") {
        Ok(parsed) => parsed,
        Err(error) => {
            return LoadedQueue::Untrusted(format!("queue file is not a host record: {error}"));
        }
    };
    if parsed.schema != SCHEMA {
        return LoadedQueue::Untrusted(format!("queue schema {} is not {SCHEMA}", parsed.schema));
    }
    match validate(&parsed.items) {
        Ok(items) => LoadedQueue::Items(items),
        Err(error) => LoadedQueue::Untrusted(error),
    }
}

pub(crate) fn save(
    host: &HostProcess,
    profile: &Path,
    items: &[RestoredItem],
) -> Result<(), String> {
    let host_dir = ensure_host_dir(profile)?;
    let key = super::seal::mac_key_bytes(host, profile)?;
    log::info!("needs_you: sealing journal with the host seal key");
    let disk: Vec<DiskItem> = items.iter().map(DiskItem::from).collect();
    let body = serde_json::to_vec(&DiskQueue {
        schema: SCHEMA,
        items: &disk,
    })
    .map_err(|error| error.to_string())?;
    let mac = hmac_sha256(&key, &body);
    let mut file_bytes = body;
    file_bytes.push(b'\n');
    file_bytes.extend_from_slice(b"#mac ");
    file_bytes.extend_from_slice(hex_encode(&mac).as_bytes());
    file_bytes.push(b'\n');
    let journal = host_dir.join("needs-you.json");
    super::seal::require_sealed_profile(host, profile)?;
    write_private(&journal, &file_bytes)?;
    store_tip(host, profile, &mac)?;
    log::info!(
        "needs_you: persisted {} open items to {}",
        items.len(),
        journal.display()
    );
    Ok(())
}

pub(crate) fn reset_after_key_rotation(host: &HostProcess, profile: &Path) -> Result<(), String> {
    let journal = journal_path(profile);
    if journal.is_file() {
        super::seal::copy_aside(&journal, "needs-you.json.before-key-rotation");
    }
    let continuations = continuations_path(profile);
    if continuations.is_file() {
        super::seal::copy_aside(&continuations, "continuations.json.before-key-rotation");
        fs::remove_file(&continuations)
            .map_err(|error| format!("remove {}: {error}", continuations.display()))?;
    }
    save(host, profile, &[])?;
    log::info!(
        "needs_you: reset unverified queue in {} after seal-key rotation",
        profile.display()
    );
    Ok(())
}

pub(crate) fn quarantine(profile: &Path) -> Option<PathBuf> {
    let path = journal_path(profile);
    if !path.exists() {
        return None;
    }
    let dest = path.with_file_name(format!(
        "needs-you.json.untrusted-{}-{}",
        crate::platform::clock::now_secs(),
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ));
    match fs::rename(&path, &dest) {
        Ok(()) => {
            log::info!(
                "needs_you: quarantined {} to {}",
                path.display(),
                dest.display()
            );
            Some(dest)
        }
        Err(error) => {
            log::error!(
                "needs_you: could not quarantine {}: {error}",
                path.display()
            );
            None
        }
    }
}

fn validate(items: &[DiskItem]) -> Result<Vec<RestoredItem>, String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if item.id.is_empty() || !seen.insert(item.id.clone()) {
            return Err(format!("queue item id {:?} is empty or repeated", item.id));
        }
        if item.origin_session.is_empty() {
            return Err(format!("queue item {} has no origin session", item.id));
        }
        if item.actor.label().is_empty() {
            return Err(format!("queue item {} has no actor", item.id));
        }
        match item.kind {
            NeedsYouKind::ApprovalClick => {
                let Some(pending) = &item.pending else {
                    return Err(format!(
                        "approval {} has no pending request and cannot be restored",
                        item.id
                    ));
                };
                if pending.binding.args_fingerprint.is_empty()
                    || pending.binding.actor.id().is_empty()
                    || pending.binding.target_id.is_empty()
                    || pending.tool.is_empty()
                {
                    return Err(format!("approval {} has an empty binding", item.id));
                }
            }
            NeedsYouKind::Question
            | NeedsYouKind::BlockedRun
            | NeedsYouKind::PermissionChange
            | NeedsYouKind::PermissionRequest
            | NeedsYouKind::Integrity
            | NeedsYouKind::Keychain
            | NeedsYouKind::AgentDone => {
                if item.pending.is_some() {
                    return Err(format!(
                        "item {} is not an approval and cannot carry a pending grant",
                        item.id
                    ));
                }
            }
        }
        out.push(RestoredItem {
            id: item.id.clone(),
            kind: item.kind,
            actor: item.actor.clone(),
            resource: item.resource.clone(),
            summary: item.summary.clone(),
            created_at: item.created_at,
            expires_at: item.expires_at,
            run_tag: item.run_tag.clone(),
            origin_session: item.origin_session.clone(),
            pending: item.pending.clone().map(|pending| RestoredPending {
                tool: pending.tool,
                input_summary: pending.input_summary,
                binding: pending.binding,
            }),
            details: item.details.clone(),
        });
    }
    Ok(out)
}

impl From<&RestoredItem> for DiskItem {
    fn from(item: &RestoredItem) -> Self {
        Self {
            id: item.id.clone(),
            kind: item.kind,
            actor: item.actor.clone(),
            resource: item.resource.clone(),
            summary: item.summary.clone(),
            created_at: item.created_at,
            expires_at: item.expires_at,
            run_tag: item.run_tag.clone(),
            origin_session: item.origin_session.clone(),
            pending: item.pending.as_ref().map(|pending| DiskPending {
                tool: pending.tool.clone(),
                input_summary: pending.input_summary.clone(),
                binding: pending.binding.clone(),
            }),
            details: item.details.clone(),
        }
    }
}

pub(crate) fn save_continuations(
    host: &HostProcess,
    profile: &Path,
    records: &[ContinuationRecord],
) -> Result<(), String> {
    ensure_host_dir(profile)?;
    let key = super::seal::mac_key_bytes(host, profile)?;
    let body = serde_json::to_vec(&DiskContinuations {
        schema: SCHEMA,
        records: records
            .iter()
            .map(|row| DiskRun {
                row,
                owner: &row.actor,
            })
            .collect(),
    })
    .map_err(|error| error.to_string())?;
    let mac = hmac_sha256(&key, &body);
    let mut file_bytes = body;
    file_bytes.push(b'\n');
    file_bytes.extend_from_slice(b"#mac ");
    file_bytes.extend_from_slice(hex_encode(&mac).as_bytes());
    file_bytes.push(b'\n');
    let path = continuations_path(profile);
    write_private(&path, &file_bytes)?;
    log::info!(
        "needs_you: persisted {} continuation records to {}",
        records.len(),
        path.display()
    );
    Ok(())
}

pub(crate) fn load_continuations(host: &HostProcess, profile: &Path) -> Vec<ContinuationRecord> {
    let path = continuations_path(profile);
    if is_symlink(&path) || !journal_is_file(&path) {
        return Vec::new();
    }
    let Ok(Some(key)) = super::seal::existing_mac_key(host, profile) else {
        return Vec::new();
    };
    let Ok(raw) = fs::read(&path) else {
        return Vec::new();
    };
    let Some((body, mac)) = split_mac(&raw) else {
        log::warn!(
            "needs_you: continuations file has no seal: {}",
            path.display()
        );
        return Vec::new();
    };
    if !ct_eq(&hmac_sha256(&key, body), &mac) {
        log::warn!(
            "needs_you: continuations file failed seal: {}",
            path.display()
        );
        return Vec::new();
    }
    match decode_sealed::<DiskContinuationsOwned>(body, "records") {
        Ok(parsed) if parsed.schema == SCHEMA => parsed.records,
        Ok(parsed) => {
            log::warn!(
                "needs_you: continuations schema {} != {SCHEMA}",
                parsed.schema
            );
            Vec::new()
        }
        Err(error) => {
            log::warn!("needs_you: continuations parse error: {error}");
            Vec::new()
        }
    }
}

fn continuations_path(profile: &Path) -> PathBuf {
    host_private_dir(profile).join("continuations.json")
}

fn journal_path(profile: &Path) -> PathBuf {
    host_private_dir(profile).join("needs-you.json")
}

pub(super) fn tip_account(profile: &Path) -> String {
    let canon = super::seal::canonical_profile_dir(profile).join("host");
    let hash = hex_encode(&Sha256::digest(canon.to_string_lossy().as_bytes()));
    format!("{TIP_PREFIX}{hash}")
}

fn read_tip(host: &HostProcess, profile: &Path) -> Result<Option<[u8; MAC_LEN]>, String> {
    match super::seal::needs_you_tip_lookup(host, profile)? {
        None => Ok(None),
        Some(text) => decode_mac(text.trim()),
    }
}

fn store_tip(host: &HostProcess, profile: &Path, mac: &[u8; MAC_LEN]) -> Result<(), String> {
    super::seal::store_needs_you_tip(host, profile, Some(&hex_encode(mac)))
}

fn tip_is_set(tip: &Option<[u8; MAC_LEN]>) -> bool {
    tip.as_ref()
        .is_some_and(|mac| mac.iter().any(|byte| *byte != 0))
}

fn decode_mac(text: &str) -> Result<Option<[u8; MAC_LEN]>, String> {
    let bytes = decode_hex(text).ok_or_else(|| "needs-you journal tip is not hex".to_string())?;
    if bytes.len() != MAC_LEN {
        return Err("needs-you journal tip has the wrong length".to_string());
    }
    let mut mac = [0u8; MAC_LEN];
    mac.copy_from_slice(&bytes);
    Ok(Some(mac))
}

fn ensure_host_dir(profile: &Path) -> Result<PathBuf, String> {
    let host = host_private_dir(profile);
    if is_symlink(&host) {
        return Err(format!("host directory {} is a symlink", host.display()));
    }
    fs::create_dir_all(&host).map_err(|error| format!("create {}: {error}", host.display()))?;
    tighten(&host, 0o700)?;
    Ok(host)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if is_symlink(path) {
        return Err(format!("{} is a symlink", path.display()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
        tighten(parent, 0o700)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .map_err(|error| format!("open {}: {error}", tmp.display()))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("write {}: {error}", tmp.display()))?;
    }
    tighten(&tmp, 0o600)?;
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        format!("rename into {}: {error}", path.display())
    })?;
    tighten(path, 0o600)?;
    Ok(())
}

fn tighten(path: &Path, mode: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|error| format!("chmod {mode:o} {}: {error}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

fn journal_is_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file())
}

fn split_mac(raw: &[u8]) -> Option<(&[u8], [u8; MAC_LEN])> {
    let text = std::str::from_utf8(raw).ok()?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    let (body, mac_line) = text.rsplit_once('\n')?;
    let hex = mac_line.strip_prefix("#mac ")?;
    let mac = decode_hex(hex)?;
    if mac.len() != MAC_LEN {
        return None;
    }
    let mut out = [0u8; MAC_LEN];
    out.copy_from_slice(&mac);
    Some((body.as_bytes(), out))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; MAC_LEN] {
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
    for index in 0..BLOCK {
        ipad[index] ^= key_block[index];
        opad[index] ^= key_block[index];
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
    let mut out = [0u8; MAC_LEN];
    out.copy_from_slice(&outer.finalize());
    out
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

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let hi = from_hex(bytes[index])?;
        let lo = from_hex(bytes[index + 1])?;
        out.push((hi << 4) | lo);
        index += 2;
    }
    Some(out)
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    const TEST_HOST: &crate::broker::custody::HostProcess =
        &crate::broker::custody::HostProcess::for_test();
    use super::*;

    #[test]
    fn sealed_v1_questions_runs_and_pending_approvals_keep_their_original_owner() {
        use crate::broker::gate::{NeedsYouResolution, PermissionMonitor};
        use crate::broker::principal::Principal;
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path();
        let original = PanePrincipal::for_test(7);
        let original_key = original.actor_id().split_once('#').unwrap().1;
        let replacement = Principal::Pane(PanePrincipal::for_test(7));
        save(TEST_HOST, profile, &[]).unwrap();
        let host_dir = ensure_host_dir(profile).unwrap();
        let key = super::super::seal::mac_key_bytes(TEST_HOST, profile).unwrap();
        let seal_fixture = |name: &str, value: serde_json::Value| {
            let body = serde_json::to_vec(&value).unwrap();
            let mac = hex_encode(&hmac_sha256(&key, &body));
            let mut sealed = body;
            sealed.extend_from_slice(format!("\n#mac {mac}\n").as_bytes());
            write_private(&host_dir.join(name), &sealed).unwrap();
        };
        let question = |id: &str, details| {
            json!({
                "id":id, "actor":"pane:7", "kind":"question", "resource":"workspace",
                "summary":"review", "created_at":1, "origin_session":"before", "details":details
            })
        };
        let mut approval = serde_json::to_value(DiskItem::from(&sample_item())).unwrap();
        approval["actor"] = json!("pane:7");
        approval["pending"]["binding"]["actor_id"] = json!("pane:7");
        seal_fixture(
            "needs-you.json",
            json!({"schema":1,"items":[
                question("old-keyed", json!({"pane_incarnation":original_key})),
                question("old-number", json!({"pane_id":7})), approval
            ]}),
        );
        let run = |id: &str, key: Option<&str>| {
            json!({
                "run_id":id, "actor":"pane:7","pane_id":7,"pane_incarnation":key,
                "target_resource":"workspace","state":"waiting_user","origin_session":"before",
                "needs_you_id":if key.is_some() {"old-keyed"} else {"old-number"},
                "created_at":1,"updated_at":1
            })
        };
        seal_fixture(
            "continuations.json",
            json!({"schema":1,"records":[
                run("run-keyed",Some(original_key)),run("run-number",None)
            ]}),
        );
        let monitor = PermissionMonitor::open_profile(profile).unwrap();
        let host = monitor.host_records();
        let old = monitor.records_for(&Principal::Pane(original));
        assert!(old.question("old-keyed").is_some());
        assert!(old.run("run-keyed").is_some());
        assert!(old.question("old-number").is_none());
        assert!(old.run("run-number").is_none());
        assert!(host.question("old-number").is_some());
        assert!(host.run("run-number").is_some());
        assert_eq!(
            host.question("req_1").unwrap().resolution,
            Some(NeedsYouResolution::Retired)
        );
        let next = monitor.records_for(&replacement);
        assert!(next.questions(false).is_empty());
        assert!(next.runs(None, None, None).is_empty());
        let fresh = monitor
            .file_needs_you(crate::broker::gate::NeedsYouFile {
                kind: crate::broker::gate::NeedsYouKind::Question,
                actor: RecordOwner::requester(&replacement),
                resource: "workspace".into(),
                summary: "fresh".into(),
                expires_at: None,
                run_tag: None,
            })
            .unwrap();
        assert!(next.question(&fresh).is_some());
        assert!(host.question(&fresh).is_some());
        assert!(host.question("old-keyed").is_some());
        assert!(host.question("old-number").is_some());
    }

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
    fn a_resolution_field_fails_the_seal_even_with_the_host_key() {
        let dir = tempfile::tempdir().unwrap();
        let item = sample_item();
        save(TEST_HOST, dir.path(), &[item]).unwrap();
        let key = crate::broker::seal::existing_mac_key(TEST_HOST, dir.path())
            .unwrap()
            .unwrap();
        let forged = br#"{"schema":1,"items":[{"id":"req_forged","kind":"approval_click","actor":"agent:chess","resource":"game-1","summary":"forged","created_at":1,"origin_session":"sess-forged","resolution":"approved","pending":null}]}"#;
        let mac = hmac_sha256(&key, forged);
        let mut file = forged.to_vec();
        file.push(b'\n');
        file.extend_from_slice(format!("#mac {}\n", hex_encode(&mac)).as_bytes());
        fs::write(journal_path(dir.path()), file).unwrap();
        match load(TEST_HOST, dir.path()) {
            LoadedQueue::Untrusted(reason) => {
                assert!(
                    reason.contains("not a host record") || reason.contains("resolution"),
                    "{reason}"
                );
            }
            other => panic!("a pre-approved record must not load: {other:?}"),
        }
    }

    #[test]
    fn journal_hmac_uses_the_host_permission_mac() {
        let dir = tempfile::tempdir().unwrap();
        save(TEST_HOST, dir.path(), &[sample_item()]).unwrap();
        assert!(!host_private_dir(dir.path()).join("seal.key").exists());
        let key = crate::broker::seal::existing_mac_key(TEST_HOST, dir.path())
            .unwrap()
            .unwrap();
        let raw = fs::read(journal_path(dir.path())).unwrap();
        let (body, mac) = split_mac(&raw).unwrap();
        assert!(ct_eq(&hmac_sha256(&key, body), &mac));
        // A stale entry that cannot be read as a key must not affect load or save.
        fs::create_dir(host_private_dir(dir.path()).join("seal.key")).unwrap();
        match load(TEST_HOST, dir.path()) {
            LoadedQueue::Items(items) => assert_eq!(items[0].id, "req_1"),
            other => panic!("sealed journal did not load: {other:?}"),
        }
        save(TEST_HOST, dir.path(), &[]).unwrap();
    }

    #[test]
    fn legacy_needs_you_key_is_never_read() {
        let dir = tempfile::tempdir().unwrap();
        save(TEST_HOST, dir.path(), &[]).unwrap();
        write_legacy_journal(dir.path(), &[sample_item()], &[9; 32]);
        let original = fs::read(journal_path(dir.path())).unwrap();
        assert!(matches!(load(TEST_HOST, dir.path()),
            LoadedQueue::Untrusted(reason) if reason == "queue seal does not match"));
        let monitor = crate::broker::gate::PermissionMonitor::for_profile(dir.path()).unwrap();
        let items = monitor.host_records().questions(true);
        assert!(items.iter().all(|item| item.id != "req_1"));
        assert!(items
            .iter()
            .any(|item| item.kind == NeedsYouKind::Integrity));
        let quarantined = fs::read_dir(host_private_dir(dir.path()))
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("needs-you.json.untrusted-")
            })
            .expect("quarantined journal");
        assert_eq!(fs::read(quarantined.path()).unwrap(), original);
    }

    #[test]
    fn a_legacy_only_profile_takes_the_normal_integrity_path() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy_journal(dir.path(), &[sample_item()], &[7; 32]);
        let LoadedQueue::Untrusted(with_key) = load(TEST_HOST, dir.path()) else {
            panic!("legacy-only journal was not rejected");
        };
        fs::remove_file(host_private_dir(dir.path()).join("seal.key")).unwrap();
        assert!(
            matches!(load(TEST_HOST, dir.path()), LoadedQueue::Untrusted(reason) if reason == with_key)
        );
        assert!(super::super::seal::existing_mac_key(TEST_HOST, dir.path())
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_legacy_tip_cannot_report_a_missing_journal() {
        let dir = tempfile::tempdir().unwrap();
        write_legacy_journal(dir.path(), &[], &[4; 32]);
        fs::remove_file(journal_path(dir.path())).unwrap();
        assert!(matches!(load(TEST_HOST, dir.path()), LoadedQueue::Empty));
        assert_eq!(read_tip(TEST_HOST, dir.path()).unwrap(), None);
    }

    fn write_legacy_journal(profile: &Path, items: &[RestoredItem], key: &[u8; 32]) {
        let host = ensure_host_dir(profile).unwrap();
        let disk: Vec<DiskItem> = items.iter().map(DiskItem::from).collect();
        let body = serde_json::to_vec(&DiskQueue {
            schema: SCHEMA,
            items: &disk,
        })
        .unwrap();
        let mac = hmac_sha256(key, &body);
        let mut file_bytes = body;
        file_bytes.push(b'\n');
        file_bytes.extend_from_slice(b"#mac ");
        file_bytes.extend_from_slice(hex_encode(&mac).as_bytes());
        file_bytes.push(b'\n');
        write_private(&host.join("needs-you.json"), &file_bytes).unwrap();
        let mut stored = key.to_vec();
        stored.extend_from_slice(&mac);
        write_private(&host.join("seal.key"), &stored).unwrap();
    }

    fn sample_item() -> RestoredItem {
        RestoredItem {
            id: "req_1".into(),
            kind: NeedsYouKind::ApprovalClick,
            actor: RecordOwner::requester(
                &super::super::principal::Principal::agent("agent:chess").unwrap(),
            ),
            resource: "game-1".into(),
            summary: "e2e4".into(),
            created_at: 10,
            expires_at: None,
            run_tag: Some("call-1".into()),
            origin_session: "sess-old".into(),
            pending: Some(RestoredPending {
                tool: "chess.play".into(),
                input_summary: "e2e4".into(),
                binding: ExactBinding {
                    agent_run: None,
                    actor: crate::broker::principal::Principal::agent("agent:chess")
                        .unwrap()
                        .into(),
                    actor_scope: super::super::ActorScope::User,
                    trust_origin: "host".into(),
                    workspace_root: PathBuf::from("/ws"),
                    target_type: super::super::TargetType::PluginConnector,
                    target_id: "chess.play".into(),
                    resource_scope: super::super::ResourceScope::Game,
                    resource_id: Some("game-1".into()),
                    args_fingerprint: "abc".into(),
                    session_id: Some("sess-old".into()),
                    package_id: "chess".into(),
                    instance_id: Some(4),
                    context_id: Some(9),
                    call_id: "call-1".into(),
                    operation_id: "op-1".into(),
                },
            }),
            details: None,
        }
    }
}
