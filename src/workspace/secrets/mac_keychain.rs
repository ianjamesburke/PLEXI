//! macOS file-Keychain operations on one selected item.
//!
//! Reads, removals and updates share one selector: the first generic-password
//! item for service `plexi` and the full account, searched in the
//! operation's ordered domain. The domain is the user's keychain search list,
//! or only the `PLEXI_KEYCHAIN_PATH` test files. It is resolved once per
//! operation. Selection asks `SecKeychainFindGenericPassword` for the item
//! reference only (both password outputs null), so it never evaluates the
//! item's value ACL. A removal deletes the reference it selected and keeps
//! the status `SecKeychainItemDelete` returns.
//!
//! Every operation turns Keychain interaction off first, because the switch
//! is process-wide and an interactive scope elsewhere may have failed to
//! restore it. If that fails, nothing else runs. Removal never turns it on.
//!
//! The logic is written against [`Native`] so tests can drive every status
//! without a keychain. The production [`SecurityFramework`] is a one-to-one
//! pass-through and, like `MacKeychain`, is absent from test builds.

use zeroize::Zeroizing;

use super::store::{DeleteOutcome, KeychainStage, SecretError};
use super::KEYCHAIN_SERVICE;

/// `errSecItemNotFound`.
const ITEM_NOT_FOUND: i32 = -25300;
/// `errSecDuplicateItem`.
const DUPLICATE_ITEM: i32 = -25299;

/// The keychains one operation searches.
pub(super) struct Domain<K> {
    /// Ordered keychains a selection searches; the first match wins.
    pub search: Vec<K>,
    /// Whether an attributes-only query can enumerate `search` (the user's
    /// search list). An explicit file is off-list, so a scan confirms known
    /// names by selection instead.
    pub enumerable: bool,
}

/// One method per Security call this backend makes. The production
/// implementation passes each call straight through and returns its status.
pub(super) trait Native {
    type Keychain;
    type Item;
    /// `SecKeychainSetUserInteractionAllowed`.
    fn set_interaction_allowed(&self, allowed: bool) -> i32;
    /// Opens the operation's domain. An error opening an explicit file is
    /// terminal; no other keychain is consulted.
    fn open_domain(&self) -> Result<Domain<Self::Keychain>, SecretError>;
    /// Opens the destination for a new item. Called only when a write is
    /// about to create one; removal and reads never open it.
    fn open_default(&self) -> Result<Self::Keychain, SecretError>;
    /// Attributes-only enumeration of accounts under `service` in the search
    /// list. `errSecItemNotFound` is an empty list.
    fn list_accounts(&self, service: &str) -> Result<Vec<String>, i32>;
    /// `SecKeychainFindGenericPassword` over `search` with both password
    /// outputs null: the item reference only, never its data.
    fn find_item(
        &self,
        search: &[Self::Keychain],
        service: &str,
        account: &str,
    ) -> (i32, Option<Self::Item>);
    /// `SecKeychainItemCopyContent` on a selected item.
    fn copy_data(&self, item: &Self::Item) -> Result<Zeroizing<Vec<u8>>, i32>;
    /// `SecKeychainItemDelete` on a selected item.
    fn delete_item(&self, item: &Self::Item) -> i32;
    /// `SecKeychainItemModifyAttributesAndData` replacing the data only.
    fn modify_data(&self, item: &Self::Item, value: &[u8]) -> i32;
    /// Create-only add of a labeled generic password to `keychain`.
    fn add_item(
        &self,
        keychain: &Self::Keychain,
        service: &str,
        account: &str,
        label: &str,
        value: &[u8],
    ) -> i32;
    /// Whether two handles name the same keychain file.
    fn same_keychain(&self, left: &Self::Keychain, right: &Self::Keychain) -> bool;
}

/// A finished removal attempt and what the cache needs to know.
pub(super) struct Removal {
    pub outcome: DeleteOutcome,
    /// After a confirmed deletion: whether the same selector still finds a
    /// copy in a later searched keychain. `None` when nothing was deleted.
    pub survivor: Option<Result<bool, SecretError>>,
}

fn fail(stage: KeychainStage, status: i32) -> SecretError {
    SecretError::Keychain { stage, status }
}

fn suppress_ui<N: Native>(native: &N) -> Result<(), SecretError> {
    match native.set_interaction_allowed(false) {
        0 => Ok(()),
        status => Err(fail(KeychainStage::SuppressUi, status)),
    }
}

/// Allows the existing item's authentication UI for an explicit read or a
/// user write, then turns it off again.
struct AllowUi<'a, N: Native>(&'a N);

impl<'a, N: Native> AllowUi<'a, N> {
    fn enter(native: &'a N) -> Result<Self, SecretError> {
        match native.set_interaction_allowed(true) {
            0 => {
                log::info!(
                    "workspace_secrets: allowing Keychain authentication for this operation"
                );
                Ok(Self(native))
            }
            status => Err(fail(KeychainStage::AllowUi, status)),
        }
    }
}

