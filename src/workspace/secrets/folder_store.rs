//! Backends for folder-scoped secrets.
//!
//! macOS uses the Keychain and Windows uses Credential Manager, both selected
//! by [`super::folder_store`]. Linux tries the Secret Service (via `secret-tool`,
//! which talks to `org.freedesktop.secrets`) and, when that daemon is missing
//! or locked, an AES-256-GCM file. The file is labeled as a fallback. Its key
//! lives outside the Plexi profile. Plaintext values are never written under
//! `.plexi`.

#[cfg(any(test, target_os = "linux"))]
use super::store::{NonDestructiveStore, SecretError, SecretStore};
#[cfg(any(test, target_os = "linux"))]
use ring::rand::SecureRandom;
#[cfg(any(test, target_os = "linux"))]
use std::collections::BTreeMap;
#[cfg(any(test, target_os = "linux"))]
use std::io::Write;
#[cfg(any(test, target_os = "linux"))]
use std::path::{Path, PathBuf};
#[cfg(any(test, target_os = "linux"))]
use std::sync::Mutex;
#[cfg(any(test, target_os = "linux"))]
use zeroize::Zeroizing;

/// On-disk label for the Linux encrypted-file fallback. It is not a secret.
#[cfg(any(test, target_os = "linux"))]
pub const ENCRYPTED_FILE_LABEL: &str = "encrypted-file fallback: not Secret Service and not an OS keyring; ciphertext only; the unlock key is stored outside the Plexi profile";

/// `PLEXI_FOLDER_SECRETS_BACKEND` test hook. `encrypted-file-fallback` selects
/// the labeled file and does not contact Secret Service. Any other non-empty
/// value is an error so a typo cannot fall through to the user keyring.
#[cfg(any(test, target_os = "linux"))]
pub fn folder_backend_override(value: Option<&str>) -> Result<Option<&'static str>, &'static str> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some("encrypted-file-fallback") => Ok(Some("encrypted-file-fallback")),
        Some(_) => Err("PLEXI_FOLDER_SECRETS_BACKEND must be unset or encrypted-file-fallback"),
    }
}

#[cfg(all(target_os = "linux", not(test)))]
const ENCRYPTED_FILE: &str = "folder-secrets.enc";
#[cfg(all(target_os = "linux", not(test)))]
const CHOICE_FILE: &str = "folder-secrets-backend.txt";
#[cfg(all(target_os = "linux", not(test)))]
const SERVICE_INDEX: &str = "folder-secrets-names.txt";

pub fn backend_label() -> String {
    #[cfg(test)]
    {
        "in-memory".to_string()
    }
    #[cfg(all(not(test), target_os = "linux"))]
    {
        linux_backend_label()
    }
    #[cfg(all(not(test), target_os = "macos"))]
    {
        "macos-keychain".to_string()
    }
    #[cfg(all(not(test), windows))]
    {
        "windows-credential-manager".to_string()
    }
    #[cfg(all(
        not(test),
        not(any(target_os = "linux", target_os = "macos", windows))
    ))]
    {
        "unavailable".to_string()
    }
}

#[cfg(all(target_os = "linux", not(test)))]
fn profile_dir() -> PathBuf {
    crate::config::config_dir()
}

#[cfg(all(target_os = "linux", not(test)))]
fn key_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("plexi")
        .join("folder-secret.key")
}

#[cfg(any(test, target_os = "linux"))]
#[derive(serde::Serialize, serde::Deserialize)]
struct EncryptedBlob {
    label: String,
    entries: BTreeMap<String, String>,
}

/// AES-256-GCM file. Account names are plaintext so `list` does not decrypt.
/// Values are not.
#[cfg(any(test, target_os = "linux"))]
pub struct EncryptedFileStore {
    pub path: PathBuf,
    pub key_path: PathBuf,
}

