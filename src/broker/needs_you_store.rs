//! Host-only persistence for the Needs you queue and its pending approvals.
//!
//! The queue lives under `<profile>/host`, mode `0700` on Unix. That directory
//! is not the grant file and not `secrets.json`, and pane environment
//! construction does not receive its path. The journal is HMAC-SHA256 sealed
//! with a key that lives only in that directory. A journal that fails the MAC,
//! is deleted while a key remembers it, or carries a resolution is quarantined.
//! Loading it never writes a grant.

use super::gate::NeedsYouKind;
use super::ExactBinding;
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const SCHEMA: u32 = 1;
const KEY_LEN: usize = 32;
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
    pub actor: String,
    pub resource: String,
    pub summary: String,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub run_tag: Option<String>,
    pub origin_session: String,
    pub pending: Option<RestoredPending>,
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
    actor: String,
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
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DiskPending {
    tool: String,
    input_summary: String,
    binding: ExactBinding,
}

struct KeyMaterial {
    key: [u8; KEY_LEN],
    /// MAC of the last journal the host wrote. A missing journal with a
    /// non-zero tip was deleted out from under the host.
    last_mac: [u8; MAC_LEN],
}

pub(crate) fn load(profile: &Path) -> LoadedQueue {
    let journal = journal_path(profile);
    let key_path = key_path(profile);
    if is_symlink(&journal) || is_symlink(&key_path) || is_symlink(&host_private_dir(profile)) {
        return LoadedQueue::Untrusted("host queue path is a symlink".to_string());
    }
    let key = match read_key(&key_path) {
        Ok(Some(key)) => key,
        Ok(None) => {
            return if journal_is_file(&journal) {
                LoadedQueue::Untrusted("queue file has no host seal key".to_string())
            } else {
                LoadedQueue::Empty
            };
        }
        Err(error) => return LoadedQueue::Untrusted(error),
    };
    if !journal_is_file(&journal) {
        if key.last_mac.iter().any(|byte| *byte != 0) {
            return LoadedQueue::Untrusted(
                "needs-you queue file is missing; the host seal still remembers one".to_string(),
            );
        }
        return LoadedQueue::Empty;
    }
    let raw = match fs::read(&journal) {
        Ok(raw) => raw,
        Err(error) => {
            return LoadedQueue::Untrusted(format!("read {}: {error}", journal.display()));
        }
    };
    let Some((body, mac)) = split_mac(&raw) else {
        return LoadedQueue::Untrusted("queue file has no seal".to_string());
    };
    let expected = hmac_sha256(&key.key, body);
    if !ct_eq(&expected, &mac) {
        return LoadedQueue::Untrusted("queue seal does not match".to_string());
    }
    let parsed: DiskQueueOwned = match serde_json::from_slice(body) {
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

pub(crate) fn save(profile: &Path, items: &[RestoredItem]) -> Result<(), String> {
    let host = ensure_host_dir(profile)?;
    let key_path = host.join("seal.key");
    let mut key = match read_key(&key_path)? {
        Some(key) => key,
        None => {
            let created = KeyMaterial {
                key: fresh_key(),
                last_mac: [0u8; MAC_LEN],
            };
            write_key(&key_path, &created)?;
            log::info!("needs_you: created host seal key {}", key_path.display());
            created
        }
    };
    let disk: Vec<DiskItem> = items.iter().map(DiskItem::from).collect();
    let body = serde_json::to_vec(&DiskQueue {
        schema: SCHEMA,
        items: &disk,
    })
    .map_err(|error| error.to_string())?;
    let mac = hmac_sha256(&key.key, &body);
    let mut file_bytes = body;
    file_bytes.push(b'\n');
    file_bytes.extend_from_slice(b"#mac ");
    file_bytes.extend_from_slice(hex_encode(&mac).as_bytes());
    file_bytes.push(b'\n');
    let journal = host.join("needs-you.json");
    write_private(&journal, &file_bytes)?;
    key.last_mac = mac;
    write_key(&key_path, &key)?;
    log::info!(
        "needs_you: persisted {} open items to {}",
        items.len(),
        journal.display()
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
        if item.actor.is_empty() {
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
                    || pending.binding.actor_id.is_empty()
                    || pending.binding.target_id.is_empty()
                    || pending.tool.is_empty()
                {
                    return Err(format!("approval {} has an empty binding", item.id));
                }
            }
            NeedsYouKind::Question | NeedsYouKind::BlockedRun | NeedsYouKind::Integrity => {
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
        }
    }
}

fn journal_path(profile: &Path) -> PathBuf {
    host_private_dir(profile).join("needs-you.json")
}

fn key_path(profile: &Path) -> PathBuf {
    host_private_dir(profile).join("seal.key")
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

fn read_key(path: &Path) -> Result<Option<KeyMaterial>, String> {
    if is_symlink(path) {
        return Err(format!("seal key {} is a symlink", path.display()));
    }
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("stat {}: {error}", path.display())),
    };
    if !meta.file_type().is_file() {
        return Err(format!("seal key {} is not a file", path.display()));
    }
    if !is_private(&meta) {
        return Err(format!(
            "seal key {} is readable outside the host user",
            path.display()
        ));
    }
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    if bytes.len() != KEY_LEN + MAC_LEN && bytes.len() != KEY_LEN {
        return Err(format!("seal key {} has the wrong length", path.display()));
    }
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&bytes[..KEY_LEN]);
    let mut last_mac = [0u8; MAC_LEN];
    if bytes.len() == KEY_LEN + MAC_LEN {
        last_mac.copy_from_slice(&bytes[KEY_LEN..]);
    }
    Ok(Some(KeyMaterial { key, last_mac }))
}