impl<N: Native> Drop for AllowUi<'_, N> {
    fn drop(&mut self) {
        let status = self.0.set_interaction_allowed(false);
        if status != 0 {
            log::error!(
                "workspace_secrets: restoring noninteractive Keychain mode failed: {status}"
            );
        }
    }
}

/// The one selector for an existing item.
fn select<N: Native>(
    native: &N,
    search: &[N::Keychain],
    account: &str,
) -> Result<Option<N::Item>, SecretError> {
    match native.find_item(search, KEYCHAIN_SERVICE, account) {
        (0, Some(item)) => Ok(Some(item)),
        (0, None) => Err(fail(KeychainStage::Select, 0)),
        (ITEM_NOT_FOUND, _) => Ok(None),
        (status, _) => Err(fail(KeychainStage::Select, status)),
    }
}

fn delete_selected<N: Native>(
    native: &N,
    domain: &Domain<N::Keychain>,
    item: &N::Item,
    account: &str,
) -> Result<Removal, SecretError> {
    let status = native.delete_item(item);
    log::info!("workspace_secrets: keychain delete account={account} status={status}");
    if status != 0 {
        return Err(fail(KeychainStage::Delete, status));
    }
    Ok(Removal {
        outcome: DeleteOutcome::Deleted,
        survivor: Some(select(native, &domain.search, account).map(|item| item.is_some())),
    })
}

/// Select, then delete that item. The caller holds the Keychain call lock
/// for the whole call. A selected item that is gone by delete time is an
/// error at the delete stage; this never re-selects by name to retry.
pub(super) fn remove<N: Native>(native: &N, account: &str) -> Result<Removal, SecretError> {
    suppress_ui(native)?;
    let domain = native.open_domain()?;
    let Some(item) = select(native, &domain.search, account)? else {
        log::info!("workspace_secrets: keychain remove account={account} no match reported");
        return Ok(Removal {
            outcome: DeleteOutcome::NoMatchReported,
            survivor: None,
        });
    };
    delete_selected(native, &domain, &item, account)
}

/// Migration's value-guarded delete: read, compare and delete the same
/// selected item. A cross-process write between the read and the delete is
/// not closed; macOS has no compare-and-delete.
pub(super) fn remove_if_value<N: Native>(
    native: &N,
    account: &str,
    expected: &str,
) -> Result<Removal, SecretError> {
    suppress_ui(native)?;
    let domain = native.open_domain()?;
    let Some(item) = select(native, &domain.search, account)? else {
        return Ok(Removal {
            outcome: DeleteOutcome::NoMatchReported,
            survivor: None,
        });
    };
    let current = item_text(native, &item)?;
    if current.as_str() != expected {
        return Err(SecretError::ValueChanged(account.to_string()));
    }
    delete_selected(native, &domain, &item, account)
}

fn item_text<N: Native>(native: &N, item: &N::Item) -> Result<Zeroizing<String>, SecretError> {
    let data = native
        .copy_data(item)
        .map_err(|status| fail(KeychainStage::Read, status))?;
    Ok(Zeroizing::new(
        String::from_utf8_lossy(&data).trim().to_string(),
    ))
}

/// Read the selected item's value. `interactive` allows the item's existing
/// authentication UI for an explicit or admitted read.
pub(super) fn read<N: Native>(
    native: &N,
    account: &str,
    interactive: bool,
) -> Result<Option<Zeroizing<String>>, SecretError> {
    suppress_ui(native)?;
    let domain = native.open_domain()?;
    let _ui = if interactive {
        Some(AllowUi::enter(native)?)
    } else {
        None
    };
    match select(native, &domain.search, account)? {
        Some(item) => item_text(native, &item).map(Some),
        None => Ok(None),
    }
}

/// Inventory for reconciliation. Interaction is turned off first, as for
/// every other call. The search list is enumerated attributes-only; an
/// explicit file is not enumerable, so the names the index already holds are
/// confirmed with the reference-only selector in one opened domain.
pub(super) fn scan<N: Native>(native: &N, indexed: &[String]) -> Result<Vec<String>, SecretError> {
    suppress_ui(native)?;
    let domain = native.open_domain()?;
    if domain.enumerable {
        return native
            .list_accounts(KEYCHAIN_SERVICE)
            .map_err(|status| fail(KeychainStage::Select, status));
    }
    let mut found = Vec::new();
    for account in indexed {
        if select(native, &domain.search, account)?.is_some() {
            found.push(account.clone());
        }
    }
    Ok(found)
}