#[cfg(any(test, target_os = "linux"))]
impl EncryptedFileStore {
    #[cfg(not(test))]
    pub fn profile() -> Self {
        Self {
            path: profile_dir().join(ENCRYPTED_FILE),
            key_path: key_path(),
        }
    }

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|err| err.into_inner())
    }

    fn load_key(&self) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        match std::fs::read(&self.key_path) {
            Ok(bytes) if bytes.len() == 32 => Ok(Zeroizing::new(bytes)),
            Ok(_) => Err(SecretError::Backend(format!(
                "folder secret key at {} is not 32 bytes; refusing to overwrite it",
                self.key_path.display()
            ))),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => self.create_key(),
            Err(err) => Err(SecretError::Backend(format!(
                "read folder secret key {}: {err}",
                self.key_path.display()
            ))),
        }
    }

    fn create_key(&self) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        if let Some(parent) = self.key_path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                SecretError::Backend(format!("create {}: {err}", parent.display()))
            })?;
        }
        let mut bytes = [0u8; 32];
        ring::rand::SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| SecretError::Backend("failed to generate folder secret key".into()))?;
        write_private(&self.key_path, &bytes)?;
        log::info!(
            "folder_secrets: created encrypted-file key outside the profile at {}",
            self.key_path.display()
        );
        Ok(Zeroizing::new(bytes.to_vec()))
    }

    fn load(&self) -> Result<EncryptedBlob, SecretError> {
        let raw = match std::fs::read(&self.path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(EncryptedBlob {
                    label: ENCRYPTED_FILE_LABEL.to_string(),
                    entries: BTreeMap::new(),
                });
            }
            Err(err) => {
                return Err(SecretError::Backend(format!(
                    "read {}: {err}",
                    self.path.display()
                )));
            }
        };
        serde_json::from_slice(&raw).map_err(|err| {
            SecretError::Backend(format!(
                "{} is not a folder-secret file ({err}); refusing to rewrite it",
                self.path.display()
            ))
        })
    }

    fn save(&self, blob: &EncryptedBlob) -> Result<(), SecretError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                SecretError::Backend(format!("create {}: {err}", parent.display()))
            })?;
        }
        let body = serde_json::to_vec_pretty(blob)
            .map_err(|err| SecretError::Backend(format!("serialize folder secrets: {err}")))?;
        let tmp = self.path.with_extension("enc.tmp");
        write_private(&tmp, &body)?;
        std::fs::rename(&tmp, &self.path).map_err(|err| {
            let _ = std::fs::remove_file(&tmp);
            SecretError::Backend(format!("rename into {}: {err}", self.path.display()))
        })
    }

    fn decrypt(&self, account: &str, encoded: &str) -> Result<Zeroizing<String>, SecretError> {
        let key = self.load_key()?;
        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            encoded,
        )
        .map_err(|_| SecretError::Backend(format!("folder secret {account} is not valid ciphertext")))?;
        if bytes.len() < 12 + 16 {
            return Err(SecretError::Backend(format!(
                "folder secret {account} ciphertext is truncated"
            )));
        }
        let (nonce_bytes, cipher) = bytes.split_at(12);
        let unbound = ring::aead::UnboundKey::new(&ring::aead::AES_256_GCM, &key)
            .map_err(|_| SecretError::Backend("folder secret key rejected".into()))?;
        let key = ring::aead::LessSafeKey::new(unbound);
        let nonce = ring::aead::Nonce::try_assume_unique_for_key(nonce_bytes)
            .map_err(|_| SecretError::Backend("folder secret nonce rejected".into()))?;
        let mut in_out = cipher.to_vec();
        let plain = key
            .open_in_place(
                nonce,
                ring::aead::Aad::from(account.as_bytes()),
                &mut in_out,
            )
            .map_err(|_| SecretError::Backend(format!("folder secret {account} failed to decrypt")))?;
        let text = String::from_utf8(plain.to_vec()).map_err(|_| {
            SecretError::Backend(format!("folder secret {account} is not utf-8"))
        })?;
        Ok(Zeroizing::new(text))
    }

    fn encrypt(&self, account: &str, value: &str) -> Result<String, SecretError> {
        let key = self.load_key()?;
        let unbound = ring::aead::UnboundKey::new(&ring::aead::AES_256_GCM, &key)
            .map_err(|_| SecretError::Backend("folder secret key rejected".into()))?;
        let key = ring::aead::LessSafeKey::new(unbound);
        let mut nonce_bytes = [0u8; 12];
        ring::rand::SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|_| SecretError::Backend("failed to generate folder secret nonce".into()))?;
        let nonce = ring::aead::Nonce::assume_unique_for_key(nonce_bytes);
        let mut in_out = value.as_bytes().to_vec();
        key.seal_in_place_append_tag(
            nonce,
            ring::aead::Aad::from(account.as_bytes()),
            &mut in_out,
        )
        .map_err(|_| SecretError::Backend(format!("folder secret {account} failed to encrypt")))?;
        let mut packed = nonce_bytes.to_vec();
        packed.extend(in_out);
        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            packed,
        ))
    }
}

