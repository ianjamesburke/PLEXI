//! Host seal key custody.
//!
//! The permission MAC and the audit-chain tip are not user secrets. They are
//! never written to `secrets.json` and they are not readable with
//! `plexi secret get`. The account namespace `plexi:host:*` is reserved.
//!
//! - Tests use a process-local mock. They do not open a keychain or a session bus.
//! - Linux stores the key only in Secret Service. If that service is missing,
//!   sealing fails and says why. There is no plaintext fallback.
//! - macOS stores it in the keychain under service `plexi-host-seal`, created
//!   by this binary. `PLEXI_KEYCHAIN_PATH` selects a throwaway keychain file
//!   and never falls back to the login keychain. The `security` tool is not
//!   the creating binary, so a direct keychain read does not yield the key.
//! - Windows stores it in Credential Manager under `plexi-host-seal/`, which
//!   `plexi secret get` does not read.

#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::Mutex;
use zeroize::Zeroizing;

use crate::workspace::secrets::system_store;

/// Item name of the MAC key inside the host store. Not a user-secret account.
pub(crate) const MAC_ITEM: &str = "permission-mac";

/// Service name on macOS and the Credential Manager prefix on Windows.
/// Distinct from the user-secret service `plexi`.
#[cfg(any(target_os = "macos", windows))]
const HOST_SERVICE: &str = "plexi-host-seal";

/// Why Linux will not write a plaintext seal key.
///
/// macOS and Windows have their own host stores, so a release build on those
/// platforms does not call this. Tests on every platform do.
#[cfg(any(test, not(any(target_os = "macos", windows))))]
pub(crate) fn plaintext_seal_refusal(detail: &str) -> String {
    format!(
        "Linux refuses to seal the permission MAC with a plaintext key in secrets.json. \
         The seal key is stored only in Secret Service (org.freedesktop.secrets). \
         Secret Service is not available: {detail}"
    )
}

/// Remove `plexi:host:*` from the user secret store so `cat secrets.json` and
/// `plexi secret get` cannot yield a key an older build wrote there.
pub(crate) fn scrub_user_secret_host_namespace() {
    let store = system_store();
    let accounts = store.list_with_prefix("plexi:host:");
    if accounts.is_empty() {
        return;
    }
    for account in accounts {
        match store.delete(&account) {
            Ok(()) => log::info!(
                "permission_seal: removed {account} from the user secret store"
            ),
            Err(error) => log::error!(
                "permission_seal: could not remove {account} from the user secret store: {error}"
            ),
        }
    }
}

pub(crate) fn get(account: &str) -> Result<Option<Zeroizing<String>>, String> {
    backend().get(account)
}

pub(crate) fn add_new(account: &str, value: &str) -> Result<(), String> {
    backend().add_new(account, value)
}

pub(crate) fn set(account: &str, value: &str) -> Result<(), String> {
    backend().set(account, value)
}

pub(crate) fn delete(account: &str) -> Result<(), String> {
    backend().delete(account)
}

fn backend() -> &'static dyn HostKeyBackend {
    #[cfg(test)]
    {
        use std::sync::OnceLock;
        static MEMORY: OnceLock<MemoryBackend> = OnceLock::new();
        MEMORY.get_or_init(MemoryBackend::default)
    }
    #[cfg(all(target_os = "linux", not(test)))]
    {
        static LINUX: LinuxBackend = LinuxBackend;
        &LINUX
    }
    #[cfg(all(target_os = "macos", not(test)))]
    {
        static MAC: MacBackend = MacBackend;
        &MAC
    }
    #[cfg(all(windows, not(test)))]
    {
        static WIN: WindowsBackend = WindowsBackend;
        &WIN
    }
    #[cfg(all(
        not(test),
        not(any(target_os = "linux", target_os = "macos", windows))
    ))]
    {
        static UNSUPPORTED: UnsupportedBackend = UnsupportedBackend;
        &UNSUPPORTED
    }
}

