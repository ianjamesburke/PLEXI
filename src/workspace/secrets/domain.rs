//! Which credential store a profile may use (stint 1695).
//!
//! The one decision, from the profile's own identity. An account-rooted
//! profile uses the OS credential store. A `--profile-root` elsewhere is a
//! scratch facility and never opens it: the host seal stays in the profile's
//! file store, and user secrets need an explicit test store or are refused.
//! An override the installer strips therefore falls to the profile, never to
//! the login keychain.

use super::store::{DeleteOutcome, NonDestructiveStore, SecretError, SecretStore};
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CredentialDomain {
    /// The OS credential store.
    OsDefault,
    /// A store this process was explicitly pointed at.
    Explicit,
    /// A scratch root with no explicit store: never the OS default.
    ScratchProfile,
}

pub(crate) fn credential_domain(account_rooted: bool, explicit: bool) -> CredentialDomain {
    match (explicit, account_rooted) {
        (true, _) => CredentialDomain::Explicit,
        (false, true) => CredentialDomain::OsDefault,
        (false, false) => CredentialDomain::ScratchProfile,
    }
}

const REFUSAL: &str = "this profile is under a scratch --profile-root, which never opens the OS credential store. Set PLEXI_KEYCHAIN_PATH and PLEXI_KEYCHAIN_PASSWORD to a throwaway keychain file on macOS, or PLEXI_FOLDER_SECRETS_BACKEND=encrypted-file-fallback on Linux";

/// The user-secret store for a scratch root with no explicit store: reads
/// find nothing, and anything that must succeed says why it cannot.
struct RefusedStore;

static REFUSED: RefusedStore = RefusedStore;

fn refused<T>() -> Result<T, SecretError> {
    Err(SecretError::Backend(REFUSAL.to_string()))
}

impl NonDestructiveStore for RefusedStore {
    fn get(&self, _: &str) -> Option<Zeroizing<String>> {
        None
    }
    fn get_interactive(&self, _: &str) -> Result<Option<Zeroizing<String>>, SecretError> {
        refused()
    }
    fn add_new(&self, _: &str, _: &str) -> Result<(), SecretError> {
        refused()
    }
    fn delete_if_value(&self, _: &str, _: &str) -> Result<(), SecretError> {
        refused()
    }
    fn list_with_prefix(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
    fn scan_accounts(&self) -> Result<Vec<String>, SecretError> {
        refused()
    }
}

impl SecretStore for RefusedStore {
    fn set(&self, _: &str, _: &str) -> Result<(), SecretError> {
        refused()
    }
    fn delete(&self, _: &str) -> Result<DeleteOutcome, SecretError> {
        refused()
    }
}

/// `real` unless `domain` forbids opening it.
fn route(domain: CredentialDomain, real: &dyn SecretStore) -> &dyn SecretStore {
    match domain {
        CredentialDomain::ScratchProfile => &REFUSED,
        CredentialDomain::OsDefault | CredentialDomain::Explicit => real,
    }
}

/// Whether this process was pointed at an explicit user-secret store.
#[cfg(not(test))]
fn explicit_user_store() -> bool {
    #[cfg(target_os = "macos")]
    {
        matches!(
            super::parse_keychain_path(
                std::env::var_os(super::store::KEYCHAIN_PATH_ENV).as_deref()
            ),
            Ok(Some(_))
        )
    }
    #[cfg(target_os = "linux")]
    {
        matches!(
            super::folder_store::folder_backend_override(
                std::env::var("PLEXI_FOLDER_SECRETS_BACKEND")
                    .ok()
                    .as_deref()
            ),
            Ok(Some(_))
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

/// `real`, gated by this process's profile.
#[cfg(not(test))]
pub(super) fn gate(real: &'static dyn SecretStore) -> &'static dyn SecretStore {
    let account_rooted = crate::config::profile_target::profile_target().account_rooted();
    route(
        credential_domain(account_rooted, explicit_user_store()),
        real,
    )
}

#[cfg(test)]
mod tests {
    use super::super::store::DeleteOnlyStore;
    use super::*;

    #[test]
    fn the_domain_follows_the_profile_and_any_explicit_store() {
        use CredentialDomain::*;
        for (account_rooted, explicit, domain) in [
            (true, false, OsDefault),
            (true, true, Explicit),
            (false, true, Explicit),
            (false, false, ScratchProfile),
        ] {
            assert_eq!(credential_domain(account_rooted, explicit), domain);
        }
    }

    /// A scratch root with no explicit store makes no call on the real store.
    #[test]
    fn a_scratch_root_never_reaches_the_real_store() {
        let real = DeleteOnlyStore::answering(|| Ok(DeleteOutcome::Deleted));
        let scratch = route(CredentialDomain::ScratchProfile, &real);
        assert!(scratch.get("plexi:user:X").is_none());
        assert!(scratch.list_with_prefix("plexi:").is_empty());
        for error in [
            scratch.set("plexi:user:X", "v").unwrap_err(),
            scratch.add_new("plexi:user:X", "v").unwrap_err(),
            scratch.delete("plexi:user:X").unwrap_err(),
            scratch.scan_accounts().unwrap_err(),
        ] {
            assert!(error.to_string().contains("PLEXI_KEYCHAIN_PATH"), "{error}");
        }
        assert_eq!(real.deletes(), 0);
        for domain in [CredentialDomain::OsDefault, CredentialDomain::Explicit] {
            route(domain, &real).delete("plexi:user:X").unwrap();
        }
        assert_eq!(real.deletes(), 2);
    }
}
