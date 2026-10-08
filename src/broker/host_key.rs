//! Host seal key custody.
//!
//! The permission MAC and the audit-chain tip are not user secrets. They are
//! never written to `secrets.json` and they are not readable with
//! `plexi secret get`. The account namespace `plexi:host:*` is reserved.
//!
//! `get` / `set` / `add_new` / `delete` store the account string unchanged.
//! The Needs you journal tip is `plexi:host:needs-you-journal-tip:` plus the
//! sha256 of the profile host directory. The permission MAC those callers
//! share is `seal::mac_key_bytes` / `seal::existing_mac_key`.
//!
//! - Tests use a process-local mock. They do not open a keychain or a session bus.
//! - Linux stores the key only in Secret Service. If that service is missing,
//!   sealing fails and says why. There is no plaintext fallback. The session
//!   handshake and each Secret Service call stop at a short startup deadline.
//!   `PlexiApp::new` reads this key before the notify socket exists, so a bus
//!   that accepts and never handshakes must not hold `host start` until its
//!   own deadline.
//! - macOS stores it in the keychain under service `plexi-host-seal`.
//!   `PLEXI_KEYCHAIN_PATH` selects a throwaway keychain file and never falls
//!   back to the login keychain. The `security` tool is not the creating
//!   binary, so a direct keychain read does not yield the key. Startup
//!   disables keychain prompts so `host start` cannot block on Allow. A
//!   Needs you approval is the only path that turns prompts on, for that
//!   one read, and then turns them off again. New and interactively read
//!   items trust this app's designated requirement so a same-signer rebuild
//!   keeps access. An any-application ACL is never installed.
//! - Windows stores it in Credential Manager under `plexi-host-seal/`, which
//!   `plexi secret get` does not read.

#[cfg(test)]
use std::collections::BTreeMap;
use std::sync::Mutex;
use zeroize::Zeroizing;

use crate::workspace::secrets::system_store;

/// Item name of the MAC key inside the host store. Not a user-secret account.
pub(crate) const MAC_ITEM: &str = "permission-mac";

/// Prefix on a host-store read that did not return the secret.
///
/// A mismatch is a different failure: the key was read and the MAC did not
/// match. Callers must not quarantine on this prefix.
pub(crate) const KEY_UNREADABLE_PREFIX: &str = "key unreadable:";

#[cfg(any(test, target_os = "macos"))]
pub(crate) fn key_unreadable(detail: &str) -> String {
    format!("{KEY_UNREADABLE_PREFIX} {detail}")
}

pub(crate) fn is_key_unreadable(error: &str) -> bool {
    error.starts_with(KEY_UNREADABLE_PREFIX)
}

#[cfg(test)]
pub(crate) fn ensure_key_unreadable(error: String) -> String {
    if is_key_unreadable(&error) {
        error
    } else {
        key_unreadable(&error)
    }
}

/// `errSecAuthFailed` (-25293), `errSecInteractionNotAllowed` (-25308), and
/// `userCanceledErr` (-128). Item-not-found (-25300) is not an access failure.
#[cfg(any(test, target_os = "macos"))]
pub(crate) fn keychain_status_is_unreadable(code: i32) -> bool {
    matches!(code, -25293 | -25308 | -128)
}

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
            Ok(()) => log::info!("permission_seal: removed {account} from the user secret store"),
            Err(error) => log::error!(
                "permission_seal: could not remove {account} from the user secret store: {error}"
            ),
        }
    }
}

pub(crate) fn get(account: &str) -> Result<Option<Zeroizing<String>>, String> {
    if account == MAC_ITEM {
        if let Some(cached) = session_mac() {
            return Ok(Some(cached));
        }
    }
    match with_backend(|| backend().get(account)) {
        Ok(value) => {
            if account == MAC_ITEM && interaction_allowed() {
                if let Some(value) = &value {
                    remember_session_mac(value);
                }
            }
            Ok(value)
        }
        Err(error) => Err(error),
    }
}