#[cfg(any(test, target_os = "linux"))]
impl NonDestructiveStore for EncryptedFileStore {
    fn get(&self, account: &str) -> Option<Zeroizing<String>> {
        let _guard = Self::lock();
        let blob = self.load().ok()?;
        let encoded = blob.entries.get(account)?;
        match self.decrypt(account, encoded) {
            Ok(value) => Some(value),
            Err(err) => {
                log::warn!("folder_secrets: decrypt failed for account={account}: {err}");
                None
            }
        }
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), SecretError> {
        let _guard = Self::lock();
        let mut blob = self.load()?;
        if blob.entries.contains_key(account) {
            return Err(SecretError::AlreadyExists(account.to_string()));
        }
        let cipher = self.encrypt(account, value)?;
        blob.label = ENCRYPTED_FILE_LABEL.to_string();
        blob.entries.insert(account.to_string(), cipher);
        self.save(&blob)
    }

    fn delete_if_value(&self, account: &str, expected: &str) -> Result<(), SecretError> {
        let _guard = Self::lock();
        let mut blob = self.load()?;
        let Some(encoded) = blob.entries.get(account).cloned() else {
            return Ok(());
        };
        let current = self.decrypt(account, &encoded)?;
        if current.as_str() != expected {
            return Err(SecretError::ValueChanged(account.to_string()));
        }
        blob.entries.remove(account);
        self.save(&blob)
    }

    fn list_with_prefix(&self, prefix: &str) -> Vec<String> {
        let _guard = Self::lock();
        self.load()
            .unwrap_or(EncryptedBlob {
                label: ENCRYPTED_FILE_LABEL.to_string(),
                entries: BTreeMap::new(),
            })
            .entries
            .into_keys()
            .filter(|account| account.starts_with(prefix))
            .collect()
    }

    fn scan_accounts(&self) -> Result<Vec<String>, SecretError> {
        let _guard = Self::lock();
        Ok(self.load()?.entries.into_keys().collect())
    }
}

#[cfg(any(test, target_os = "linux"))]
impl SecretStore for EncryptedFileStore {
    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        let _guard = Self::lock();
        let mut blob = self.load()?;
        let cipher = self.encrypt(account, value)?;
        blob.label = ENCRYPTED_FILE_LABEL.to_string();
        blob.entries.insert(account.to_string(), cipher);
        self.save(&blob)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        let _guard = Self::lock();
        let mut blob = self.load()?;
        blob.entries.remove(account);
        blob.label = ENCRYPTED_FILE_LABEL.to_string();
        self.save(&blob)
    }
}

#[cfg(any(test, target_os = "linux"))]
fn write_private(path: &Path, body: &[u8]) -> Result<(), SecretError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|err| SecretError::Backend(format!("open {}: {err}", path.display())))?;
        file.write_all(body)
            .and_then(|()| file.sync_all())
            .map_err(|err| SecretError::Backend(format!("write {}: {err}", path.display())))?;
        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, body)
            .map_err(|err| SecretError::Backend(format!("write {}: {err}", path.display())))
    }
}

#[cfg(all(target_os = "linux", not(test)))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinuxBackend {
    SecretService,
    EncryptedFile,
}