trait HostKeyBackend: Sync + Send {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, String>;
    fn add_new(&self, account: &str, value: &str) -> Result<(), String>;
    fn set(&self, account: &str, value: &str) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}

#[cfg(test)]
#[derive(Default)]
struct MemoryBackend {
    values: Mutex<BTreeMap<String, String>>,
}

#[cfg(test)]
impl HostKeyBackend for MemoryBackend {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, String> {
        Ok(self
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(account)
            .cloned()
            .map(Zeroizing::new))
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), String> {
        let mut map = self.values.lock().unwrap_or_else(|error| error.into_inner());
        if map.contains_key(account) {
            return Err(format!("host key already exists: {account}"));
        }
        map.insert(account.to_string(), value.to_string());
        Ok(())
    }

    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        self.values
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(account.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        self.values
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(account);
        Ok(())
    }
}

#[cfg(all(target_os = "linux", not(test)))]
struct LinuxBackend;

#[cfg(all(target_os = "linux", not(test)))]
impl HostKeyBackend for LinuxBackend {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, String> {
        linux::get(account)
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), String> {
        linux::add_new(account, value)
    }

    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        linux::set(account, value)
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        linux::delete(account)
    }
}

#[cfg(all(target_os = "macos", not(test)))]
struct MacBackend;

#[cfg(all(target_os = "macos", not(test)))]
impl HostKeyBackend for MacBackend {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, String> {
        mac::get(account)
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), String> {
        mac::add_new(account, value)
    }

    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        mac::set(account, value)
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        mac::delete(account)
    }
}

#[cfg(all(windows, not(test)))]
struct WindowsBackend;

#[cfg(all(windows, not(test)))]
impl HostKeyBackend for WindowsBackend {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, String> {
        windows::get(account)
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), String> {
        windows::add_new(account, value)
    }

    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        windows::set(account, value)
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        windows::delete(account)
    }
}

#[cfg(all(
    not(test),
    not(any(target_os = "linux", target_os = "macos", windows))
))]
struct UnsupportedBackend;

#[cfg(all(
    not(test),
    not(any(target_os = "linux", target_os = "macos", windows))
))]
impl HostKeyBackend for UnsupportedBackend {
    fn get(&self, _account: &str) -> Result<Option<Zeroizing<String>>, String> {
        Err(plaintext_seal_refusal(
            "this platform has no host seal-key store",
        ))
    }

    fn add_new(&self, _account: &str, _value: &str) -> Result<(), String> {
        Err(plaintext_seal_refusal(
            "this platform has no host seal-key store",
        ))
    }

    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        self.add_new(account, value)
    }

    fn delete(&self, _account: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(all(target_os = "linux", not(test)))]