/// Upsert (`create_only == false`) updates the selected item in place, or
/// creates one in the default keychain. Create-only always adds, and the
/// destination refuses a duplicate. A new item is created only when the
/// default keychain is in the search domain, so it is readable afterwards.
pub(super) fn write<N: Native>(
    native: &N,
    account: &str,
    label: &str,
    value: &str,
    create_only: bool,
) -> Result<(), SecretError> {
    suppress_ui(native)?;
    let domain = native.open_domain()?;
    let _ui = if create_only {
        None
    } else {
        Some(AllowUi::enter(native)?)
    };
    if !create_only {
        if let Some(item) = select(native, &domain.search, account)? {
            return match native.modify_data(&item, value.as_bytes()) {
                0 => Ok(()),
                status => Err(fail(KeychainStage::Update, status)),
            };
        }
    }
    let default = native.open_default()?;
    if !domain
        .search
        .iter()
        .any(|keychain| native.same_keychain(keychain, &default))
    {
        return Err(SecretError::Backend(
            "the default keychain is not in the keychain search list, so a new secret there could not be read back; nothing was stored. Add it to the search list or choose a default keychain that is on it".to_string(),
        ));
    }
    match native.add_item(&default, KEYCHAIN_SERVICE, account, label, value.as_bytes()) {
        0 => Ok(()),
        DUPLICATE_ITEM if create_only => Err(SecretError::AlreadyExists(account.to_string())),
        status => Err(fail(KeychainStage::Add, status)),
    }
}

#[cfg(all(target_os = "macos", not(test)))]
pub(super) use production::SecurityFramework;

