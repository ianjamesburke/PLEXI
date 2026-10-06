//! One model credential per tenant.
//!
//! The credential lives in a mode-0600 file under the channel profile. That
//! file is the Linux secret-store stub, and this slice uses it on every
//! platform so a keychain dialog never opens. The value is never written into
//! an image, a log, or a command argument. [`credential_usable`] is false once
//! [`TenantVault::revoke`] has removed it.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

const ACCOUNT_PREFIX: &str = "plexi:tenant:";
const ACCOUNT_SUFFIX: &str = ":model";

pub struct TenantVault {
    path: PathBuf,
}

impl TenantVault {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn for_profile() -> Self {
        Self::open(
            crate::config::config_dir()
                .join("cloud-house")
                .join("vault.json"),
        )
    }

    pub fn set(&self, tenant: &str, secret: &str) -> Result<String, String> {
        reject_empty(secret)?;
        let mut map = self.load()?;
        let account = account(tenant);
        if map.contains_key(&account) {
            return Err(format!(
                "tenant {tenant} already has a model credential; rotate it"
            ));
        }
        map.insert(account, secret.to_string());
        self.save(&map)?;
        let fingerprint = fingerprint(secret);
        log::info!("cloud agent: vault set tenant={tenant} fingerprint={fingerprint}");
        Ok(fingerprint)
    }

    pub fn rotate(&self, tenant: &str, secret: &str) -> Result<String, String> {
        reject_empty(secret)?;
        let mut map = self.load()?;
        let account = account(tenant);
        if !map.contains_key(&account) {
            return Err(format!("tenant {tenant} has no model credential to rotate"));
        }
        map.insert(account, secret.to_string());
        self.save(&map)?;
        let fingerprint = fingerprint(secret);
        log::info!("cloud agent: vault rotate tenant={tenant} fingerprint={fingerprint}");
        Ok(fingerprint)
    }

    /// Removes the tenant credential. A second call is success: the key is
    /// already unusable.
    pub fn revoke(&self, tenant: &str) -> Result<(), String> {
        let mut map = self.load()?;
        let removed = map.remove(&account(tenant)).is_some();
        if removed {
            self.save(&map)?;
            log::info!("cloud agent: vault revoke tenant={tenant}");
        } else {
            log::info!("cloud agent: vault revoke tenant={tenant} already-absent");
        }
        Ok(())
    }

    pub fn get(&self, tenant: &str) -> Result<Option<Zeroizing<String>>, String> {
        let map = self.load()?;
        Ok(map
            .get(&account(tenant))
            .filter(|value| credential_usable(Some(value)))
            .map(|value| Zeroizing::new(value.clone())))
    }

    pub fn status(&self, tenant: &str) -> Result<serde_json::Value, String> {
        let secret = self.get(tenant)?;
        let fingerprint = secret.as_deref().map(|value| fingerprint(value));
        Ok(serde_json::json!({
            "tenant": tenant,
            "present": secret.is_some(),
            "fingerprint": fingerprint,
        }))
    }
}

pub fn credential_usable(secret: Option<&str>) -> bool {
    secret.is_some_and(|value| !value.is_empty())
}

pub fn fingerprint(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn read_secret_from_stdin() -> Result<String, String> {
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .map_err(|error| format!("read credential: {error}"))?;
    if raw.ends_with('\n') {
        raw.pop();
        if raw.ends_with('\r') {
            raw.pop();
        }
    }
    reject_empty(&raw)?;
    Ok(raw)
}

fn reject_empty(secret: &str) -> Result<(), String> {
    if secret.is_empty() {
        Err("model credential is empty".into())
    } else {
        Ok(())
    }
}

fn account(tenant: &str) -> String {
    format!("{ACCOUNT_PREFIX}{tenant}{ACCOUNT_SUFFIX}")
}

impl TenantVault {
    fn load(&self) -> Result<BTreeMap<String, String>, String> {
        match fs::read_to_string(&self.path) {
            Ok(raw) => serde_json::from_str(&raw).map_err(|error| {
                format!(
                    "vault {} is not valid JSON ({error}); refusing to rewrite it",
                    self.path.display()
                )
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(format!("read {}: {error}", self.path.display())),
        }
    }

    fn save(&self, map: &BTreeMap<String, String>) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let body =
            serde_json::to_vec_pretty(map).map_err(|error| format!("serialize vault: {error}"))?;
        {
            let mut file = open_secret(&tmp)?;
            file.write_all(&body)
                .and_then(|()| file.sync_all())
                .map_err(|error| format!("write {}: {error}", tmp.display()))?;
        }
        fs::rename(&tmp, &self.path).map_err(|error| {
            let _ = fs::remove_file(&tmp);
            format!("rename into {}: {error}", self.path.display())
        })?;
        Ok(())
    }
}

fn open_secret(path: &Path) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| format!("open {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault() -> (PathBuf, TenantVault) {
        let dir = std::env::temp_dir().join(format!("plexi-vault-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vault.json");
        (dir, TenantVault::open(&path))
    }

    #[test]
    fn revoked_model_key_stops_working() {
        let (dir, vault) = temp_vault();
        let first = vault.set("local", "sk-one").unwrap();
        assert!(credential_usable(
            vault.get("local").unwrap().as_deref().map(String::as_str)
        ));
        assert_eq!(first, fingerprint("sk-one"));
        let second = vault.rotate("local", "sk-two").unwrap();
        assert_ne!(first, second);
        assert_eq!(
            vault.get("local").unwrap().as_deref().map(String::as_str),
            Some("sk-two")
        );
        vault.set("other", "sk-other").unwrap();
        vault.revoke("local").unwrap();
        assert!(vault.get("local").unwrap().is_none());
        assert!(!credential_usable(
            vault.get("local").unwrap().as_deref().map(String::as_str)
        ));
        assert_eq!(
            vault.get("other").unwrap().as_deref().map(String::as_str),
            Some("sk-other")
        );
        vault.revoke("local").unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn vault_file_is_mode_0600_and_set_does_not_overwrite() {
        let (dir, vault) = temp_vault();
        vault.set("local", "sk-one").unwrap();
        assert!(vault.set("local", "sk-other").is_err());
        assert_eq!(
            vault.get("local").unwrap().as_deref().map(String::as_str),
            Some("sk-one")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join("vault.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = fs::remove_dir_all(dir);
    }
}
