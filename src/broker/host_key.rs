//! Host seal key custody until the permission seal (#2718) replaces this module.
//!
//! The permission MAC and the Needs you journal tip are not user secrets.
//! They are never written to `secrets.json` or `seal.key`.
//!
//! - Tests use a process-local mock. They do not open a keychain or a session bus.
//! - Linux stores the key only in Secret Service (`org.freedesktop.secrets`).
//!   If that service is missing, sealing fails and says why. There is no
//!   plaintext fallback.
//! - macOS and Windows keep the same function names and refuse to seal until
//!   the permission-seal keychain and Credential Manager stores land. That
//!   refusal is what lets this tree compile before those stores exist.

#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::sync::Mutex;
use zeroize::Zeroizing;

use crate::workspace::secrets::system_store;

/// Item name of the MAC key inside the host store. Not a user-secret account.
pub(crate) const MAC_ITEM: &str = "permission-mac";

/// Why Linux will not write a plaintext seal key.
#[cfg(all(target_os = "linux", not(test)))]
pub(crate) fn plaintext_seal_refusal(detail: &str) -> String {
    format!(
        "Linux refuses to seal the permission MAC with a plaintext key in secrets.json. \
         The seal key is stored only in Secret Service (org.freedesktop.secrets). \
         Secret Service is not available: {detail}"
    )
}

/// Remove `plexi:host:*` from the user secret store so an older build's
/// plaintext copy cannot be read back as the seal key.
pub(crate) fn scrub_user_secret_host_namespace() {
    let store = system_store();
    let accounts = store.list_with_prefix("plexi:host:");
    if accounts.is_empty() {
        return;
    }
    for account in accounts {
        match store.delete(&account) {
            Ok(()) => log::info!("permission_seal: removed {account} from the user secret store"),
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
    #[cfg(all(not(test), not(target_os = "linux")))]
    {
        static UNLINKED: UnlinkedBackend = UnlinkedBackend;
        &UNLINKED
    }
}

trait HostKeyBackend: Sync + Send {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, String>;
    fn add_new(&self, account: &str, value: &str) -> Result<(), String>;
    fn set(&self, account: &str, value: &str) -> Result<(), String>;
}

#[cfg(test)]
#[derive(Default)]
struct MemoryBackend {
    items: Mutex<BTreeMap<String, String>>,
}

#[cfg(test)]
impl HostKeyBackend for MemoryBackend {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, String> {
        Ok(self
            .items
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(account)
            .cloned()
            .map(Zeroizing::new))
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), String> {
        let mut items = self.items.lock().unwrap_or_else(|error| error.into_inner());
        if items.contains_key(account) {
            return Err(format!("host key already exists: {account}"));
        }
        items.insert(account.to_string(), value.to_string());
        Ok(())
    }

    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        self.items
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(account.to_string(), value.to_string());
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
}

/// Compiles on macOS and Windows before the permission-seal stores exist.
/// Sealing fails instead of writing `seal.key` or `secrets.json`.
#[cfg(all(not(test), not(target_os = "linux")))]
struct UnlinkedBackend;

#[cfg(all(not(test), not(target_os = "linux")))]
impl HostKeyBackend for UnlinkedBackend {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, String> {
        let _ = account;
        Err(unlinked())
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), String> {
        let _ = (account, value);
        Err(unlinked())
    }

    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        let _ = (account, value);
        Err(unlinked())
    }
}

#[cfg(all(not(test), not(target_os = "linux")))]
fn unlinked() -> String {
    "host seal store is not linked on this platform until the permission seal lands; refusing to write a plaintext key".to_string()
}

#[cfg(all(target_os = "linux", not(test)))]
mod linux {
    use super::plaintext_seal_refusal;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use zbus::blocking::Connection;
    use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};
    use zeroize::Zeroizing;

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

    fn create_item(
        session: &Session,
        account: &str,
        value: &str,
        replace: bool,
    ) -> Result<(), String> {
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
}
