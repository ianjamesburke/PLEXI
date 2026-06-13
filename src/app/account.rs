//! Marketplace account boundary — sign-up, login, and the locally-stored
//! session that proves who you are (stint `0021`, shared with `0022`).
//!
//! # What an account is for
//!
//! Per `docs/prm/marketplace-hosted.md`, an account is **never** required to
//! install a free app, run an installed app, or browse the public catalog. It
//! is required only to:
//!
//! - **publish** an app,
//! - **buy** a paid app, or
//! - use the **Plexi AI subscription**.
//!
//! So this module is the identity seam those three flows hang off. It is fully
//! stubbed today: [`StubAccountProvider`] fails closed on every network
//! operation, exactly like the payment stub. A real auth backend (the
//! plexiapp.com API, or a hosted IdP) drops into [`account_provider`] with no
//! change at any call site — `signup`/`login` start returning real sessions and
//! everything downstream keeps working.
//!
//! # The session
//!
//! [`AccountSession`] is the on-disk proof of being logged in: an opaque
//! provider-issued `token` plus the account identity. It lives at
//! `<config_dir>/account.toml`, per-channel by construction. [`AccountStore`]
//! is the only thing that reads or writes it. Payment and publishing consume a
//! borrowed `&AccountSession`; they never construct one — only a provider does.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Proof of an authenticated marketplace account, stored locally. The `token`
/// is opaque and provider-issued; the host presents it on authenticated
/// requests (purchase, publish, subscription) but never interprets it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountSession {
    pub schema_version: u32,
    /// Stable account id from the provider.
    pub account_id: String,
    pub email: String,
    /// Opaque bearer token for authenticated requests.
    pub token: String,
    /// Which provider issued this session (e.g. `"stub"`, `"plexi"`).
    pub provider: String,
    /// ISO-8601 UTC login time.
    pub issued_at: String,
}

/// On-disk account session at `<config_dir>/account.toml`. Per-channel because
/// `config_dir()` is per-channel — logging into alpha does not log you into
/// beta.
pub struct AccountStore {
    path: PathBuf,
}

impl AccountStore {
    /// Open the store for the active channel.
    pub fn open() -> Self {
        AccountStore {
            path: crate::config::config_dir().join("account.toml"),
        }
    }

    /// Open a store at an explicit path (tests).
    #[cfg(test)]
    pub fn open_at(path: PathBuf) -> Self {
        AccountStore { path }
    }

    /// The current session, if logged in.
    pub fn current(&self) -> Option<AccountSession> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        match toml::from_str::<AccountSession>(&text) {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn!("account: corrupt session at {}: {e}", self.path.display());
                None
            }
        }
    }

    /// Persist a session (the result of a successful login/signup).
    pub fn save(&self, session: &AccountSession) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create account dir {}: {e}", parent.display()))?;
        }
        let text = toml::to_string_pretty(session)
            .map_err(|e| format!("could not serialize session: {e}"))?;
        std::fs::write(&self.path, text)
            .map_err(|e| format!("could not write session {}: {e}", self.path.display()))?;
        log::info!(
            "account: saved session for {} (provider {})",
            session.email,
            session.provider
        );
        Ok(())
    }

    /// Clear the stored session (logout). Idempotent — succeeds if absent.
    pub fn clear(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {
                log::info!("account: cleared session at {}", self.path.display());
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("could not clear session {}: {e}", self.path.display())),
        }
    }
}

/// Why an account operation failed.
#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    /// No real auth backend is built in. The stub returns this for every
    /// signup/login — the honest, expected failure until a provider is wired
    /// into [`account_provider`].
    #[error(
        "marketplace accounts are not enabled in this build yet — sign-up and login require a \
         configured auth backend. Free apps install without an account. \
         See https://plexiapp.com/docs/marketplace"
    )]
    NotConfigured,
}

/// The identity boundary. A provider turns an email into an [`AccountSession`].
/// This is the single seam a real auth backend implements; nothing else in the
/// codebase references a concrete provider.
///
/// Factory rule (`CLAUDE.md`): no method may panic. The stub returns `Err` for
/// everything it cannot do.
pub trait AccountProvider: Send + Sync {
    /// Provider name for logs and session records.
    fn name(&self) -> &'static str;

    /// Whether this provider can actually authenticate. The stub returns
    /// `false` so callers can give a clear "accounts unavailable" message.
    fn is_configured(&self) -> bool;

    /// Create a new account for `email`, returning a logged-in session.
    fn signup(&self, email: &str) -> Result<AccountSession, AccountError>;

    /// Log in an existing account, returning a session.
    fn login(&self, email: &str) -> Result<AccountSession, AccountError>;
}

/// The fail-closed default provider. Authenticates no one and never panics.
/// Replacing this is the entire job of adding real accounts.
pub struct StubAccountProvider;

impl AccountProvider for StubAccountProvider {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn is_configured(&self) -> bool {
        false
    }

    fn signup(&self, email: &str) -> Result<AccountSession, AccountError> {
        log::warn!("account: signup({email}) attempted with stub provider — not configured");
        Err(AccountError::NotConfigured)
    }

    fn login(&self, email: &str) -> Result<AccountSession, AccountError> {
        log::warn!("account: login({email}) attempted with stub provider — not configured");
        Err(AccountError::NotConfigured)
    }
}

/// Resolve the active account provider from config. Today this only returns the
/// stub; a real provider drops in here keyed on `account_backend` with zero
/// changes at any call site.
///
/// ```ignore
/// match backend.as_deref() {
///     Some("plexi") => Box::new(PlexiAuthProvider::from_config(cfg)),
///     _ => Box::new(StubAccountProvider),
/// }
/// ```
pub fn account_provider() -> Box<dyn AccountProvider> {
    let backend = crate::config::marketplace_account_backend();
    match backend.as_deref() {
        Some("none") | None => Box::new(StubAccountProvider),
        Some(other) => {
            log::warn!(
                "account: account_backend='{other}' configured but no provider is built — \
                 falling back to stub (login/signup fail closed)"
            );
            Box::new(StubAccountProvider)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(email: &str) -> AccountSession {
        AccountSession {
            schema_version: 1,
            account_id: "acct_123".to_string(),
            email: email.to_string(),
            token: "tok_test".to_string(),
            provider: "stub".to_string(),
            issued_at: "2026-06-12T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn store_round_trips_and_clears() {
        let tmp = std::env::temp_dir().join(format!("plexi-acct-{}.toml", uuid::Uuid::new_v4()));
        let store = AccountStore::open_at(tmp.clone());
        assert!(store.current().is_none(), "no session initially");

        let s = session("user@example.com");
        store.save(&s).unwrap();
        assert_eq!(store.current(), Some(s));

        store.clear().unwrap();
        assert!(store.current().is_none(), "cleared");
        // clear is idempotent
        store.clear().unwrap();
    }

    // Factory-rule stub test: every method runs without panicking and auth
    // fails closed.
    #[test]
    fn stub_provider_never_panics_and_fails_closed() {
        let provider = account_provider();
        assert_eq!(provider.name(), "stub");
        assert!(!provider.is_configured());
        assert!(matches!(
            provider.signup("a@b.com"),
            Err(AccountError::NotConfigured)
        ));
        assert!(matches!(
            provider.login("a@b.com"),
            Err(AccountError::NotConfigured)
        ));
    }
}