#[cfg(all(target_os = "macos", not(test)))]
mod production {
    use core_foundation::array::CFArray;
    use core_foundation::base::TCFType;
    use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};
    use security_framework::os::macos::keychain_item::SecKeychainItem;
    use security_framework_sys::base::{SecKeychainItemRef, SecKeychainRef};
    use zeroize::Zeroizing;

    use super::{Domain, Native};
    use crate::workspace::secrets::store::{
        explicit_keychain_password, fixture_selection, keychain_path_choice, KeychainPathChoice,
        KeychainStage, SecretError,
    };

    #[link(name = "Security", kind = "framework")]
    extern "C" {
        // Not bound by security-framework-sys.
        fn SecKeychainCopySearchList(search_list: *mut core_foundation::array::CFArrayRef) -> i32;
        fn SecKeychainGetPath(
            keychain: SecKeychainRef,
            io_path_length: *mut u32,
            path_name: *mut std::os::raw::c_char,
        ) -> i32;
        fn SecKeychainItemCopyContent(
            item: SecKeychainItemRef,
            item_class: *mut u32,
            attr_list: *mut security_framework_sys::base::SecKeychainAttributeList,
            length: *mut u32,
            out_data: *mut *mut std::ffi::c_void,
        ) -> i32;
    }

    /// The real Security.framework calls.
    pub(in crate::workspace::secrets) struct SecurityFramework;

    fn keychain_path(keychain: &SecKeychain) -> Option<Vec<u8>> {
        let mut buffer = vec![0u8; 1024];
        let mut length = buffer.len() as u32;
        let status = unsafe {
            SecKeychainGetPath(
                keychain.as_concrete_TypeRef(),
                &mut length,
                buffer.as_mut_ptr().cast(),
            )
        };
        if status != 0 {
            log::warn!("workspace_secrets: SecKeychainGetPath failed: {status}");
            return None;
        }
        buffer.truncate(length as usize);
        Some(buffer)
    }

    /// Resolved once per call from the environment. A release build passes
    /// no fixture selection, so only `PLEXI_KEYCHAIN_PATH` can apply.
    fn choice() -> Result<KeychainPathChoice, SecretError> {
        let fixture = fixture_selection();
        keychain_path_choice(
            std::env::var_os("PLEXI_KEYCHAIN_PATH").as_deref(),
            fixture
                .as_ref()
                .map(|(search, default)| (search.as_os_str(), default.as_deref())),
        )
        .map_err(|error| SecretError::Backend(error.to_string()))
    }

    fn test_password() -> Result<Zeroizing<String>, SecretError> {
        Ok(Zeroizing::new(
            explicit_keychain_password(std::env::var("PLEXI_KEYCHAIN_PASSWORD").ok().as_deref())
                .map_err(|error| SecretError::Backend(error.to_string()))?
                .to_string(),
        ))
    }

    fn open_test_file(path: &std::path::Path, password: &str) -> Result<SecKeychain, SecretError> {
        let open_error = |status: i32| {
            log::warn!(
                "workspace_secrets: test keychain {} failed to open: {status}",
                path.display()
            );
            SecretError::Keychain {
                stage: KeychainStage::Open,
                status,
            }
        };
        let mut keychain = if path.is_file() {
            SecKeychain::open(path).map_err(|error| open_error(error.code()))?
        } else {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent).map_err(|error| {
                    SecretError::Backend(format!(
                        "could not create test keychain directory {}: {error}",
                        parent.display()
                    ))
                })?;
            }
            CreateOptions::new()
                .password(password)
                .prompt_user(false)
                .create(path)
                .map_err(|error| open_error(error.code()))?
        };
        keychain
            .unlock(Some(password))
            .map_err(|error| open_error(error.code()))?;
        Ok(keychain)
    }

    impl Native for SecurityFramework {
        type Keychain = SecKeychain;
        type Item = SecKeychainItem;

        fn set_interaction_allowed(&self, allowed: bool) -> i32 {
            let status = unsafe {
                security_framework_sys::keychain::SecKeychainSetUserInteractionAllowed(u8::from(
                    allowed,
                ))
            };
            if allowed || status != 0 {
                log::info!(
                    "workspace_secrets: SecKeychainSetUserInteractionAllowed({allowed}) status={status}"
                );
            }
            status
        }

        /// A test file is created with `SecKeychainCreate` when missing,
        /// then opened and unlocked with its password. None of these calls
        /// change the user's search list or default keychain.
        fn open_domain(&self) -> Result<Domain<SecKeychain>, SecretError> {
            let paths = match choice()? {
                KeychainPathChoice::SearchList => {
                    let mut list: core_foundation::array::CFArrayRef = std::ptr::null();
                    let status = unsafe { SecKeychainCopySearchList(&mut list) };
                    if status != 0 || list.is_null() {
                        return Err(SecretError::Keychain {
                            stage: KeychainStage::Open,
                            status,
                        });
                    }
                    let list: CFArray<SecKeychain> =
                        unsafe { CFArray::wrap_under_create_rule(list) };
                    return Ok(Domain {
                        search: list.iter().map(|keychain| (*keychain).clone()).collect(),
                        enumerable: true,
                    });
                }
                KeychainPathChoice::File(path) => vec![path],
                KeychainPathChoice::Fixture { search, .. } => search,
            };
            let password = test_password()?;
            static LOGGED: std::sync::Once = std::sync::Once::new();
            LOGGED.call_once(|| {
                log::info!(
                    "workspace_secrets: test keychains {paths:?} opened by path; search list and default keychain unchanged"
                );
            });
            Ok(Domain {
                search: paths
                    .iter()
                    .map(|path| open_test_file(path, &password))
                    .collect::<Result<Vec<_>, _>>()?,
                enumerable: false,
            })
        }

        fn open_default(&self) -> Result<SecKeychain, SecretError> {
            let path = match choice()? {
                KeychainPathChoice::SearchList => {
                    return SecKeychain::default().map_err(|error| SecretError::Keychain {
                        stage: KeychainStage::DefaultKeychain,
                        status: error.code(),
                    })
                }
                KeychainPathChoice::File(path) => path,
                KeychainPathChoice::Fixture { default, .. } => default,
            };
            open_test_file(&path, &test_password()?)
        }

        fn list_accounts(&self, service: &str) -> Result<Vec<String>, i32> {
            use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
            let mut search = ItemSearchOptions::new();
            search
                .class(ItemClass::generic_password())
                .service(service)
                .load_attributes(true)
                .limit(Limit::All);
            let results = match search.search() {
                Ok(results) => results,
                Err(error) if error.code() == super::ITEM_NOT_FOUND => return Ok(Vec::new()),
                Err(error) => return Err(error.code()),
            };
            let mut accounts = Vec::new();
            for result in &results {
                match result.simplify_dict().and_then(|d| d.get("acct").cloned()) {
                    Some(account) => accounts.push(account),
                    None => log::warn!(
                        "workspace_secrets::scan: keychain item under service '{service}' has no account attribute; skipping"
                    ),
                }
            }
            Ok(accounts)
        }

        fn find_item(
            &self,
            search: &[SecKeychain],
            service: &str,
            account: &str,
        ) -> (i32, Option<SecKeychainItem>) {
            let (Ok(service_len), Ok(account_len)) =
                (u32::try_from(service.len()), u32::try_from(account.len()))
            else {
                return (-50, None); // errSecParam
            };
            let search = CFArray::from_CFTypes(search);
            let mut item: SecKeychainItemRef = std::ptr::null_mut();
            let status = unsafe {
                security_framework_sys::keychain::SecKeychainFindGenericPassword(
                    search.as_CFTypeRef(),
                    service_len,
                    service.as_ptr().cast(),
                    account_len,
                    account.as_ptr().cast(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut item,
                )
            };
            let item =
                (!item.is_null()).then(|| unsafe { SecKeychainItem::wrap_under_create_rule(item) });
            (status, item)
        }

        fn copy_data(&self, item: &SecKeychainItem) -> Result<Zeroizing<Vec<u8>>, i32> {
            let mut length = 0u32;
            let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
            let status = unsafe {
                SecKeychainItemCopyContent(
                    item.as_concrete_TypeRef(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut length,
                    &mut data,
                )
            };
            if status != 0 {
                log::warn!("workspace_secrets: SecKeychainItemCopyContent status={status}");
                return Err(status);
            }
            if data.is_null() {
                return Ok(Zeroizing::new(Vec::new()));
            }
            let bytes = Zeroizing::new(
                unsafe { std::slice::from_raw_parts(data.cast::<u8>(), length as usize) }.to_vec(),
            );
            let freed = unsafe {
                security_framework_sys::keychain_item::SecKeychainItemFreeContent(
                    std::ptr::null_mut(),
                    data,
                )
            };
            if freed != 0 {
                log::warn!("workspace_secrets: SecKeychainItemFreeContent failed: {freed}");
            }
            Ok(bytes)
        }

        fn delete_item(&self, item: &SecKeychainItem) -> i32 {
            let status = unsafe {
                security_framework_sys::keychain_item::SecKeychainItemDelete(
                    item.as_concrete_TypeRef(),
                )
            };
            log::info!("workspace_secrets: SecKeychainItemDelete status={status}");
            status
        }

        fn modify_data(&self, item: &SecKeychainItem, value: &[u8]) -> i32 {
            let Ok(length) = u32::try_from(value.len()) else {
                return -50; // errSecParam
            };
            unsafe {
                security_framework_sys::keychain_item::SecKeychainItemModifyAttributesAndData(
                    item.as_concrete_TypeRef(),
                    std::ptr::null(),
                    length,
                    value.as_ptr().cast(),
                )
            }
        }

        fn add_item(
            &self,
            keychain: &SecKeychain,
            service: &str,
            account: &str,
            label: &str,
            value: &[u8],
        ) -> i32 {
            use core_foundation::data::CFData;
            use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass, Location};
            match ItemAddOptions::new(ItemAddValue::Data {
                class: ItemClass::generic_password(),
                data: CFData::from_buffer(value),
            })
            .set_service(service)
            .set_account_name(account)
            .set_label(label)
            .set_location(Location::FileKeychain(keychain.clone()))
            .add()
            {
                Ok(()) => 0,
                Err(error) => error.code(),
            }
        }

        fn same_keychain(&self, left: &SecKeychain, right: &SecKeychain) -> bool {
            matches!((keychain_path(left), keychain_path(right)), (Some(a), Some(b)) if a == b)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Interaction(bool),
        Open,
        OpenDefault,
        ListAccounts,
        Find,
        CopyData,
        Delete,
        Modify,
        Add(u8),
    }

    /// Keychains are small ids; items are `(keychain, account, value)`.
    struct Fake {
        calls: RefCell<Vec<Call>>,
        interaction: Cell<bool>,
        suppress_status: i32,
        search: Vec<u8>,
        default: Result<u8, i32>,
        enumerable: bool,
        items: RefCell<Vec<(u8, String, String)>>,
        find_override: Option<(i32, bool)>,
        delete_status: Option<i32>,
    }

    impl Fake {
        fn new(search: &[u8], default: u8) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                interaction: Cell::new(false),
                suppress_status: 0,
                search: search.to_vec(),
                default: Ok(default),
                enumerable: false,
                items: RefCell::new(Vec::new()),
                find_override: None,
                delete_status: None,
            }
        }

        fn holding(self, keychain: u8, account: &str, value: &str) -> Self {
            self.items
                .borrow_mut()
                .push((keychain, account.to_string(), value.to_string()));
            self
        }

        fn calls(&self) -> Vec<Call> {
            self.calls.borrow().clone()
        }

        fn in_keychain(&self, keychain: u8, account: &str) -> Option<String> {
            self.items
                .borrow()
                .iter()
                .find(|(k, a, _)| *k == keychain && a == account)
                .map(|(_, _, v)| v.clone())
        }
    }

    impl Native for Fake {
        type Keychain = u8;
        type Item = (u8, String);

        fn set_interaction_allowed(&self, allowed: bool) -> i32 {
            self.calls.borrow_mut().push(Call::Interaction(allowed));
            if !allowed && self.suppress_status != 0 {
                return self.suppress_status;
            }
            self.interaction.set(allowed);
            0
        }

        fn open_domain(&self) -> Result<Domain<u8>, SecretError> {
            self.calls.borrow_mut().push(Call::Open);
            Ok(Domain {
                search: self.search.clone(),
                enumerable: self.enumerable,
            })
        }

        fn open_default(&self) -> Result<u8, SecretError> {
            self.calls.borrow_mut().push(Call::OpenDefault);
            self.default.map_err(|status| SecretError::Keychain {
                stage: KeychainStage::DefaultKeychain,
                status,
            })
        }

        fn list_accounts(&self, service: &str) -> Result<Vec<String>, i32> {
            assert_eq!(service, KEYCHAIN_SERVICE);
            self.calls.borrow_mut().push(Call::ListAccounts);
            Ok(self
                .items
                .borrow()
                .iter()
                .filter(|(k, _, _)| self.search.contains(k))
                .map(|(_, a, _)| a.clone())
                .collect())
        }

        fn find_item(
            &self,
            search: &[u8],
            service: &str,
            account: &str,
        ) -> (i32, Option<(u8, String)>) {
            assert_eq!(service, KEYCHAIN_SERVICE);
            self.calls.borrow_mut().push(Call::Find);
            if let Some((status, with_item)) = self.find_override {
                return (status, with_item.then(|| (search[0], account.to_string())));
            }
            for keychain in search {
                if self.in_keychain(*keychain, account).is_some() {
                    return (0, Some((*keychain, account.to_string())));
                }
            }
            (ITEM_NOT_FOUND, None)
        }

        fn copy_data(&self, item: &(u8, String)) -> Result<Zeroizing<Vec<u8>>, i32> {
            self.calls.borrow_mut().push(Call::CopyData);
            self.in_keychain(item.0, &item.1)
                .map(|value| Zeroizing::new(value.into_bytes()))
                .ok_or(ITEM_NOT_FOUND)
        }

        fn delete_item(&self, item: &(u8, String)) -> i32 {
            self.calls.borrow_mut().push(Call::Delete);
            if let Some(status) = self.delete_status {
                return status;
            }
            let mut items = self.items.borrow_mut();
            let before = items.len();
            items.retain(|(k, a, _)| !(*k == item.0 && *a == item.1));
            if items.len() == before {
                ITEM_NOT_FOUND
            } else {
                0
            }
        }

        fn modify_data(&self, item: &(u8, String), value: &[u8]) -> i32 {
            self.calls.borrow_mut().push(Call::Modify);
            for entry in self.items.borrow_mut().iter_mut() {
                if entry.0 == item.0 && entry.1 == item.1 {
                    entry.2 = String::from_utf8(value.to_vec()).unwrap();
                    return 0;
                }
            }
            ITEM_NOT_FOUND
        }

        fn add_item(&self, keychain: &u8, _: &str, account: &str, _: &str, value: &[u8]) -> i32 {
            self.calls.borrow_mut().push(Call::Add(*keychain));
            if self.in_keychain(*keychain, account).is_some() {
                return DUPLICATE_ITEM;
            }
            self.items.borrow_mut().push((
                *keychain,
                account.to_string(),
                String::from_utf8(value.to_vec()).unwrap(),
            ));
            0
        }

        fn same_keychain(&self, left: &u8, right: &u8) -> bool {
            left == right
        }
    }

    const ACCOUNT: &str = "plexi:folder:aa:TOKEN";

    fn stage_status(error: &SecretError) -> (KeychainStage, i32) {
        match error {
            SecretError::Keychain { stage, status } => (*stage, *status),
            other => panic!("expected a staged Keychain error, got {other:?}"),
        }
    }

    #[test]
    fn failed_ui_suppression_stops_before_opening_the_keychain() {
        let mut fake = Fake::new(&[1], 1).holding(1, ACCOUNT, "v");
        fake.suppress_status = -25291;
        for result in [
            remove(&fake, ACCOUNT).map(|_| ()),
            read(&fake, ACCOUNT, false).map(|_| ()),
            write(&fake, ACCOUNT, "label", "new", false),
        ] {
            let error = result.unwrap_err();
            assert_eq!(stage_status(&error), (KeychainStage::SuppressUi, -25291));
        }
        assert!(
            fake.calls()
                .iter()
                .all(|call| *call == Call::Interaction(false)),
            "{:?}",
            fake.calls()
        );
        assert_eq!(fake.in_keychain(1, ACCOUNT).as_deref(), Some("v"));
    }

    #[test]
    fn removal_disables_interaction_left_on_by_a_failed_restore() {
        let fake = Fake::new(&[1], 1).holding(1, ACCOUNT, "v");
        fake.interaction.set(true);
        let removal = remove(&fake, ACCOUNT).unwrap();
        assert_eq!(removal.outcome, DeleteOutcome::Deleted);
        let calls = fake.calls();
        assert_eq!(calls[0], Call::Interaction(false), "{calls:?}");
        assert!(!calls.contains(&Call::Interaction(true)), "{calls:?}");
        assert!(
            !calls.contains(&Call::CopyData),
            "removal must not read data"
        );
        assert!(!fake.interaction.get());
    }

    #[test]
    fn removal_deletes_the_selected_item_once_and_reports_no_match() {
        let fake = Fake::new(&[1], 1).holding(1, ACCOUNT, "v");
        let removal = remove(&fake, ACCOUNT).unwrap();
        assert_eq!(removal.outcome, DeleteOutcome::Deleted);
        assert!(matches!(removal.survivor, Some(Ok(false))));
        assert_eq!(fake.in_keychain(1, ACCOUNT), None);
        let calls = fake.calls();
        assert_eq!(calls.iter().filter(|c| **c == Call::Delete).count(), 1);

        let again = remove(&fake, ACCOUNT).unwrap();
        assert_eq!(again.outcome, DeleteOutcome::NoMatchReported);
        assert!(again.survivor.is_none());
        assert_eq!(
            fake.calls().iter().filter(|c| **c == Call::Delete).count(),
            1,
            "a no-match selection must not call delete"
        );
    }

    #[test]
    fn selection_success_without_an_item_reference_is_an_error() {
        let mut fake = Fake::new(&[1], 1);
        fake.find_override = Some((0, false));
        let error = remove(&fake, ACCOUNT).err().expect("must fail");
        assert_eq!(stage_status(&error), (KeychainStage::Select, 0));
        assert!(!fake.calls().contains(&Call::Delete));
    }

    #[test]
    fn selection_errors_keep_their_status_and_never_delete() {
        for status in [-25293, -25308, -25294, -25295, -50] {
            let mut fake = Fake::new(&[1], 1).holding(1, ACCOUNT, "v");
            fake.find_override = Some((status, false));
            let error = remove(&fake, ACCOUNT).err().expect("must fail");
            assert_eq!(stage_status(&error), (KeychainStage::Select, status));
            assert!(!fake.calls().contains(&Call::Delete));
        }
    }

    #[test]
    fn delete_time_statuses_are_errors_at_the_delete_stage() {
        for (status, outcome) in [
            (-25293, "auth_failed"),
            (-25308, "auth_failed"),
            (ITEM_NOT_FOUND, "backend_error"),
            (-25295, "backend_error"),
        ] {
            let mut fake = Fake::new(&[1, 2], 1)
                .holding(1, ACCOUNT, "v")
                .holding(2, ACCOUNT, "w");
            fake.delete_status = Some(status);
            let error = remove(&fake, ACCOUNT).err().expect("must fail");
            assert_eq!(stage_status(&error), (KeychainStage::Delete, status));
            assert_eq!(error.outcome(), outcome);
            let calls = fake.calls();
            assert_eq!(
                calls.iter().filter(|c| **c == Call::Find).count(),
                1,
                "no re-selection by name after a failed delete: {calls:?}"
            );
            assert_eq!(calls.iter().filter(|c| **c == Call::Delete).count(), 1);
        }
        let mut stale = Fake::new(&[1], 1).holding(1, ACCOUNT, "v");
        stale.delete_status = Some(ITEM_NOT_FOUND);
        let message = remove(&stale, ACCOUNT).err().unwrap().to_string();
        assert!(message.contains("could no longer be deleted"), "{message}");
    }

    #[test]
    fn read_and_removal_select_the_same_first_match() {
        let fake = Fake::new(&[2, 1], 1)
            .holding(1, ACCOUNT, "default-copy")
            .holding(2, ACCOUNT, "first-searched");
        assert_eq!(
            read(&fake, ACCOUNT, false).unwrap().unwrap().as_str(),
            "first-searched"
        );
        let removal = remove(&fake, ACCOUNT).unwrap();
        assert_eq!(removal.outcome, DeleteOutcome::Deleted);
        assert!(matches!(removal.survivor, Some(Ok(true))));
        assert_eq!(fake.in_keychain(2, ACCOUNT), None);
        assert_eq!(
            fake.in_keychain(1, ACCOUNT).as_deref(),
            Some("default-copy")
        );
        assert_eq!(
            read(&fake, ACCOUNT, false).unwrap().unwrap().as_str(),
            "default-copy"
        );
    }

    #[test]
    fn a_non_default_searched_item_is_reachable() {
        let fake = Fake::new(&[1, 2], 1).holding(2, ACCOUNT, "elsewhere");
        assert_eq!(
            read(&fake, ACCOUNT, false).unwrap().unwrap().as_str(),
            "elsewhere"
        );
        assert_eq!(
            remove(&fake, ACCOUNT).unwrap().outcome,
            DeleteOutcome::Deleted
        );
        assert_eq!(fake.in_keychain(2, ACCOUNT), None);
    }

    #[test]
    fn an_excluded_default_neither_removes_nor_receives_new_items() {
        let fake = Fake::new(&[2], 1).holding(1, ACCOUNT, "outside-search");
        assert_eq!(
            remove(&fake, ACCOUNT).unwrap().outcome,
            DeleteOutcome::NoMatchReported
        );
        assert_eq!(
            fake.in_keychain(1, ACCOUNT).as_deref(),
            Some("outside-search")
        );
        for create_only in [false, true] {
            let error = write(&fake, "plexi:user:NEW", "label", "v", create_only).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("not in the keychain search list"),
                "{error}"
            );
        }
        assert!(!fake.calls().iter().any(|call| matches!(call, Call::Add(_))));
    }

    #[test]
    fn upsert_updates_the_selected_item_in_place_and_creates_in_the_default() {
        let fake = Fake::new(&[2, 1], 1).holding(2, ACCOUNT, "old");
        write(&fake, ACCOUNT, "label", "new", false).unwrap();
        assert_eq!(fake.in_keychain(2, ACCOUNT).as_deref(), Some("new"));
        assert!(
            !fake.calls().contains(&Call::Delete),
            "update must not delete first"
        );
        write(&fake, "plexi:user:FRESH", "label", "v", false).unwrap();
        assert_eq!(
            fake.in_keychain(1, "plexi:user:FRESH").as_deref(),
            Some("v")
        );
        assert_eq!(
            write(&fake, "plexi:user:FRESH", "label", "v2", true)
                .err()
                .map(|error| error.to_string()),
            Some(SecretError::AlreadyExists("plexi:user:FRESH".into()).to_string())
        );
        let mut missing_default = Fake::new(&[1], 1);
        missing_default.default = Err(-25307);
        let error = write(&missing_default, "plexi:user:X", "l", "v", false).unwrap_err();
        assert_eq!(
            stage_status(&error),
            (KeychainStage::DefaultKeychain, -25307)
        );
    }

    #[test]
    fn guarded_delete_reads_and_deletes_one_selection() {
        let fake = Fake::new(&[1], 1).holding(1, ACCOUNT, "copied");
        assert!(matches!(
            remove_if_value(&fake, ACCOUNT, "other"),
            Err(SecretError::ValueChanged(_))
        ));
        assert_eq!(fake.in_keychain(1, ACCOUNT).as_deref(), Some("copied"));
        let removal = remove_if_value(&fake, ACCOUNT, "copied").unwrap();
        assert_eq!(removal.outcome, DeleteOutcome::Deleted);
        assert_eq!(
            remove_if_value(&fake, ACCOUNT, "copied").unwrap().outcome,
            DeleteOutcome::NoMatchReported
        );
        assert!(!fake.calls().contains(&Call::Interaction(true)));
    }

    #[test]
    fn scan_turns_ui_off_before_enumerating() {
        let mut fake = Fake::new(&[1], 1).holding(1, ACCOUNT, "v");
        fake.enumerable = true;
        fake.interaction.set(true);
        assert_eq!(scan(&fake, &[]).unwrap(), vec![ACCOUNT.to_string()]);
        let calls = fake.calls();
        assert_eq!(calls[0], Call::Interaction(false), "{calls:?}");
        assert!(!calls.contains(&Call::CopyData), "{calls:?}");

        fake.suppress_status = -25291;
        let fresh_calls = fake.calls().len();
        let error = scan(&fake, &[]).unwrap_err();
        assert_eq!(stage_status(&error), (KeychainStage::SuppressUi, -25291));
        assert_eq!(
            &fake.calls()[fresh_calls..],
            &[Call::Interaction(false)],
            "a failed suppression must stop before the scan"
        );
    }

    #[test]
    fn scan_of_an_explicit_file_confirms_indexed_names_by_reference() {
        let fake = Fake::new(&[1], 1).holding(1, ACCOUNT, "v");
        let indexed = [ACCOUNT.to_string(), "plexi:user:ABSENT".to_string()];
        assert_eq!(scan(&fake, &indexed).unwrap(), vec![ACCOUNT.to_string()]);
        let calls = fake.calls();
        assert_eq!(calls.iter().filter(|c| **c == Call::Open).count(), 1);
        assert!(!calls.contains(&Call::ListAccounts), "{calls:?}");
        assert!(!calls.contains(&Call::CopyData), "{calls:?}");
        assert!(!calls.contains(&Call::Interaction(true)), "{calls:?}");
    }

    #[test]
    fn only_a_create_opens_the_default_keychain() {
        let fake = Fake::new(&[1], 1).holding(1, ACCOUNT, "v");
        read(&fake, ACCOUNT, false).unwrap();
        read(&fake, ACCOUNT, true).unwrap();
        remove_if_value(&fake, "plexi:user:ABSENT", "x").unwrap();
        write(&fake, ACCOUNT, "label", "updated", false).unwrap();
        remove(&fake, ACCOUNT).unwrap();
        remove(&fake, ACCOUNT).unwrap();
        scan(&fake, &[ACCOUNT.to_string()]).unwrap();
        assert!(
            !fake.calls().contains(&Call::OpenDefault),
            "{:?}",
            fake.calls()
        );
        write(&fake, ACCOUNT, "label", "created", false).unwrap();
        assert_eq!(
            fake.calls()
                .iter()
                .filter(|c| **c == Call::OpenDefault)
                .count(),
            1
        );
    }

    #[test]
    fn interactive_read_allows_ui_only_for_its_duration() {
        let fake = Fake::new(&[1], 1).holding(1, ACCOUNT, "v");
        read(&fake, ACCOUNT, true).unwrap();
        let calls = fake.calls();
        assert_eq!(calls.first(), Some(&Call::Interaction(false)));
        assert_eq!(calls.last(), Some(&Call::Interaction(false)));
        assert!(calls.contains(&Call::Interaction(true)));
        assert!(!fake.interaction.get());
    }
}
