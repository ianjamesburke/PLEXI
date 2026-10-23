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
//!   read that fails with an auth or ACL status (`errSecAuthFailed` -25293
//!   and the same class) is rotated: the old item is deleted without reading
//!   its secret, and a new key is created. New items trust this app's
//!   designated requirement, attached when the item is created, so a
//!   same-signer rebuild keeps access when that requirement is stable. An
//!   ad-hoc signature changes every build, so the next launch rotates again.
//!   Startup never edits an existing item's ACL in place. Every silent keychain
//!   call runs off the UI thread and stops after three seconds. Past that
//!   deadline the host continues with grants unloaded and one resolvable
//!   keychain Needs you item. An any-application ACL is never installed. If
//!   the old item cannot be deleted while prompts are off, a Needs you
//!   approval turns prompts on and either retries the read or finishes the
//!   rotation.
//! - Windows stores it in Credential Manager under `plexi-host-seal/`, which
//!   `plexi secret get` does not read.

use std::collections::BTreeMap;
use std::sync::Mutex;
#[cfg(any(test, target_os = "macos"))]
use std::time::Duration;
use zeroize::Zeroizing;

/// Silent keychain work during startup stops here.
///
/// `plexi host start` gives up at 90s. A keychain call that outlives this
/// deadline must not be what consumes that budget: the host continues with
/// grants unloaded and a keychain Needs you item.
#[cfg(any(test, target_os = "macos"))]
pub(crate) const STARTUP_KEYCHAIN_DEADLINE: Duration = Duration::from_secs(3);

#[cfg(any(test, target_os = "macos"))]
fn keychain_gave_up() -> &'static std::sync::atomic::AtomicBool {
    static GAVE_UP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    &GAVE_UP
}

#[cfg(test)]
fn abandoned_keychain_calls() -> &'static Mutex<Vec<std::thread::JoinHandle<()>>> {
    static ABANDONED: Mutex<Vec<std::thread::JoinHandle<()>>> = Mutex::new(Vec::new());
    &ABANDONED
}

#[cfg(test)]
fn join_abandoned_keychain_calls() {
    let handles = std::mem::take(
        &mut *abandoned_keychain_calls()
            .lock()
            .unwrap_or_else(|error| error.into_inner()),
    );
    for handle in handles {
        let _ = handle.join();
    }
}

#[cfg(test)]
use crate::workspace::secrets::system_store;

/// Legacy unscoped item name of the MAC key inside the host store.
/// New seals use [`MAC_ACCOUNT_PREFIX`] plus the sha256 of the canonical profile dir.
pub(crate) const MAC_ITEM: &str = "permission-mac";

/// Per-profile MAC key account prefix (`plexi:host:permission-mac:<sha256(canonical_profile_dir)>`).
pub(crate) const MAC_ACCOUNT_PREFIX: &str = "plexi:host:permission-mac:";

pub(crate) fn is_mac_account(account: &str) -> bool {
    account == MAC_ITEM || account.starts_with(MAC_ACCOUNT_PREFIX)
}

/// Human-readable label shown in macOS Keychain Access and SecurityAgent dialogs
/// so any Keychain entry or prompt explains what the key is and why Plexi uses it.
#[cfg(any(test, target_os = "macos"))]
pub(crate) fn keychain_item_label(account: &str) -> String {
    let channel = crate::config::build_channel().unwrap_or_else(|| "stable".to_string());
    if is_mac_account(account) {
        format!(
            "Plexi ({channel}) — Local Permission & Approval Signing Key (verifies saved tool approvals)"
        )
    } else {
        format!("Plexi ({channel}) — Host Integrity Key ({account})")
    }
}

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
#[cfg(all(any(target_os = "macos", windows), not(test)))]
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

pub(crate) fn get(account: &str) -> Result<Option<Zeroizing<String>>, String> {
    if is_mac_account(account) && !pr_isolation_active() {
        if let Some(cached) = session_mac(account) {
            return Ok(cached);
        }
    }
    let account = account.to_string();
    dispatch_keychain(move || get_direct(&account))
}