/// Read `body` while keychain prompts are allowed.
///
/// Startup stays silent. The only caller is a person approving the keychain
/// Needs you item, which is what lets macOS show Allow / Always Allow.
pub(crate) fn with_keychain_interaction<T>(body: impl FnOnce() -> T) -> T {
    KEYCHAIN_INTERACTION.with(|flag| flag.set(true));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    KEYCHAIN_INTERACTION.with(|flag| flag.set(false));
    match result {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn interaction_allowed() -> bool {
    KEYCHAIN_INTERACTION.with(|flag| flag.get())
}

thread_local! {
    static KEYCHAIN_INTERACTION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn session_mac_slot() -> &'static Mutex<Option<Zeroizing<String>>> {
    static SLOT: std::sync::OnceLock<Mutex<Option<Zeroizing<String>>>> = std::sync::OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn session_mac() -> Option<Zeroizing<String>> {
    session_mac_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

fn remember_session_mac(value: &Zeroizing<String>) {
    *session_mac_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(value.clone());
}

fn clear_session_mac() {
    *session_mac_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = None;
}

pub(crate) fn add_new(account: &str, value: &str) -> Result<(), String> {
    with_backend(|| backend().add_new(account, value))
}

pub(crate) fn set(account: &str, value: &str) -> Result<(), String> {
    with_backend(|| backend().set(account, value))
}

pub(crate) fn delete(account: &str) -> Result<(), String> {
    if account == MAC_ITEM {
        clear_session_mac();
    }
    with_backend(|| backend().delete(account))
}

fn with_backend<T>(body: impl FnOnce() -> T) -> T {
    #[cfg(test)]
    {
        let _guard = TestBackendGuard::acquire_if_needed();
        body()
    }
    #[cfg(not(test))]
    {
        body()
    }
}

/// Holds the process-local host-key map so a test can remove `permission-mac`
/// without a parallel test observing that gap.
#[cfg(test)]
pub(crate) struct ExclusiveHostKey {
    _guard: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl ExclusiveHostKey {
    pub(crate) fn acquire() -> Self {
        let guard = host_key_test_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        HOST_KEY_HELD.with(|held| held.set(true));
        Self { _guard: guard }
    }
}

#[cfg(test)]
impl Drop for ExclusiveHostKey {
    fn drop(&mut self) {
        HOST_KEY_HELD.with(|held| held.set(false));
    }
}

/// Run `body` with the permission MAC absent, then put the previous key back.
///
/// The in-memory seal store is process-global. Legacy-adopt tests need a
/// profile whose host has never created that key, including when an earlier
/// test already did.
#[cfg(test)]
pub(crate) fn without_mac_key<T>(body: impl FnOnce() -> T) -> T {
    let _exclusive = ExclusiveHostKey::acquire();
    let previous = get(MAC_ITEM).expect("read permission mac");
    if previous.is_some() {
        delete(MAC_ITEM).expect("clear permission mac");
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    match &previous {
        Some(value) => set(MAC_ITEM, value.as_str()).expect("restore permission mac"),
        None => delete(MAC_ITEM).expect("clear permission mac created by the test"),
    }
    match result {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

#[cfg(test)]
thread_local! {
    static HOST_KEY_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
struct MacReadFault {
    denied: bool,
    interactive_reads: u32,
}

#[cfg(test)]
fn mac_read_fault() -> &'static Mutex<MacReadFault> {
    static FAULT: Mutex<MacReadFault> = Mutex::new(MacReadFault {
        denied: false,
        interactive_reads: 0,
    });
    &FAULT
}

#[cfg(test)]
fn mac_read_denied() -> bool {
    mac_read_fault()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .denied
}

#[cfg(test)]
fn note_interactive_mac_read() {
    let mut fault = mac_read_fault()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    fault.interactive_reads += 1;
    fault.denied = false;
}

/// The permission MAC exists, but silent reads fail the way a rebuilt binary
/// fails with `errSecAuthFailed` while prompts are off.
///
/// An interactive read counts once and then succeeds, which is the Always Allow
/// / designated-requirement outcome. The guard holds the process-wide host-key
/// lock so other tests do not observe the denial.
#[cfg(test)]
pub(crate) struct UnreadableMacKey {
    _exclusive: ExclusiveHostKey,
}

#[cfg(test)]
impl UnreadableMacKey {
    pub(crate) fn acquire() -> Self {
        let exclusive = ExclusiveHostKey::acquire();
        clear_session_mac();
        let mut fault = mac_read_fault()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        fault.denied = true;
        fault.interactive_reads = 0;
        Self {
            _exclusive: exclusive,
        }
    }

    pub(crate) fn interactive_reads(&self) -> u32 {
        mac_read_fault()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .interactive_reads
    }
}

#[cfg(test)]
impl Drop for UnreadableMacKey {
    fn drop(&mut self) {
        let mut fault = mac_read_fault()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        fault.denied = false;
        fault.interactive_reads = 0;
        clear_session_mac();
    }
}

#[cfg(test)]
fn host_key_test_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}

#[cfg(test)]
struct TestBackendGuard {
    release: bool,
    _guard: Option<std::sync::MutexGuard<'static, ()>>,
}

#[cfg(test)]
impl TestBackendGuard {
    fn acquire_if_needed() -> Self {
        if HOST_KEY_HELD.with(|held| held.get()) {
            return Self {
                release: false,
                _guard: None,
            };
        }
        let guard = host_key_test_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        HOST_KEY_HELD.with(|held| held.set(true));
        Self {
            release: true,
            _guard: Some(guard),
        }
    }
}

#[cfg(test)]
impl Drop for TestBackendGuard {
    fn drop(&mut self) {
        if self.release {
            HOST_KEY_HELD.with(|held| held.set(false));
        }
    }
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
    #[cfg(all(not(test), not(any(target_os = "linux", target_os = "macos", windows))))]
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
        if account == MAC_ITEM && mac_read_denied() {
            let exists = self
                .values
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .contains_key(account);
            if exists {
                if !interaction_allowed() {
                    return Err(key_unreadable("errSecAuthFailed"));
                }
                note_interactive_mac_read();
            }
        }
        Ok(self
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(account)
            .cloned()
            .map(Zeroizing::new))
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), String> {
        let mut map = self
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner());
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

#[cfg(all(not(test), not(any(target_os = "linux", target_os = "macos", windows))))]
struct UnsupportedBackend;

#[cfg(all(not(test), not(any(target_os = "linux", target_os = "macos", windows))))]
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use zeroize::Zeroizing;
    use zbus::blocking::Connection;
    use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};
    use zeroize::Zeroizing;

    const SCHEMA: &str = "com.plexi.HostSeal";
    /// Bound for the session handshake and for Secret Service method calls.
    ///
    /// Longer than a healthy local bus, shorter than `plexi host start`'s
    /// readiness wait. A socket that accepts and never finishes SASL used to
    /// sit in `Connection::session` until that wait expired, because this read
    /// runs inside `PlexiApp::new` before the notify socket is bound.
    const STARTUP_DEADLINE: Duration = Duration::from_secs(2);

    fn bus_gave_up() -> &'static AtomicBool {
        static GAVE_UP: AtomicBool = AtomicBool::new(false);
        &GAVE_UP
    }

    fn session() -> Result<Connection, String> {
        if bus_gave_up().load(Ordering::Relaxed) {
            return Err(plaintext_seal_refusal(
                "session bus did not answer; host startup will not wait again",
            ));
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let spawned = std::thread::Builder::new()
            .name("seal-bus".to_string())
            .spawn(move || {
                let built = zbus::blocking::connection::Builder::session()
                    .and_then(|builder| builder.method_timeout(STARTUP_DEADLINE).build());
                let _ = tx.send(built.map_err(|error| error.to_string()));
            });
        if let Err(error) = spawned {
            return Err(plaintext_seal_refusal(&format!(
                "could not start the session-bus handshake: {error}"
            )));
        }
        match rx.recv_timeout(STARTUP_DEADLINE) {
            Ok(Ok(connection)) => Ok(connection),
            Ok(Err(error)) => Err(plaintext_seal_refusal(&error)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                bus_gave_up().store(true, Ordering::Relaxed);
                log::info!(
                    "permission_seal: session bus did not finish the handshake within {}s; continuing so the notify socket can bind",
                    STARTUP_DEADLINE.as_secs()
                );
                Err(plaintext_seal_refusal(
                    "session bus accepted a connection and did not complete the handshake before the startup deadline",
                ))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(plaintext_seal_refusal(
                "session bus handshake ended without a connection",
            )),
        }
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
    use core_foundation::array::CFArray;
    use core_foundation::base::{CFType, CFTypeRef, TCFType};
    use core_foundation::string::CFString;
    use security_framework::base::Error;
    use security_framework::os::macos::access::SecAccess;
    use security_framework::os::macos::keychain::SecKeychain;
    use security_framework::os::macos::keychain_item::SecKeychainItem;
    use security_framework_sys::base::{errSecSuccess, SecAccessRef};
    use security_framework_sys::keychain::SecKeychainSetUserInteractionAllowed;
    use zeroize::Zeroizing;

    // errSecItemNotFound. A missing host item is an empty store, not a failure.
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    // Not in security-framework-sys. A null path is the calling app's designated
    // requirement. A null trusted list would trust every app, so this code never
    // passes one.
    #[link(name = "Security", kind = "framework")]
    extern "C" {
        fn SecTrustedApplicationCreateFromPath(
            path: *const std::ffi::c_char,
            app: *mut *mut std::ffi::c_void,
        ) -> i32;
        fn SecAccessCreate(
            descriptor: *const std::ffi::c_void,
            trusted_list: *const std::ffi::c_void,
            access: *mut SecAccessRef,
        ) -> i32;
        fn SecKeychainItemSetAccess(
            item: security_framework_sys::base::SecKeychainItemRef,
            access: SecAccessRef,
        ) -> i32;
    }

    /// Disable keychain dialogs once and leak the guard.
    ///
    /// `MacKeychain` does the same. Dropping that guard turns prompts back on,
    /// which would let a later seal read block `host start` again. The audit
    /// tip is read from `PlexiApp::new`, before the event loop can answer
    /// `ListPanes`. A Needs you approval turns prompts on for one read via
    /// [`PromptScope`], then turns them off again.
    fn keychain_calls_cannot_prompt() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| match SecKeychain::disable_user_interaction() {
            Ok(guard) => {
                std::mem::forget(guard);
                log::info!(
                    "permission_seal: disabled keychain prompts so host startup cannot wait on a credential dialog"
                );
            }
            Err(error) => {
                log::warn!("permission_seal: could not disable keychain prompts: {error}");
            }
        });
    }

    /// Turns prompts on only while a Needs you approval is reading the key.
    struct PromptScope {
        restore_off: bool,
    }

    impl PromptScope {
        fn enter() -> Self {
            keychain_calls_cannot_prompt();
            if super::interaction_allowed() {
                let code = unsafe { SecKeychainSetUserInteractionAllowed(1) };
                if code != errSecSuccess {
                    log::error!("permission_seal: could not allow a keychain prompt: {code}");
                } else {
                    log::info!("permission_seal: keychain interaction allowed for this read");
                }
                Self { restore_off: true }
            } else {
                let code = unsafe { SecKeychainSetUserInteractionAllowed(0) };
                if code != errSecSuccess {
                    log::error!("permission_seal: could not keep keychain prompts off: {code}");
                }
                Self { restore_off: false }
            }
        }
    }

    impl Drop for PromptScope {
        fn drop(&mut self) {
            if !self.restore_off {
                return;
            }
            let code = unsafe { SecKeychainSetUserInteractionAllowed(0) };
            if code != errSecSuccess {
                log::error!("permission_seal: could not disable keychain prompts again: {code}");
            } else {
                log::info!(
                    "permission_seal: keychain prompts disabled again after interactive read"
                );
            }
        }
    }

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
            _ => SecKeychain::default().map_err(|error| map_keychain_error("login", error)),
        }
    }

    fn map_keychain_error(account: &str, error: Error) -> String {
        if super::keychain_status_is_unreadable(error.code()) {
            log::info!(
                "permission_seal: macOS keychain refused {account} ({})",
                error.code()
            );
            super::key_unreadable(&format!("{} ({})", error.code(), error))
        } else {
            error.to_string()
        }
    }

    pub(super) fn get(account: &str) -> Result<Option<Zeroizing<String>>, String> {
        let _prompts = PromptScope::enter();
        let chain = keychain()?;
        match chain.find_generic_password(HOST_SERVICE, account) {
            Ok((password, item)) => {
                if super::interaction_allowed() {
                    trust_designated_requirement(account, &item);
                }
                let bytes = password.as_ref();
                let text = String::from_utf8(bytes.to_vec())
                    .map_err(|_| "host seal item is not utf-8".to_string())?;
                Ok(Some(Zeroizing::new(text)))
            }
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(error) => Err(map_keychain_error(account, error)),
        }
    }

    pub(super) fn add_new(account: &str, value: &str) -> Result<(), String> {
        let _prompts = PromptScope::enter();
        if get(account)?.is_some() {
            return Err(format!("host key already exists: {account}"));
        }
        let chain = keychain()?;
        // The creating binary is the trusted application. `security` is not,
        // so a direct keychain read does not return the secret. The ACL is
        // then widened to this app's designated requirement, never to any app.
        chain
            .add_generic_password(HOST_SERVICE, account, value.as_bytes())
            .map_err(|error| map_keychain_error(account, error))?;
        trust_item_in(account, &chain);
        log::info!("permission_seal: stored host item {account} in the macOS keychain");
        Ok(())
    }

    pub(super) fn set(account: &str, value: &str) -> Result<(), String> {
        let _prompts = PromptScope::enter();
        let chain = keychain()?;
        chain
            .set_generic_password(HOST_SERVICE, account, value.as_bytes())
            .map_err(|error| map_keychain_error(account, error))?;
        trust_item_in(account, &chain);
        log::info!("permission_seal: updated host item {account} in the macOS keychain");
        Ok(())
    }

    pub(super) fn delete(account: &str) -> Result<(), String> {
        let _prompts = PromptScope::enter();
        let chain = keychain()?;
        match chain.find_generic_password(HOST_SERVICE, account) {
            // security-framework 3.7 `SecKeychainItem::delete` returns `()`
            // and discards the OSStatus. Lookup failures stay mapped below.
            Ok((_password, item)) => {
                item.delete();
                Ok(())
            }
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(error) => Err(map_keychain_error(account, error)),
        }
    }

    fn trust_item_in(account: &str, chain: &SecKeychain) {
        match chain.find_generic_password(HOST_SERVICE, account) {
            Ok((_password, item)) => trust_designated_requirement(account, &item),
            Err(error) => log::error!(
                "permission_seal: could not load {account} to update its keychain ACL: {error}"
            ),
        }
    }

    /// Trust this app's designated requirement. Failure leaves the cdhash ACL
    /// the keychain already wrote. It does not fail the key write or the read.
    fn trust_designated_requirement(account: &str, item: &SecKeychainItem) {
        let status = unsafe { set_designated_requirement_acl(item) };
        if status == errSecSuccess {
            if account == super::MAC_ITEM {
                log::info!(
                    "permission_seal: keychain ACL for {account} trusts this app's designated requirement"
                );
            }
        } else {
            log::error!(
                "permission_seal: could not set keychain ACL for {account} to the designated requirement: {status}"
            );
        }
    }

    unsafe fn set_designated_requirement_acl(item: &SecKeychainItem) -> i32 {
        let mut app: *mut std::ffi::c_void = std::ptr::null_mut();
        let status = SecTrustedApplicationCreateFromPath(std::ptr::null(), &mut app);
        if status != errSecSuccess || app.is_null() {
            return if status == errSecSuccess { -1 } else { status };
        }
        let app = CFType::wrap_under_create_rule(app as CFTypeRef);
        // One trusted application. A null list would allow any process.
        let trusted = CFArray::<CFType>::from_CFTypes(&[app]);
        let label = CFString::new("Plexi permission seal");
        let mut access: SecAccessRef = std::ptr::null_mut();
        let status = SecAccessCreate(
            label.as_concrete_TypeRef().cast(),
            trusted.as_concrete_TypeRef().cast(),
            &mut access,
        );
        if status != errSecSuccess || access.is_null() {
            return if status == errSecSuccess { -1 } else { status };
        }
        let access = SecAccess::wrap_under_create_rule(access);
        SecKeychainItemSetAccess(item.as_concrete_TypeRef(), access.as_concrete_TypeRef())
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
        use windows_sys::Win32::Security::Credentials::{
            CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
        };

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
        text.map(|value| Some(Zeroizing::new(value)))
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
    fn keychain_auth_failed_is_unreadable_not_a_seal_mismatch() {
        assert!(keychain_status_is_unreadable(-25293));
        assert!(keychain_status_is_unreadable(-25308));
        assert!(keychain_status_is_unreadable(-128));
        assert!(
            !keychain_status_is_unreadable(-25300),
            "a missing item is not an access failure"
        );
        assert!(!keychain_status_is_unreadable(0));
        let error = ensure_key_unreadable(
            "The user name or passphrase you entered is not correct.".to_string(),
        );
        assert!(is_key_unreadable(&error), "{error}");
        assert!(is_key_unreadable(&key_unreadable("errSecAuthFailed")));
        assert!(!is_key_unreadable("bad mac"));
    }

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

    #[test]
    fn needs_you_journal_tip_account_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_profile_dir(dir.path().to_path_buf());
        let account = "plexi:host:needs-you-journal-tip:\
            0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(get(account).unwrap().is_none());
        set(account, "abcd").unwrap();
        assert_eq!(get(account).unwrap().unwrap().as_str(), "abcd");
        set(account, "ef01").unwrap();
        assert_eq!(get(account).unwrap().unwrap().as_str(), "ef01");
        delete(account).unwrap();
        assert!(get(account).unwrap().is_none());
        assert!(system_store().get(account).is_none());
    }
}