mod linux {
    use super::plaintext_seal_refusal;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use zeroize::Zeroizing;
    use zbus::blocking::Connection;
    use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};

    const SCHEMA: &str = "com.plexi.HostSeal";

    fn session() -> Result<Connection, String> {
        Connection::session().map_err(|error| plaintext_seal_refusal(&error.to_string()))
    }

    fn call<B>(
        connection: &Connection,
        path: &str,
        interface: &str,
        method: &str,
        body: &B,
    ) -> Result<zbus::Message, String>
    where
        B: serde::ser::Serialize + zbus::zvariant::DynamicType,
    {
        connection
            .call_method(
                Some("org.freedesktop.secrets"),
                path,
                Some(interface),
                method,
                body,
            )
            .map_err(|error| plaintext_seal_refusal(&error.to_string()))
    }

    struct Session {
        connection: Connection,
        session: OwnedObjectPath,
        collection: OwnedObjectPath,
    }

    fn open_session() -> Result<Session, String> {
        let connection = session()?;
        let reply = call(
            &connection,
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
            "OpenSession",
            &("plain", Value::from("")),
        )?;
        let (_output, session_path): (Value<'_>, OwnedObjectPath) = reply
            .body()
            .deserialize()
            .map_err(|error| plaintext_seal_refusal(&error.to_string()))?;
        let collection = collection_path(&connection)?;
        Ok(Session {
            connection,
            session: session_path,
            collection,
        })
    }

    fn collection_path(connection: &Connection) -> Result<OwnedObjectPath, String> {
        for alias in ["default", "session"] {
            let reply = call(
                connection,
                "/org/freedesktop/secrets",
                "org.freedesktop.Secret.Service",
                "ReadAlias",
                &(alias,),
            )?;
            let path: OwnedObjectPath = reply
                .body()
                .deserialize()
                .map_err(|error| plaintext_seal_refusal(&error.to_string()))?;
            if path.as_str() != "/" {
                log::info!("permission_seal: secret service collection alias {alias}");
                return Ok(path);
            }
        }
        Err(plaintext_seal_refusal(
            "Secret Service has no default or session collection",
        ))
    }

    fn attributes(account: &str) -> HashMap<&str, &str> {
        let mut attributes = HashMap::new();
        attributes.insert("xdg:schema", SCHEMA);
        attributes.insert("item", account);
        attributes
    }

    fn search(session: &Session, account: &str) -> Result<Vec<OwnedObjectPath>, String> {
        let reply = call(
            &session.connection,
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
            "SearchItems",
            &(attributes(account),),
        )?;
        let (unlocked, _locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = reply
            .body()
            .deserialize()
            .map_err(|error| plaintext_seal_refusal(&error.to_string()))?;
        Ok(unlocked)
    }

    fn read_item(session: &Session, item: &OwnedObjectPath) -> Result<Zeroizing<String>, String> {
        let items = vec![item.clone()];
        let reply = call(
            &session.connection,
            "/org/freedesktop/secrets",
            "org.freedesktop.Secret.Service",
            "GetSecrets",
            &(&items, &session.session),
        )?;
        let secrets: HashMap<OwnedObjectPath, (OwnedObjectPath, Vec<u8>, Vec<u8>, String)> = reply
            .body()
            .deserialize()
            .map_err(|error| plaintext_seal_refusal(&error.to_string()))?;
        let Some((_, (_, _, value, _))) = secrets.into_iter().next() else {
            return Err(plaintext_seal_refusal(
                "Secret Service returned no secret for the host seal item",
            ));
        };
        let text = String::from_utf8(value)
            .map_err(|_| plaintext_seal_refusal("host seal item is not utf-8"))?;
        Ok(Zeroizing::new(text))
    }

    fn prompt_blocks(path: &OwnedObjectPath) -> bool {
        path.as_str() != "/"
    }

    fn get_unlocked(account: &str) -> Result<Option<Zeroizing<String>>, String> {
        let session = open_session()?;
        let found = search(&session, account)?;
        let Some(item) = found.first() else {
            return Ok(None);
        };
        read_item(&session, item).map(Some)
    }

    fn add_new_unlocked(account: &str, value: &str) -> Result<(), String> {
        let session = open_session()?;
        if !search(&session, account)?.is_empty() {
            return Err(format!("host key already exists: {account}"));
        }
        create_item(&session, account, value, false)
    }

    fn set_unlocked(account: &str, value: &str) -> Result<(), String> {
        let session = open_session()?;
        create_item(&session, account, value, true)
    }

    fn create_item(session: &Session, account: &str, value: &str, replace: bool) -> Result<(), String> {
        let mut properties: HashMap<&str, Value> = HashMap::new();
        properties.insert(
            "org.freedesktop.Secret.Item.Label",
            Value::from(format!("plexi-host-seal:{account}")),
        );
        properties.insert(
            "org.freedesktop.Secret.Item.Attributes",
            Value::from(attributes(account)),
        );
        let session_path = ObjectPath::try_from(session.session.as_str())
            .map_err(|error| plaintext_seal_refusal(&error.to_string()))?;
        let secret = (
            session_path,
            Vec::<u8>::new(),
            value.as_bytes().to_vec(),
            "text/plain",
        );
        let reply = call(
            &session.connection,
            session.collection.as_str(),
            "org.freedesktop.Secret.Collection",
            "CreateItem",
            &(&properties, &secret, replace),
        )?;
        let (_item, prompt): (OwnedObjectPath, OwnedObjectPath) = reply
            .body()
            .deserialize()
            .map_err(|error| plaintext_seal_refusal(&error.to_string()))?;
        if prompt_blocks(&prompt) {
            return Err(plaintext_seal_refusal(
                "Secret Service asked for a prompt to store the host seal key, and Plexi will not write a plaintext key instead",
            ));
        }
        log::info!("permission_seal: stored host item {account} in Secret Service");
        Ok(())
    }

    fn delete_unlocked(account: &str) -> Result<(), String> {
        let session = open_session()?;
        for item in search(&session, account)? {
            let reply = call(
                &session.connection,
                item.as_str(),
                "org.freedesktop.Secret.Item",
                "Delete",
                &(),
            )?;
            let prompt: OwnedObjectPath = reply
                .body()
                .deserialize()
                .map_err(|error| plaintext_seal_refusal(&error.to_string()))?;
            if prompt_blocks(&prompt) {
                return Err(plaintext_seal_refusal(
                    "Secret Service asked for a prompt to delete the host seal key",
                ));
            }
        }
        Ok(())
    }

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(super) fn get(account: &str) -> Result<Option<Zeroizing<String>>, String> {
        let _guard = lock();
        get_unlocked(account)
    }

    pub(super) fn add_new(account: &str, value: &str) -> Result<(), String> {
        let _guard = lock();
        add_new_unlocked(account, value)
    }

    pub(super) fn set(account: &str, value: &str) -> Result<(), String> {
        let _guard = lock();
        set_unlocked(account, value)
    }

    pub(super) fn delete(account: &str) -> Result<(), String> {
        let _guard = lock();
        delete_unlocked(account)
    }
}

#[cfg(all(target_os = "macos", not(test)))]
mod mac {
    use super::HOST_SERVICE;
    use security_framework::os::macos::keychain::SecKeychain;
    use zeroize::Zeroizing;

    fn keychain() -> Result<SecKeychain, String> {
        match std::env::var("PLEXI_KEYCHAIN_PATH") {
            Ok(path) if !path.trim().is_empty() => {
                let path = path.trim();
                log::info!("permission_seal: macOS host keychain {path}");
                SecKeychain::open(path).map_err(|error| {
                    format!(
                        "PLEXI_KEYCHAIN_PATH {path} could not be opened ({error}). \
                         The host seal key is not written to the login keychain."
                    )
                })
            }
            _ => SecKeychain::default().map_err(|error| error.to_string()),
        }
    }

    pub(super) fn get(account: &str) -> Result<Option<Zeroizing<String>>, String> {
        let chain = keychain()?;
        match chain.find_generic_password(HOST_SERVICE, account) {
            Ok((password, _item)) => {
                let bytes = password.as_ref();
                let text = String::from_utf8(bytes.to_vec())
                    .map_err(|_| "host seal item is not utf-8".to_string())?;
                Ok(Some(Zeroizing::new(text)))
            }
            Err(error) if error.code() == -25300 => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub(super) fn add_new(account: &str, value: &str) -> Result<(), String> {
        if get(account)?.is_some() {
            return Err(format!("host key already exists: {account}"));
        }
        let chain = keychain()?;
        // The creating binary is the trusted application. `security` is not,
        // so a direct keychain read does not return the secret.
        chain
            .add_generic_password(HOST_SERVICE, account, value.as_bytes())
            .map_err(|error| error.to_string())?;
        log::info!("permission_seal: stored host item {account} in the macOS keychain");
        Ok(())
    }

    pub(super) fn set(account: &str, value: &str) -> Result<(), String> {
        let chain = keychain()?;
        chain
            .set_generic_password(HOST_SERVICE, account, value.as_bytes())
            .map_err(|error| error.to_string())?;
        log::info!("permission_seal: updated host item {account} in the macOS keychain");
        Ok(())
    }

    pub(super) fn delete(account: &str) -> Result<(), String> {
        let chain = keychain()?;
        match chain.find_generic_password(HOST_SERVICE, account) {
            Ok((_password, item)) => item.delete().map_err(|error| error.to_string()),
            Err(error) if error.code() == -25300 => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(all(windows, not(test)))]
mod windows {
    use super::HOST_SERVICE;
    use zeroize::Zeroizing;

    fn target(account: &str) -> String {
        format!("{HOST_SERVICE}/{account}")
    }

    fn to_wide_nul(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub(super) fn get(account: &str) -> Result<Option<Zeroizing<String>>, String> {
        use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
        use windows_sys::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC};

        let wide = to_wide_nul(&target(account));
        let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
        let ok = unsafe { CredReadW(wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
                return Ok(None);
            }
            return Err(format!("CredReadW failed: {error}"));
        }
        let text = unsafe {
            let size = (*credential).CredentialBlobSize as usize;
            let bytes = std::slice::from_raw_parts((*credential).CredentialBlob, size);
            let text = String::from_utf8(bytes.to_vec());
            CredFree(credential as *const core::ffi::c_void);
            text
        };
        text
            .map(|value| Some(Zeroizing::new(value)))
            .map_err(|_| "host seal item is not utf-8".to_string())
    }

    pub(super) fn add_new(account: &str, value: &str) -> Result<(), String> {
        if get(account)?.is_some() {
            return Err(format!("host key already exists: {account}"));
        }
        write(account, value)
    }

    pub(super) fn set(account: &str, value: &str) -> Result<(), String> {
        write(account, value)
    }

    fn write(account: &str, value: &str) -> Result<(), String> {
        use windows_sys::Win32::Security::Credentials::{
            CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
        };

        let name = target(account);
        let mut wide = to_wide_nul(&name);
        let blob = Zeroizing::new(value.as_bytes().to_vec());
        let mut credential: CREDENTIALW = unsafe { std::mem::zeroed() };
        credential.Type = CRED_TYPE_GENERIC;
        credential.TargetName = wide.as_mut_ptr();
        credential.CredentialBlobSize = blob.len() as u32;
        credential.CredentialBlob = blob.as_ptr() as *mut u8;
        credential.Persist = CRED_PERSIST_LOCAL_MACHINE;
        let ok = unsafe { CredWriteW(&credential, 0) };
        let error = std::io::Error::last_os_error();
        drop(blob);
        drop(wide);
        if ok == 0 {
            return Err(format!("CredWriteW('{name}') failed: {error}"));
        }
        log::info!("permission_seal: stored host item {account} in Credential Manager");
        Ok(())
    }

    pub(super) fn delete(account: &str) -> Result<(), String> {
        use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
        use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};

        let name = target(account);
        let wide = to_wide_nul(&name);
        let ok = unsafe { CredDeleteW(wide.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
                return Ok(());
            }
            return Err(format!("CredDeleteW('{name}') failed: {error}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_names_secret_service_and_the_plaintext_file() {
        let message = plaintext_seal_refusal("no session bus");
        assert!(message.contains("Secret Service"), "{message}");
        assert!(message.contains("secrets.json"), "{message}");
        assert!(message.contains("plaintext"), "{message}");
        assert!(message.contains("no session bus"), "{message}");
    }

    #[test]
    fn host_key_is_not_written_to_the_user_secret_store() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_profile_dir(dir.path().to_path_buf());
        system_store()
            .set("plexi:host:permission-mac", "leaked-key")
            .unwrap();
        scrub_user_secret_host_namespace();
        assert!(system_store().get("plexi:host:permission-mac").is_none());
        add_new("probe-item", "host-only-key").unwrap();
        assert_eq!(
            get("probe-item").unwrap().unwrap().as_str(),
            "host-only-key"
        );
        assert!(system_store().get("plexi:host:permission-mac").is_none());
        assert!(system_store().get("permission-mac").is_none());
        assert!(!dir.path().join("secrets.json").exists());
    }
}