fn get_direct(account: &str) -> Result<Option<Zeroizing<String>>, String> {
    if is_mac_account(account) && !pr_isolation_active() {
        if let Some(cached) = session_mac(account) {
            return Ok(cached);
        }
    }
    match backend().get(account) {
        Ok(value) => {
            if is_mac_account(account) && !pr_isolation_active() {
                remember_session_mac(account, value.clone());
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

#[cfg(any(test, target_os = "macos"))]
fn interaction_allowed() -> bool {
    KEYCHAIN_INTERACTION.with(|flag| flag.get())
}

thread_local! {
    static KEYCHAIN_INTERACTION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

type SessionMacMap = BTreeMap<String, Option<Zeroizing<String>>>;

fn session_mac_slot() -> &'static Mutex<SessionMacMap> {
    static SLOT: std::sync::OnceLock<Mutex<SessionMacMap>> = std::sync::OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn session_mac(account: &str) -> Option<Option<Zeroizing<String>>> {
    session_mac_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(account)
        .cloned()
}

fn remember_session_mac(account: &str, value: Option<Zeroizing<String>>) {
    session_mac_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(account.to_string(), value);
}

fn forget_session_mac(account: &str) {
    session_mac_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(account);
}

#[cfg(test)]
pub(crate) fn clear_session_mac() {
    session_mac_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
}

pub(crate) fn add_new(account: &str, value: &str) -> Result<(), String> {
    let account = account.to_string();
    let value = value.to_string();
    dispatch_keychain(move || add_new_direct(&account, &value))
}

fn add_new_direct(account: &str, value: &str) -> Result<(), String> {
    backend().add_new(account, value)?;
    if is_mac_account(account) && !pr_isolation_active() {
        remember_session_mac(account, Some(Zeroizing::new(value.to_string())));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn set(account: &str, value: &str) -> Result<(), String> {
    let account = account.to_string();
    let value = value.to_string();
    dispatch_keychain(move || set_direct(&account, &value))
}

#[cfg(test)]
fn set_direct(account: &str, value: &str) -> Result<(), String> {
    match backend().set(account, value) {
        Ok(()) => {
            if is_mac_account(account) && !pr_isolation_active() {
                remember_session_mac(account, Some(Zeroizing::new(value.to_string())));
            }
            Ok(())
        }
        Err(error) if is_key_unreadable(&error) => {
            log::info!("permission_seal: replacing unreadable host item {account}");
            delete_direct(account)?;
            add_new_direct(account, value)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn delete(account: &str) -> Result<(), String> {
    let account = account.to_string();
    dispatch_keychain(move || delete_direct(&account))
}

fn delete_direct(account: &str) -> Result<(), String> {
    if is_mac_account(account) {
        forget_session_mac(account);
    }
    backend().delete(account)
}

fn dispatch_keychain<T>(
    op: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String>
where
    T: Send + 'static,
{
    // A test caller takes the host-key lock before the bounded call reads the
    // mock's block and gave-up state. Reading them unlocked lets a caller see
    // another test's armed block, abandon a call that then waits on the lock,
    // and deadlock that test's `BlockingKeychain` teardown, which joins
    // abandoned calls while it still holds the lock.
    #[cfg(test)]
    let _guard = TestBackendGuard::acquire_if_needed();
    #[cfg(any(test, target_os = "macos"))]
    {
        bounded_keychain_call(op)
    }
    #[cfg(not(any(test, target_os = "macos")))]
    {
        op()
    }
}

/// Run one silent keychain call off the caller and stop waiting at
/// [`STARTUP_KEYCHAIN_DEADLINE`].
///
/// The UI thread is the caller during `PlexiApp::new`. A keychain dialog
/// must not freeze that thread, and `host start` must be able to bind its
/// command socket. After one deadline, later silent calls fail immediately.
/// A Needs you approval (`interaction_allowed`) still runs inline so the
/// person can answer the dialog.
#[cfg(any(test, target_os = "macos"))]
fn bounded_keychain_call<T>(
    op: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String>
where
    T: Send + 'static,
{
    if interaction_allowed() {
        return op();
    }
    if keychain_gave_up().load(std::sync::atomic::Ordering::Relaxed) {
        return Err(key_unreadable(
            "keychain did not answer before the startup deadline; host startup will not wait again",
        ));
    }
    if !keychain_call_is_bounded() {
        return op();
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new()
        .name("seal-keychain".to_string())
        .spawn(move || {
            // The caller holds the host-key lock on this call's behalf.
            #[cfg(test)]
            HOST_KEY_HELD.with(|held| held.set(true));
            let _ = tx.send(op());
        });
    let handle = match spawned {
        Ok(handle) => handle,
        Err(error) => {
            return Err(key_unreadable(&format!(
                "could not start the keychain call: {error}"
            )));
        }
    };
    match rx.recv_timeout(STARTUP_KEYCHAIN_DEADLINE) {
        Ok(result) => {
            let _ = handle.join();
            result
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            if let Ok(result) = rx.try_recv() {
                let _ = handle.join();
                return result;
            }
            keychain_gave_up().store(true, std::sync::atomic::Ordering::Relaxed);
            log::info!(
                "permission_seal: keychain call exceeded the {}s startup deadline; grants stay unloaded so the host can become ready",
                STARTUP_KEYCHAIN_DEADLINE.as_secs()
            );
            #[cfg(test)]
            remember_abandoned(handle);
            #[cfg(not(test))]
            drop(handle);
            Err(key_unreadable("keychain call exceeded the startup deadline"))
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => match handle.join() {
            Ok(()) => Err(key_unreadable("keychain call ended without a result")),
            Err(panic) => std::panic::resume_unwind(panic),
        },
    }
}

#[cfg(test)]
fn keychain_call_is_bounded() -> bool {
    lock_fault().block != KeychainBlock::None
}

#[cfg(all(target_os = "macos", not(test)))]
fn keychain_call_is_bounded() -> bool {
    true
}

#[cfg(test)]
fn remember_abandoned(handle: std::thread::JoinHandle<()>) {
    abandoned_keychain_calls()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(handle);
}

/// Holds the process-local host-key map so a test can remove `permission-mac`
/// without a parallel test observing that gap.
///
/// The outer guard snapshots the permission MAC and the code-identity fault,
/// then puts them back on drop. A rotation inside the guard must not leave
/// the next test sealing with a different key.
#[cfg(test)]
pub(crate) struct ExclusiveHostKey {
    release: bool,
    restore: Option<MacProcessSnapshot>,
    _guard: Option<std::sync::MutexGuard<'static, ()>>,
}

#[cfg(test)]
impl ExclusiveHostKey {
    pub(crate) fn acquire() -> Self {
        if HOST_KEY_HELD.with(|held| held.get()) {
            return Self {
                release: false,
                restore: None,
                _guard: None,
            };
        }
        let guard = host_key_test_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        HOST_KEY_HELD.with(|held| held.set(true));
        let restore = Some(snapshot_mac_process());
        Self {
            release: true,
            restore,
            _guard: Some(guard),
        }
    }
}

#[cfg(test)]
impl Drop for ExclusiveHostKey {
    fn drop(&mut self) {
        if self.release {
            if let Some(snapshot) = self.restore.take() {
                restore_mac_process(&snapshot);
            }
            HOST_KEY_HELD.with(|held| held.set(false));
        }
    }
}

#[cfg(test)]
struct MacProcessSnapshot {
    values: BTreeMap<String, StoredItem>,
    fault: MacFault,
    session: SessionMacMap,
    gave_up: bool,
}

#[cfg(test)]
fn snapshot_mac_process() -> MacProcessSnapshot {
    let values = memory_backend()
        .map(|memory| {
            memory
                .values
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        })
        .unwrap_or_default();
    let session = session_mac_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    MacProcessSnapshot {
        values,
        fault: lock_fault().clone(),
        session,
        gave_up: keychain_gave_up().load(std::sync::atomic::Ordering::Relaxed),
    }
}

#[cfg(test)]
fn restore_mac_process(snapshot: &MacProcessSnapshot) {
    if let Some(memory) = memory_backend() {
        *memory
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = snapshot.values.clone();
    }
    *lock_fault() = snapshot.fault.clone();
    *session_mac_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = snapshot.session.clone();
    keychain_gave_up().store(snapshot.gave_up, std::sync::atomic::Ordering::Relaxed);
}

/// Run `body` with the permission MAC absent, then put the previous key back.
///
/// The in-memory seal store is process-global. Legacy-adopt tests need a
/// profile whose host has never created that key, including when an earlier
/// test already did.
#[cfg(test)]
pub(crate) fn without_mac_key<T>(body: impl FnOnce() -> T) -> T {
    let _exclusive = ExclusiveHostKey::acquire();
    if let Some(memory) = memory_backend() {
        memory
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|account, _| !is_mac_account(account));
    }
    clear_session_mac();
    body()
}

#[cfg(test)]
thread_local! {
    static HOST_KEY_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
#[derive(Clone)]
struct StoredItem {
    value: String,
    /// Code identity that wrote this item. A later identity cannot read it
    /// unless `stable` was recorded while stable access was feasible.
    generation: u64,
    stable: bool,
    /// Silent reads fail with `errSecAuthFailed` (-25293).
    silent_unreadable: bool,
    /// Prompts are allowed and the read still fails. Rotation is the only way in.
    interactive_unreadable: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum KeychainBlock {
    None,
    /// Simulated in-place ACL edit block.
    SetAccess,
    /// Every keychain call, including an ACL edit.
    AnyCall,
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Call,
    SetAccess,
}

#[cfg(test)]
#[derive(Clone)]
struct MacFault {
    generation: u64,
    /// Designated-requirement ACLs survive a rebuild only for a stable signer.
    /// Ad-hoc builds leave this false: the next identity cannot read the item.
    stable_feasible: bool,
    /// Deleting an unreadable item would raise a keychain prompt.
    delete_unreadable_needs_prompt: bool,
    /// Secret Service / host seal store is absent (e.g. headless Linux session bus missing).
    service_unavailable: bool,
    interactive_reads: u32,
    backend_gets: u32,
    block: KeychainBlock,
    /// Times startup entered an ACL-mutating API (`note_rebuild_stable_access`).
    acl_mutations: u32,
    /// The mock stopped blocking on its own because startup never timed out.
    block_expired: bool,
}

#[cfg(test)]
fn mac_fault() -> &'static Mutex<MacFault> {
    static FAULT: Mutex<MacFault> = Mutex::new(MacFault {
        generation: 0,
        stable_feasible: true,
        delete_unreadable_needs_prompt: false,
        service_unavailable: false,
        interactive_reads: 0,
        backend_gets: 0,
        block: KeychainBlock::None,
        acl_mutations: 0,
        block_expired: false,
    });
    &FAULT
}

#[cfg(test)]
pub(crate) fn backend_get_calls() -> u32 {
    lock_fault().backend_gets
}

#[cfg(test)]
pub(crate) fn reset_backend_get_calls() {
    lock_fault().backend_gets = 0;
}

#[cfg(test)]
fn lock_fault() -> std::sync::MutexGuard<'static, MacFault> {
    mac_fault()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
fn block_var() -> &'static std::sync::Condvar {
    static VAR: std::sync::Condvar = std::sync::Condvar::new();
    &VAR
}

/// Park while the mock keychain is blocking `kind`.
///
/// The wait is longer than [`STARTUP_KEYCHAIN_DEADLINE`]. Startup has to
/// return on its own deadline; this bound only keeps a missing deadline from
/// hanging the suite.
#[cfg(test)]
fn wait_out_keychain_block(kind: BlockKind) {
    let mut fault = lock_fault();
    let give_up_at = std::time::Instant::now() + STARTUP_KEYCHAIN_DEADLINE + Duration::from_secs(3);
    while fault.blocks(kind) {
        if fault.block_expired {
            return;
        }
        let now = std::time::Instant::now();
        if now >= give_up_at {
            fault.block_expired = true;
            return;
        }
        let (guard, status) = block_var()
            .wait_timeout(fault, give_up_at.saturating_duration_since(now))
            .unwrap_or_else(|error| error.into_inner());
        fault = guard;
        if status.timed_out() && fault.blocks(kind) {
            fault.block_expired = true;
            return;
        }
    }
}

#[cfg(test)]
impl MacFault {
    fn blocks(&self, kind: BlockKind) -> bool {
        match self.block {
            KeychainBlock::None => false,
            KeychainBlock::AnyCall => true,
            KeychainBlock::SetAccess => kind == BlockKind::SetAccess,
        }
    }
}

/// Mock keychain that blocks in an ACL edit or in every call.
///
/// Drop releases the wait, joins any startup call the deadline abandoned, and
/// clears the gave-up flag so the next test can read the key. The guard holds
/// the process-wide host-key lock, so no other test observes the block.
#[cfg(test)]
pub(crate) struct BlockingKeychain {
    previous: KeychainBlock,
    _exclusive: ExclusiveHostKey,
}

#[cfg(test)]
impl BlockingKeychain {
    pub(crate) fn set_access() -> Self {
        Self::arm(KeychainBlock::SetAccess)
    }

    pub(crate) fn any_call() -> Self {
        Self::arm(KeychainBlock::AnyCall)
    }

    fn arm(block: KeychainBlock) -> Self {
        let exclusive = ExclusiveHostKey::acquire();
        clear_session_mac();
        let mut fault = lock_fault();
        let previous = fault.block;
        fault.block = block;
        fault.acl_mutations = 0;
        fault.block_expired = false;
        drop(fault);
        Self {
            previous,
            _exclusive: exclusive,
        }
    }
}

#[cfg(test)]
impl Drop for BlockingKeychain {
    fn drop(&mut self) {
        {
            let mut fault = lock_fault();
            fault.block = self.previous;
            fault.block_expired = false;
        }
        block_var().notify_all();
        keychain_gave_up().store(false, std::sync::atomic::Ordering::Relaxed);
        clear_session_mac();
        join_abandoned_keychain_calls();
    }
}

#[cfg(test)]
pub(crate) fn acl_mutations() -> u32 {
    lock_fault().acl_mutations
}

#[cfg(test)]
pub(crate) fn keychain_block_expired() -> bool {
    lock_fault().block_expired
}

#[cfg(test)]
fn item_data_readable(item: &StoredItem, fault: &MacFault, interaction: bool) -> bool {
    if item.generation != fault.generation && !(item.stable && fault.stable_feasible) {
        return false;
    }
    if item.silent_unreadable && !interaction {
        return false;
    }
    if item.interactive_unreadable && interaction {
        return false;
    }
    true
}

#[cfg(test)]
fn auth_failed() -> String {
    key_unreadable("-25293 errSecAuthFailed")
}

/// The permission MAC exists, but silent reads fail the way a rebuilt binary
/// fails with `errSecAuthFailed` while prompts are off.
///
/// An interactive read counts once and then succeeds, which is the Always Allow
/// outcome, unless [`UnreadableMacKey::interactive_also_denied`] is set. Delete
/// of that unreadable item succeeds without a prompt unless
/// [`UnreadableMacKey::delete_needs_prompt`] is set. The guard holds the
/// process-wide host-key lock so other tests do not observe the denial.
#[cfg(test)]
pub(crate) struct UnreadableMacKey {
    _exclusive: ExclusiveHostKey,
}

#[cfg(test)]
impl UnreadableMacKey {
    pub(crate) fn acquire() -> Self {
        let exclusive = ExclusiveHostKey::acquire();
        clear_session_mac();
        {
            let mut fault = lock_fault();
            fault.interactive_reads = 0;
            fault.delete_unreadable_needs_prompt = false;
        }
        mark_mac_item(|item| {
            item.silent_unreadable = true;
            item.interactive_unreadable = false;
        });
        Self {
            _exclusive: exclusive,
        }
    }

    /// Silent delete of the unreadable item fails until prompts are allowed.
    pub(crate) fn delete_needs_prompt(self) -> Self {
        lock_fault().delete_unreadable_needs_prompt = true;
        self
    }

    /// An Allow click still cannot read the old secret. The click must rotate.
    pub(crate) fn interactive_also_denied(self) -> Self {
        mark_mac_item(|item| item.interactive_unreadable = true);
        self
    }

    pub(crate) fn interactive_reads(&self) -> u32 {
        lock_fault().interactive_reads
    }
}

#[cfg(test)]
impl Drop for UnreadableMacKey {
    fn drop(&mut self) {
        mark_mac_item(|item| {
            item.silent_unreadable = false;
            item.interactive_unreadable = false;
        });
        let mut fault = lock_fault();
        fault.delete_unreadable_needs_prompt = false;
        fault.interactive_reads = 0;
        drop(fault);
        clear_session_mac();
    }
}

/// Simulates a missing OS secret store (e.g. Linux Secret Service / D-Bus session
/// bus absent), where every host-key backend operation fails with
/// `plaintext_seal_refusal`.
#[cfg(test)]
pub(crate) struct UnavailableHostKeyStore {
    _exclusive: ExclusiveHostKey,
}

#[cfg(test)]
impl UnavailableHostKeyStore {
    pub(crate) fn acquire() -> Self {
        let exclusive = ExclusiveHostKey::acquire();
        clear_session_mac();
        lock_fault().service_unavailable = true;
        Self {
            _exclusive: exclusive,
        }
    }
}

#[cfg(test)]
impl Drop for UnavailableHostKeyStore {
    fn drop(&mut self) {
        lock_fault().service_unavailable = false;
        clear_session_mac();
    }
}

/// Same-signer rebuilds can keep keychain access. Ad-hoc tests turn this off
/// before the key is written. Restores the previous value on drop.
#[cfg(test)]
pub(crate) struct RebuildStableAccess {
    previous: bool,
}

#[cfg(test)]
impl RebuildStableAccess {
    pub(crate) fn set(feasible: bool) -> Self {
        let mut fault = lock_fault();
        let previous = fault.stable_feasible;
        fault.stable_feasible = feasible;
        Self { previous }
    }
}

#[cfg(test)]
impl Drop for RebuildStableAccess {
    fn drop(&mut self) {
        lock_fault().stable_feasible = self.previous;
    }
}

/// The next host-key read is a new ad-hoc code identity.
#[cfg(test)]
pub(crate) fn simulate_new_code_identity() {
    let mut fault = lock_fault();
    fault.generation = fault.generation.wrapping_add(1);
    drop(fault);
    clear_session_mac();
}

#[cfg(test)]
fn mark_mac_item(mut update: impl FnMut(&mut StoredItem)) {
    let Some(memory) = memory_backend() else {
        return;
    };
    let mut map = memory
        .values
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for (account, item) in map.iter_mut() {
        if is_mac_account(account) {
            update(item);
        }
    }
}

#[cfg(test)]
pub(crate) fn mark_profile_mac_unreadable(dir: &std::path::Path) {
    let account = crate::broker::seal::mac_account(dir);
    if let Some(memory) = memory_backend() {
        if let Some(item) = memory
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_mut(&account)
        {
            item.silent_unreadable = true;
        }
    }
    clear_session_mac();
}

#[cfg(test)]
fn host_key_test_lock() -> &'static Mutex<()> {
    static LOCK: Mutex<()> = Mutex::new(());
    &LOCK
}

#[cfg(test)]
pub(crate) struct TestBackendGuard {
    release: bool,
    _guard: Option<std::sync::MutexGuard<'static, ()>>,
}

#[cfg(test)]
impl TestBackendGuard {
    pub(crate) fn acquire_if_needed() -> Self {
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

#[cfg(test)]
fn memory_backend() -> Option<&'static MemoryBackend> {
    use std::sync::OnceLock;
    static MEMORY: OnceLock<MemoryBackend> = OnceLock::new();
    Some(MEMORY.get_or_init(MemoryBackend::default))
}

/// True when running under an ephemeral `pr-<N>` test channel without an
/// explicit `PLEXI_KEYCHAIN_PATH`, so host-seal items must stay in
/// `<profile>/host/pr-host-seal.json` and never touch the user's login Keychain.
pub(crate) fn should_use_pr_profile_seal_store(
    compiled_test_channel: Option<&str>,
    runtime_channel: Option<&str>,
    keychain_path_env: Option<&str>,
) -> bool {
    if keychain_path_env.is_some_and(|p| !p.trim().is_empty()) {
        return false;
    }
    let Some(compiled) = compiled_test_channel else {
        return false;
    };
    compiled == runtime_channel.unwrap_or_default()
        && compiled
            .strip_prefix("pr-")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
fn test_pr_isolation_slot() -> &'static Mutex<Option<String>> {
    static SLOT: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

#[cfg(test)]
pub(crate) fn with_pr_channel_seal_isolation<T>(
    profile_dir: &std::path::Path,
    pr_channel: &str,
    body: impl FnOnce() -> T,
) -> T {
    let _exclusive = ExclusiveHostKey::acquire();
    let _profile_guard = crate::config::set_test_profile_dir(profile_dir.to_path_buf());
    let _channel_guard = crate::config::set_test_channel(pr_channel);
    *test_pr_isolation_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = Some(pr_channel.to_string());
    clear_session_mac();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    *test_pr_isolation_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = None;
    clear_session_mac();
    match result {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

fn pr_isolation_active() -> bool {
    #[cfg(test)]
    let compiled = test_pr_isolation_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    #[cfg(not(test))]
    let compiled = option_env!("PLEXI_BUILD_TEST_CHANNEL").map(str::to_string);
    let runtime = crate::config::build_channel();
    let keychain_path = std::env::var("PLEXI_KEYCHAIN_PATH").ok();
    should_use_pr_profile_seal_store(
        compiled.as_deref(),
        runtime.as_deref(),
        keychain_path.as_deref(),
    )
}

fn backend() -> &'static dyn HostKeyBackend {
    static PR_BACKEND: PrProfileBackend = PrProfileBackend;
    if pr_isolation_active() {
        return &PR_BACKEND;
    }
    #[cfg(test)]
    {
        memory_backend().expect("test host-key backend")
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

struct PrProfileBackend;

impl PrProfileBackend {
    fn store_path() -> Result<std::path::PathBuf, String> {
        let profile = crate::config::config_dir();
        let host_dir = profile.join("host");
        if std::fs::symlink_metadata(&host_dir).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!(
                "pr host directory {} is a symlink",
                host_dir.display()
            ));
        }
        std::fs::create_dir_all(&host_dir)
            .map_err(|error| format!("create {}: {error}", host_dir.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&host_dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("chmod 700 {}: {error}", host_dir.display()))?;
        }
        let path = host_dir.join("pr-host-seal.json");
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("pr host seal store {} is a symlink", path.display()));
        }
        Ok(path)
    }

    fn load_map() -> Result<BTreeMap<String, String>, String> {
        let path = Self::store_path()?;
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("parse {}: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(format!("read {}: {error}", path.display())),
        }
    }

    fn save_map(map: &BTreeMap<String, String>) -> Result<(), String> {
        let path = Self::store_path()?;
        let bytes = serde_json::to_vec_pretty(map).map_err(|error| error.to_string())?;
        crate::platform::fs::atomic_write_with_mode(&path, &bytes, 0o600)
    }
}

impl HostKeyBackend for PrProfileBackend {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, String> {
        let map = Self::load_map()?;
        Ok(map.get(account).cloned().map(Zeroizing::new))
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), String> {
        let mut map = Self::load_map()?;
        if map.contains_key(account) {
            return Err(format!("host key already exists: {account}"));
        }
        map.insert(account.to_string(), value.to_string());
        Self::save_map(&map)?;
        log::info!("permission_seal: stored pr-isolated host item {account}");
        Ok(())
    }

    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        let mut map = Self::load_map()?;
        map.insert(account.to_string(), value.to_string());
        Self::save_map(&map)?;
        log::info!("permission_seal: updated pr-isolated host item {account}");
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        let mut map = Self::load_map()?;
        if map.remove(account).is_some() {
            Self::save_map(&map)?;
        }
        Ok(())
    }
}

trait HostKeyBackend: Sync + Send {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, String>;
    fn add_new(&self, account: &str, value: &str) -> Result<(), String>;
    #[cfg_attr(not(test), allow(dead_code))]
    fn set(&self, account: &str, value: &str) -> Result<(), String>;
    /// Remove the item without reading its secret. An ACL that blocks the
    /// password must not, by itself, block deletion.
    fn delete(&self, account: &str) -> Result<(), String>;
    /// Test stand-in for an ACL edit. Startup must not call it: the real
    /// keychain call blocks on an authorization dialog with prompts disabled.
    #[cfg(test)]
    fn note_rebuild_stable_access(&self, _account: &str) {}
}

#[cfg(test)]
struct MemoryBackend {
    values: Mutex<BTreeMap<String, StoredItem>>,
}

#[cfg(test)]
impl Default for MemoryBackend {
    fn default() -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
        }
    }
}

#[cfg(test)]
impl MemoryBackend {
    fn fresh_item(&self, value: &str) -> StoredItem {
        let fault = lock_fault();
        StoredItem {
            value: value.to_string(),
            generation: fault.generation,
            stable: false,
            silent_unreadable: false,
            interactive_unreadable: false,
        }
    }
}

#[cfg(test)]
impl HostKeyBackend for MemoryBackend {
    fn get(&self, account: &str) -> Result<Option<Zeroizing<String>>, String> {
        {
            let mut fault = lock_fault();
            fault.backend_gets += 1;
            if fault.service_unavailable {
                return Err(plaintext_seal_refusal(
                    "Failed to connect to address `unix:path=/run/user/1000/bus`: No such file or directory (os error 2)",
                ));
            }
        }
        wait_out_keychain_block(BlockKind::Call);
        let mut map = self
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(item) = map.get_mut(account) else {
            return Ok(None);
        };
        let interaction = interaction_allowed();
        let fault = lock_fault();
        if !item_data_readable(item, &fault, interaction) {
            return Err(auth_failed());
        }
        drop(fault);
        if interaction && item.silent_unreadable && !item.interactive_unreadable {
            item.silent_unreadable = false;
            lock_fault().interactive_reads += 1;
        }
        Ok(Some(Zeroizing::new(item.value.clone())))
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), String> {
        if lock_fault().service_unavailable {
            return Err(plaintext_seal_refusal(
                "Failed to connect to address `unix:path=/run/user/1000/bus`: No such file or directory (os error 2)",
            ));
        }
        wait_out_keychain_block(BlockKind::Call);
        let mut map = self
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if map.contains_key(account) {
            return Err(format!("host key already exists: {account}"));
        }
        map.insert(account.to_string(), self.fresh_item(value));
        Ok(())
    }

    fn set(&self, account: &str, value: &str) -> Result<(), String> {
        if lock_fault().service_unavailable {
            return Err(plaintext_seal_refusal(
                "Failed to connect to address `unix:path=/run/user/1000/bus`: No such file or directory (os error 2)",
            ));
        }
        wait_out_keychain_block(BlockKind::Call);
        let mut map = self
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(existing) = map.get(account) {
            let fault = lock_fault();
            if !item_data_readable(existing, &fault, interaction_allowed()) {
                return Err(auth_failed());
            }
        }
        map.insert(account.to_string(), self.fresh_item(value));
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        if lock_fault().service_unavailable {
            return Err(plaintext_seal_refusal(
                "Failed to connect to address `unix:path=/run/user/1000/bus`: No such file or directory (os error 2)",
            ));
        }
        wait_out_keychain_block(BlockKind::Call);
        let mut map = self
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(existing) = map.get(account) else {
            return Ok(());
        };
        let fault = lock_fault();
        let blocked = !item_data_readable(existing, &fault, interaction_allowed())
            && fault.delete_unreadable_needs_prompt
            && !interaction_allowed();
        drop(fault);
        if blocked {
            return Err(auth_failed());
        }
        map.remove(account);
        Ok(())
    }

    fn note_rebuild_stable_access(&self, account: &str) {
        {
            let mut fault = lock_fault();
            fault.acl_mutations += 1;
        }
        wait_out_keychain_block(BlockKind::SetAccess);
        let fault = lock_fault();
        if !fault.stable_feasible {
            return;
        }
        drop(fault);
        if let Some(item) = self
            .values
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_mut(account)
        {
            item.stable = true;
        }
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
        // Keychain interaction is process-wide. Do not disable an approved
        // secret read's UI, or let a background seal read inherit its UI.
        let _keychain = crate::workspace::secrets::keychain_call_lock();
        let _prompts = PromptScope::enter();
        let chain = keychain()?;
        match chain.find_generic_password(HOST_SERVICE, account) {
            Ok((password, item)) => {
                let bytes = password.as_ref();
                let text = String::from_utf8(bytes.to_vec())
                    .map_err(|_| "host seal item is not utf-8".to_string())?;
                drop(password);
                drop(item);
                if super::interaction_allowed() {
                    // Replace the item via attribute delete + create instead of
                    // editing the existing item's ACL in place, which would
                    // force a second SecurityAgent password prompt.
                    if let Err(error) = delete_without_reading_secret(&chain, account)
                        .and_then(|()| create_with_access(&chain, account, &text))
                    {
                        log::warn!(
                            "permission_seal: could not refresh keychain ACL for {account} after interactive read: {error}"
                        );
                    }
                }
                Ok(Some(Zeroizing::new(text)))
            }
            Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(error) => Err(map_keychain_error(account, error)),
        }
    }

    // errSecDuplicateItem.
    const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;

    pub(super) fn add_new(account: &str, value: &str) -> Result<(), String> {
        // Keychain interaction is process-wide. Do not disable an approved
        // secret read's UI, or let a background seal read inherit its UI.
        let _keychain = crate::workspace::secrets::keychain_call_lock();
        let _prompts = PromptScope::enter();
        let chain = keychain()?;
        // The creating binary is the trusted application. `security` is not,
        // so a direct keychain read does not return the secret. The ACL is
        // then widened to this app's designated requirement, never to any app.
        create_with_access(&chain, account, value)?;
        log::info!("permission_seal: stored host item {account} in the macOS keychain");
        Ok(())
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn set(account: &str, value: &str) -> Result<(), String> {
        // Keychain interaction is process-wide. Do not disable an approved
        // secret read's UI, or let a background seal read inherit its UI.
        let _keychain = crate::workspace::secrets::keychain_call_lock();
        let _prompts = PromptScope::enter();
        let chain = keychain()?;
        // Editing an existing item's ACL asks securityd for the keychain
        // password, which blocked host startup on a SecurityAgent dialog.
        // Replace the item instead: the creator sets the ACL with no prompt.
        delete_without_reading_secret(&chain, account)?;
        create_with_access(&chain, account, value)?;
        log::info!("permission_seal: updated host item {account} in the macOS keychain");
        Ok(())
    }

    pub(super) fn delete(account: &str) -> Result<(), String> {
        // Keychain interaction is process-wide. Do not disable an approved
        // secret read's UI, or let a background seal read inherit its UI.
        let _keychain = crate::workspace::secrets::keychain_call_lock();
        let _prompts = PromptScope::enter();
        let chain = keychain()?;
        // Attribute delete does not read the secret. `find_generic_password`
        // returns -25293 when a previous build's ACL rejects this binary, which
        // used to make rotation unable to remove the item it cannot read.
        match delete_without_reading_secret(&chain, account) {
            Ok(()) => {
                log::info!("permission_seal: deleted host item {account} from the macOS keychain");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn delete_without_reading_secret(chain: &SecKeychain, account: &str) -> Result<(), String> {
        // Search attributes in the explicit keychain without reading password
        // data. SecItemDelete's kSecUseKeychain query can silently search the
        // login keychain instead of a private keychain supplied by the caller.
        #[link(name = "Security", kind = "framework")]
        extern "C" {
            fn SecKeychainSearchCreateFromAttributes(
                keychain: core_foundation::base::CFTypeRef,
                item_class: u32,
                attrs: *const KcAttributeList,
                search: *mut core_foundation::base::CFTypeRef,
            ) -> i32;
            fn SecKeychainSearchCopyNext(
                search: core_foundation::base::CFTypeRef,
                item: *mut security_framework_sys::base::SecKeychainItemRef,
            ) -> i32;
            fn SecKeychainItemDelete(item: security_framework_sys::base::SecKeychainItemRef) -> i32;
        }
        let mut service = HOST_SERVICE.as_bytes().to_vec();
        let mut account_bytes = account.as_bytes().to_vec();
        let mut attrs = [
            KcAttribute { tag: four_cc(b"svce"), length: service.len() as u32, data: service.as_mut_ptr().cast() },
            KcAttribute { tag: four_cc(b"acct"), length: account_bytes.len() as u32, data: account_bytes.as_mut_ptr().cast() },
        ];
        let list = KcAttributeList { count: attrs.len() as u32, attr: attrs.as_mut_ptr() };
        let mut search = std::ptr::null();
        let status = unsafe {
            SecKeychainSearchCreateFromAttributes(chain.as_CFTypeRef(), four_cc(b"genp"), &list, &mut search)
        };
        if status == ERR_SEC_ITEM_NOT_FOUND { return Ok(()); }
        if status != errSecSuccess {
            return Err(map_keychain_error(account, Error::from_code(status)));
        }
        let search = unsafe { CFType::wrap_under_create_rule(search) };
        loop {
            let mut item = std::ptr::null_mut();
            let status = unsafe { SecKeychainSearchCopyNext(search.as_CFTypeRef(), &mut item) };
            if status == ERR_SEC_ITEM_NOT_FOUND { return Ok(()); }
            if status != errSecSuccess {
                return Err(map_keychain_error(account, Error::from_code(status)));
            }
            let item = unsafe { SecKeychainItem::wrap_under_create_rule(item) };
            let status = unsafe { SecKeychainItemDelete(item.as_concrete_TypeRef()) };
            if status != errSecSuccess && status != ERR_SEC_ITEM_NOT_FOUND {
                return Err(map_keychain_error(account, Error::from_code(status)));
            }
        }
    }

    #[repr(C)]
    struct KcAttribute {
        tag: u32,
        length: u32,
        data: *mut std::ffi::c_void,
    }

    #[repr(C)]
    struct KcAttributeList {
        count: u32,
        attr: *mut KcAttribute,
    }

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        fn SecKeychainItemCreateFromContent(
            item_class: u32,
            attr_list: *mut KcAttributeList,
            length: u32,
            data: *const std::ffi::c_void,
            keychain: security_framework_sys::base::SecKeychainRef,
            initial_access: SecAccessRef,
            item: *mut security_framework_sys::base::SecKeychainItemRef,
        ) -> i32;
    }

    const fn four_cc(code: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*code)
    }

    /// `kSecLabelItemAttr` (`0x00000007`) sets the human-readable item label in
    /// Keychain Access and SecurityAgent prompts.
    const ATTR_LABEL: u32 = 7;

    /// Create the item with its ACL already attached. The creating process may
    /// set the initial access without authorization, so no SecurityAgent
    /// dialog can block `host start`.
    fn create_with_access(chain: &SecKeychain, account: &str, value: &str) -> Result<(), String> {
        let human_label = super::keychain_item_label(account);
        let access = unsafe { designated_requirement_access(&human_label) };
        let access_ref = match &access {
            Ok(access) => access.as_concrete_TypeRef(),
            Err(status) => {
                log::error!(
                    "permission_seal: could not build a designated-requirement ACL for {account}: {status}; using the default creator ACL"
                );
                std::ptr::null_mut()
            }
        };
        let mut label_bytes = human_label.into_bytes();
        let mut service = HOST_SERVICE.as_bytes().to_vec();
        let mut account_bytes = account.as_bytes().to_vec();
        let mut attrs = [
            KcAttribute {
                tag: ATTR_LABEL,
                length: label_bytes.len() as u32,
                data: label_bytes.as_mut_ptr().cast(),
            },
            KcAttribute {
                tag: four_cc(b"svce"),
                length: service.len() as u32,
                data: service.as_mut_ptr().cast(),
            },
            KcAttribute {
                tag: four_cc(b"acct"),
                length: account_bytes.len() as u32,
                data: account_bytes.as_mut_ptr().cast(),
            },
        ];
        let mut list = KcAttributeList {
            count: attrs.len() as u32,
            attr: attrs.as_mut_ptr(),
        };
        let mut item: security_framework_sys::base::SecKeychainItemRef = std::ptr::null_mut();
        let status = unsafe {
            SecKeychainItemCreateFromContent(
                four_cc(b"genp"),
                &mut list,
                value.len() as u32,
                value.as_ptr().cast(),
                chain.as_concrete_TypeRef(),
                access_ref,
                &mut item,
            )
        };
        if !item.is_null() {
            drop(unsafe { SecKeychainItem::wrap_under_create_rule(item) });
        }
        if status == ERR_SEC_DUPLICATE_ITEM {
            return Err(format!("host key already exists: {account}"));
        }
        if status != errSecSuccess {
            return Err(map_keychain_error(account, Error::from_code(status)));
        }
        if access.is_ok() && super::is_mac_account(account) {
            log::info!(
                "permission_seal: keychain ACL for {account} trusts this app's designated requirement"
            );
        }
        Ok(())
    }

    unsafe fn designated_requirement_access(descriptor: &str) -> Result<SecAccess, i32> {
        let mut app: *mut std::ffi::c_void = std::ptr::null_mut();
        let status = SecTrustedApplicationCreateFromPath(std::ptr::null(), &mut app);
        if status != errSecSuccess || app.is_null() {
            return Err(if status == errSecSuccess { -1 } else { status });
        }
        let app = CFType::wrap_under_create_rule(app as CFTypeRef);
        let trusted = CFArray::<CFType>::from_CFTypes(&[app]);
        let label = CFString::new(descriptor);
        let mut access: SecAccessRef = std::ptr::null_mut();
        let status = SecAccessCreate(
            label.as_concrete_TypeRef().cast(),
            trusted.as_concrete_TypeRef().cast(),
            &mut access,
        );
        if status != errSecSuccess || access.is_null() {
            return Err(if status == errSecSuccess { -1 } else { status });
        }
        Ok(SecAccess::wrap_under_create_rule(access))
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
struct RestoreMacItem {
    previous: Option<String>,
}

#[cfg(test)]
impl RestoreMacItem {
    fn take() -> Self {
        let previous = get(MAC_ITEM)
            .ok()
            .flatten()
            .map(|value| value.to_string());
        let _ = delete(MAC_ITEM);
        Self { previous }
    }
}

#[cfg(test)]
impl Drop for RestoreMacItem {
    fn drop(&mut self) {
        let _ = delete(MAC_ITEM);
        if let Some(value) = self.previous.take() {
            if let Err(error) = set(MAC_ITEM, &value) {
                log::error!("permission_seal: test could not restore permission mac: {error}");
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn clear_session_mac_for_test() {
    clear_session_mac();
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
        add_new("probe-item", "host-only-key").unwrap();
        assert_eq!(
            get("probe-item").unwrap().unwrap().as_str(),
            "host-only-key"
        );
        assert!(system_store().get("plexi:host:permission-mac").is_none());
        assert!(system_store().get("permission-mac").is_none());
        assert!(!dir.path().join("secrets.json").exists());
        let label = keychain_item_label("plexi:host:permission-mac:abcd");
        assert!(
            label.contains("Local Permission & Approval Signing Key"),
            "{label}"
        );
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

    /// A test that never took the host-key lock calls the keychain while
    /// another test holds it with a blocking mock armed. The caller must wait
    /// for the lock, not see the armed block, time out, and leave an
    /// abandoned call that the blocking test's teardown joins while it still
    /// holds the lock that call is waiting for.
    #[test]
    fn unlocked_caller_waits_out_a_blocking_test_instead_of_deadlocking_it() {
        let account = "plexi:host:blocking-neighbour-probe";
        let (armed_tx, armed_rx) = std::sync::mpsc::channel();
        let (torn_down_tx, torn_down_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _exclusive = ExclusiveHostKey::acquire();
            let block = BlockingKeychain::any_call();
            armed_tx.send(()).unwrap();
            std::thread::sleep(STARTUP_KEYCHAIN_DEADLINE + Duration::from_secs(1));
            drop(block);
            let _ = torn_down_tx.send(());
        });
        std::thread::spawn(move || {
            armed_rx.recv().unwrap();
            let _ = done_tx.send(get(account));
        });
        let bound = Duration::from_secs(30);
        torn_down_rx
            .recv_timeout(bound)
            .expect("the blocking test's teardown deadlocked joining the unlocked call");
        let result = done_rx
            .recv_timeout(bound)
            .expect("the unlocked keychain call never returned");
        assert_eq!(result, Ok(None), "the unlocked caller observed another test's block");
    }

    #[test]
    fn unreadable_item_can_be_deleted_without_reading_it() {
        let _guard = ExclusiveHostKey::acquire();
        let _restore = RestoreMacItem::take();
        add_new(MAC_ITEM, "previous-build-key").unwrap();
        let denied = UnreadableMacKey::acquire();
        let error = get(MAC_ITEM).expect_err("silent read fails");
        assert!(is_key_unreadable(&error), "{error}");
        assert!(error.contains("-25293"), "{error}");
        assert_eq!(denied.interactive_reads(), 0);
        delete(MAC_ITEM).expect("delete does not need the secret");
        assert!(get(MAC_ITEM).unwrap().is_none());
    }

    #[test]
    fn new_code_identity_cannot_read_a_key_whose_acl_was_not_edited() {
        let _guard = ExclusiveHostKey::acquire();
        let _restore = RestoreMacItem::take();
        let _stable = RebuildStableAccess::set(true);
        let before = acl_mutations();
        add_new(MAC_ITEM, "stable-key").unwrap();
        assert_eq!(
            acl_mutations(),
            before,
            "startup must not call an ACL-mutating API"
        );
        simulate_new_code_identity();
        let error = get(MAC_ITEM).expect_err("the mock ACL was not widened");
        assert!(is_key_unreadable(&error), "{error}");
        assert!(error.contains("-25293"), "{error}");
    }

    #[test]
    fn acl_mutation_is_counted_when_invoked_directly() {
        let _guard = ExclusiveHostKey::acquire();
        let before = acl_mutations();
        super::backend().note_rebuild_stable_access("acl-probe");
        assert_eq!(acl_mutations(), before + 1);
    }

    #[test]
    fn ad_hoc_identity_cannot_read_the_previous_key() {
        let _guard = ExclusiveHostKey::acquire();
        let _restore = RestoreMacItem::take();
        let _adhoc = RebuildStableAccess::set(false);
        add_new(MAC_ITEM, "ad-hoc-key").unwrap();
        simulate_new_code_identity();
        let error = get(MAC_ITEM).expect_err("ad-hoc rebuild loses the acl");
        assert!(is_key_unreadable(&error), "{error}");
        assert!(error.contains("-25293"), "{error}");
        delete(MAC_ITEM).expect("rotation can remove the old item");
        add_new(MAC_ITEM, "rotated-key").unwrap();
        assert_eq!(get(MAC_ITEM).unwrap().unwrap().as_str(), "rotated-key");
    }
}