#[cfg(all(target_os = "linux", not(test)))]
fn choice_path() -> PathBuf {
    profile_dir().join(CHOICE_FILE)
}

#[cfg(all(target_os = "linux", not(test)))]
fn read_choice() -> Option<LinuxBackend> {
    let raw = std::fs::read_to_string(choice_path()).ok()?;
    let line = raw.lines().next()?.trim();
    match line {
        "secret-service" => Some(LinuxBackend::SecretService),
        "encrypted-file-fallback" => Some(LinuxBackend::EncryptedFile),
        _ => None,
    }
}

#[cfg(all(target_os = "linux", not(test)))]
fn write_choice(kind: LinuxBackend) {
    let body = match kind {
        LinuxBackend::SecretService => "secret-service\n",
        LinuxBackend::EncryptedFile => {
            "encrypted-file-fallback\nencrypted-file fallback: not Secret Service and not an OS keyring\n"
        }
    };
    if let Err(err) = write_private(&choice_path(), body.as_bytes()) {
        log::warn!("folder_secrets: could not record backend choice: {err}");
    }
}

#[cfg(all(target_os = "linux", not(test)))]
fn secret_service_available() -> bool {
    let output = match std::process::Command::new("secret-tool")
        .args(["lookup", "plexi.folder.probe", "absent"])
        .stdin(std::process::Stdio::null())
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            log::info!("folder_secrets: secret-tool is not available ({err})");
            return false;
        }
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    let unavailable = stderr.contains("was not provided by any .service")
        || stderr.contains("Cannot autolaunch")
        || stderr.contains("No session bus")
        || stderr.contains("Did not receive a reply")
        || stderr.contains("Cannot spawn a message bus");
    if unavailable {
        log::info!("folder_secrets: Secret Service is unavailable");
        return false;
    }
    true
}

#[cfg(all(target_os = "linux", not(test)))]
fn linux_kind() -> LinuxBackend {
    static KIND: std::sync::OnceLock<LinuxBackend> = std::sync::OnceLock::new();
    *KIND.get_or_init(|| {
        match folder_backend_override(std::env::var("PLEXI_FOLDER_SECRETS_BACKEND").ok().as_deref())
        {
            Ok(Some(_)) => {
                log::info!(
                    "folder_secrets: backend=encrypted-file-fallback (PLEXI_FOLDER_SECRETS_BACKEND); Secret Service not contacted"
                );
                return LinuxBackend::EncryptedFile;
            }
            Err(message) => {
                log::warn!(
                    "folder_secrets: {message}; using encrypted-file fallback and not contacting Secret Service"
                );
                return LinuxBackend::EncryptedFile;
            }
            Ok(None) => {}
        }
        if let Some(saved) = read_choice() {
            log::info!("folder_secrets: backend={}", backend_name(saved));
            return saved;
        }
        let chosen = if secret_service_available() {
            LinuxBackend::SecretService
        } else {
            LinuxBackend::EncryptedFile
        };
        write_choice(chosen);
        log::info!(
            "folder_secrets: selected backend={} label={}",
            backend_name(chosen),
            if chosen == LinuxBackend::EncryptedFile {
                ENCRYPTED_FILE_LABEL
            } else {
                "secret-service"
            }
        );
        chosen
    })
}

#[cfg(all(target_os = "linux", not(test)))]
fn backend_name(kind: LinuxBackend) -> &'static str {
    match kind {
        LinuxBackend::SecretService => "secret-service",
        LinuxBackend::EncryptedFile => "encrypted-file-fallback",
    }
}

#[cfg(all(target_os = "linux", not(test)))]
fn linux_backend_label() -> String {
    match linux_kind() {
        LinuxBackend::SecretService => "secret-service".to_string(),
        LinuxBackend::EncryptedFile => ENCRYPTED_FILE_LABEL.to_string(),
    }
}

#[cfg(all(target_os = "linux", not(test)))]
fn service_index_path() -> PathBuf {
    profile_dir().join(SERVICE_INDEX)
}