fn write_key(path: &Path, key: &KeyMaterial) -> Result<(), String> {
    if is_symlink(path) {
        return Err(format!("seal key {} is a symlink", path.display()));
    }
    let mut bytes = Vec::with_capacity(KEY_LEN + MAC_LEN);
    bytes.extend_from_slice(&key.key);
    bytes.extend_from_slice(&key.last_mac);
    write_private(path, &bytes)
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

fn is_private(meta: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o077 == 0
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        true
    }
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

fn fresh_key() -> [u8; KEY_LEN] {
    let mut out = [0u8; KEY_LEN];
    out[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out
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
    fn a_resolution_field_fails_the_seal_even_with_the_host_key() {
        let dir = tempfile::tempdir().unwrap();
        let item = sample_item();
        save(dir.path(), &[item]).unwrap();
        let key = read_key(&key_path(dir.path())).unwrap().unwrap();
        let forged = br#"{"schema":1,"items":[{"id":"req_forged","kind":"approval_click","actor":"agent:chess","resource":"game-1","summary":"forged","created_at":1,"origin_session":"sess-forged","resolution":"approved","pending":null}]}"#;
        let mac = hmac_sha256(&key.key, forged);
        let mut file = forged.to_vec();
        file.push(b'\n');
        file.extend_from_slice(format!("#mac {}\n", hex_encode(&mac)).as_bytes());
        fs::write(journal_path(dir.path()), file).unwrap();
        match load(dir.path()) {
            LoadedQueue::Untrusted(reason) => {
                assert!(
                    reason.contains("not a host record") || reason.contains("resolution"),
                    "{reason}"
                );
            }
            other => panic!("a pre-approved record must not load: {other:?}"),
        }
    }

    fn sample_item() -> RestoredItem {
        RestoredItem {
            id: "req_1".into(),
            kind: NeedsYouKind::ApprovalClick,
            actor: "agent:chess".into(),
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
                    actor_type: super::super::ActorType::Agent,
                    actor_id: "agent:chess".into(),
                    actor_scope: super::super::ActorScope::User,
                    trust_origin: "host".into(),
                    workspace_root: PathBuf::from("/ws"),
                    target_type: super::super::TargetType::AppConnector,
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
        }
    }
}