#[cfg(all(target_os = "linux", not(test)))]
fn read_service_index() -> Vec<String> {
    std::fs::read_to_string(service_index_path())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(all(target_os = "linux", not(test)))]
fn write_service_index(names: &[String]) -> Result<(), SecretError> {
    let body = names.join("\n");
    let mut body = body;
    if !body.is_empty() {
        body.push('\n');
    }
    write_private(&service_index_path(), body.as_bytes())
}

/// Run `secret-tool`. Stdout may be a secret; callers must not log it.
#[cfg(all(target_os = "linux", not(test)))]
fn secret_tool(args: &[&str], stdin: Option<&[u8]>) -> Result<std::process::Output, SecretError> {
    let mut cmd = std::process::Command::new("secret-tool");
    cmd.args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|err| SecretError::Backend(format!("secret-tool failed to start: {err}")))?;
    if let Some(bytes) = stdin {
        if let Some(mut pipe) = child.stdin.take() {
            pipe.write_all(bytes).map_err(|err| {
                SecretError::Backend(format!("secret-tool stdin write failed: {err}"))
            })?;
        }
    }
    child
        .wait_with_output()
        .map_err(|err| SecretError::Backend(format!("secret-tool wait failed: {err}")))
}

#[cfg(all(target_os = "linux", not(test)))]
fn service_get(account: &str) -> Option<Zeroizing<String>> {
    let output = secret_tool(&["lookup", "account", account], None).ok()?;
    if !output.status.success() {
        log::info!(
            "folder_secrets: secret-tool lookup miss account={account} status={}",
            output.status
        );
        return None;
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    }
    Some(Zeroizing::new(text))
}

#[cfg(all(target_os = "linux", not(test)))]
fn service_set(account: &str, value: &str) -> Result<(), SecretError> {
    let output = secret_tool(
        &[
            "store",
            "--label=plexi-folder-secret",
            "account",
            account,
        ],
        Some(value.as_bytes()),
    )?;
    if !output.status.success() {
        return Err(SecretError::Backend(format!(
            "secret-tool store failed status={}",
            output.status
        )));
    }
    let mut names = read_service_index();
    if !names.iter().any(|name| name == account) {
        names.push(account.to_string());
        write_service_index(&names)?;
    }
    Ok(())
}

#[cfg(all(target_os = "linux", not(test)))]
fn service_delete(account: &str) -> Result<(), SecretError> {
    let output = secret_tool(&["clear", "account", account], None)?;
    if !output.status.success() {
        log::info!(
            "folder_secrets: secret-tool clear account={account} status={}",
            output.status
        );
    }
    let names: Vec<String> = read_service_index()
        .into_iter()
        .filter(|name| name != account)
        .collect();
    write_service_index(&names)
}

#[cfg(all(target_os = "linux", not(test)))]
pub(super) struct LinuxFolderStore;

#[cfg(all(target_os = "linux", not(test)))]
impl NonDestructiveStore for LinuxFolderStore {
    fn get(&self, account: &str) -> Option<Zeroizing<String>> {
        match linux_kind() {
            LinuxBackend::SecretService => service_get(account),
            LinuxBackend::EncryptedFile => EncryptedFileStore::profile().get(account),
        }
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), SecretError> {
        if self.get(account).is_some() {
            return Err(SecretError::AlreadyExists(account.to_string()));
        }
        self_set(self, account, value)
    }

    fn delete_if_value(&self, account: &str, expected: &str) -> Result<(), SecretError> {
        match self.get(account) {
            None => Ok(()),
            Some(current) if current.as_str() == expected => SecretStore::delete(self, account),
            Some(_) => Err(SecretError::ValueChanged(account.to_string())),
        }
    }

    fn list_with_prefix(&self, prefix: &str) -> Vec<String> {
        match linux_kind() {
            LinuxBackend::SecretService => read_service_index()
                .into_iter()
                .filter(|account| account.starts_with(prefix))
                .collect(),
            LinuxBackend::EncryptedFile => EncryptedFileStore::profile().list_with_prefix(prefix),
        }
    }

    fn scan_accounts(&self) -> Result<Vec<String>, SecretError> {
        match linux_kind() {
            LinuxBackend::SecretService => Ok(read_service_index()),
            LinuxBackend::EncryptedFile => EncryptedFileStore::profile().scan_accounts(),
        }
    }
}

#[cfg(all(target_os = "linux", not(test)))]
fn self_set(store: &LinuxFolderStore, account: &str, value: &str) -> Result<(), SecretError> {
    SecretStore::set(store, account, value)
}

#[cfg(all(target_os = "linux", not(test)))]
impl SecretStore for LinuxFolderStore {
    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        match linux_kind() {
            LinuxBackend::SecretService => service_set(account, value),
            LinuxBackend::EncryptedFile => EncryptedFileStore::profile().set(account, value),
        }
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        match linux_kind() {
            LinuxBackend::SecretService => service_delete(account),
            LinuxBackend::EncryptedFile => EncryptedFileStore::profile().delete(account),
        }
    }
}

#[cfg(all(target_os = "linux", not(test)))]
pub(super) fn platform_folder_store() -> &'static dyn SecretStore {
    static STORE: LinuxFolderStore = LinuxFolderStore;
    &STORE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_file_roundtrip_keeps_plaintext_out_of_the_file_and_the_key() {
        let dir = tempfile::tempdir().expect("dir");
        let store = EncryptedFileStore {
            path: dir.path().join("folder-secrets.enc"),
            key_path: dir.path().join("keydir").join("folder-secret.key"),
        };
        let value = "folder-secret-ciphertext-check-9f3c2a";
        store
            .set("plexi:folder:aabb:FOLDER_E2E_SECRET", value)
            .expect("set");
        let file = std::fs::read(&store.path).expect("read file");
        let key = std::fs::read(&store.key_path).expect("read key");
        let file_text = String::from_utf8_lossy(&file);
        assert!(
            !file_text.contains(value),
            "ciphertext file contains the plaintext secret"
        );
        assert!(
            file_text.contains("encrypted-file fallback"),
            "fallback file is not labeled: {file_text}"
        );
        assert!(!key.windows(value.len()).any(|window| window == value.as_bytes()));
        let got = store
            .get("plexi:folder:aabb:FOLDER_E2E_SECRET")
            .expect("get");
        assert_eq!(got.as_str(), value);
        store
            .delete("plexi:folder:aabb:FOLDER_E2E_SECRET")
            .expect("delete");
        assert!(store.get("plexi:folder:aabb:FOLDER_E2E_SECRET").is_none());
    }

    #[test]
    fn folder_backend_override_accepts_only_the_file_fallback() {
        assert_eq!(folder_backend_override(None).unwrap(), None);
        assert_eq!(folder_backend_override(Some("")).unwrap(), None);
        assert_eq!(folder_backend_override(Some("  ")).unwrap(), None);
        assert_eq!(
            folder_backend_override(Some("encrypted-file-fallback")).unwrap(),
            Some("encrypted-file-fallback")
        );
        assert!(folder_backend_override(Some("secret-service")).is_err());
        assert!(folder_backend_override(Some("macos-keychain")).is_err());
    }

    #[test]
    fn folder_secrets_e2e_snapshots_the_user_keychain_without_rewriting_it() {
        let script = include_str!("../../../scripts/folder-secrets-e2e.sh");
        assert!(
            script.contains("security default-keychain"),
            "e2e must record the default keychain"
        );
        assert!(
            script.contains("security list-keychains"),
            "e2e must record the keychain search list"
        );
        assert!(
            script.contains("encrypted-file-fallback"),
            "e2e must be able to force the file backend"
        );
        for forbidden in [
            "create-keychain",
            "delete-keychain",
            "list-keychains -s",
            "default-keychain -s",
            "default-keychain \"",
            "set-keychain-search-list",
        ] {
            assert!(
                !script.contains(forbidden),
                "e2e must not rewrite the user keychain via {forbidden}"
            );
        }
    }
}
