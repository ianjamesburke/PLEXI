//! Connector OAuth — authorization-code + PKCE sign-in for third-party
//! connectors, with the resulting token held host-side.
//!
//! Every surface implements [`ConnectorAuthFlow`]: `start` → `await_callback`
//! → `complete`, plus `status` and `revoke`. The desktop surface
//! ([`desktop::DesktopConnectorAuth`]) receives the redirect on a loopback
//! listener and stores the token set in the platform secret store. The mobile
//! surface ([`mobile::MobileConnectorAuth`]) names the same operations and
//! refuses every one of them until a mobile host exists.
//!
//! Callers only ever see a [`CredentialRef`]: the connector id, the store
//! account holding the token, granted scope and expiry. Tokens never leave
//! this module — not to stdout, not to the log, not to model context.

pub mod desktop;
pub mod mobile;
mod provider;

pub use provider::{resolve_provider, Provider};

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Secret-store account holding a connector's token set. Deliberately outside
/// the `plexi:<scope>:<name>` namespace so connector tokens are never listed
/// by `plexi secret`, adopted by reconcile, or injected into a PTY env.
pub fn credential_account(connector: &str) -> String {
    format!("connector:{connector}")
}

/// What a caller may know about a stored connector credential. Never carries
/// a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRef {
    pub connector: String,
    /// Secret-store account the token set lives under.
    pub account: String,
    pub scope: Option<String>,
    /// Access-token expiry, unix seconds.
    pub expires_at: Option<i64>,
    pub has_refresh_token: bool,
}

/// The authorization response delivered to the redirect URI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// Result of a revoke: the local credential is always removed; the remote
/// revocation outcome is reported separately so a failure is never hidden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokeOutcome {
    pub credential: CredentialRef,
    pub remote: RemoteRevoke,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteRevoke {
    Revoked,
    /// The provider has no revocation endpoint.
    NotSupported,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectorError {
    #[error("connector OAuth is not yet supported on {surface} ({operation})")]
    Unsupported {
        surface: &'static str,
        operation: &'static str,
    },
    #[error("no OAuth provider is registered for connector '{0}'")]
    UnknownConnector(String),
    #[error("invalid issuer: {0}")]
    InvalidIssuer(String),
    #[error("authorization denied by issuer: {error}{}", .description.as_deref().map(|d| format!(" ({d})")).unwrap_or_default())]
    Denied {
        error: String,
        description: Option<String>,
    },
    #[error("authorization callback state did not match this sign-in attempt")]
    StateMismatch,
    #[error("timed out after {0:?} waiting for the authorization callback")]
    Timeout(Duration),
    #[error("authorization callback failed: {0}")]
    Callback(String),
    #[error("token exchange failed: {0}")]
    Exchange(String),
    #[error("credential store failed: {0}")]
    Store(String),
    #[error("connector '{0}' is not connected")]
    NotConnected(String),
}

/// The connector sign-in operations every surface implements.
pub trait ConnectorAuthFlow {
    /// Surface-specific state carried from `start` to `complete` (desktop: the
    /// loopback listener and PKCE verifier).
    type Pending;

    /// Begin a sign-in: returns the pending attempt; its authorize URL is what
    /// the user opens.
    fn start(&self, provider: &Provider) -> Result<Self::Pending, ConnectorError>;
    /// Block until the issuer redirects back (or `timeout` elapses).
    fn await_callback(
        &self,
        pending: &Self::Pending,
        timeout: Duration,
    ) -> Result<CallbackParams, ConnectorError>;
    /// Validate the callback, exchange the code, store the token host-side.
    fn complete(
        &self,
        pending: Self::Pending,
        callback: CallbackParams,
    ) -> Result<CredentialRef, ConnectorError>;
    /// The stored credential reference, if connected.
    fn status(&self, connector: &str) -> Result<Option<CredentialRef>, ConnectorError>;
    /// Revoke at the issuer (when supported) and delete the local credential.
    fn revoke(&self, connector: &str) -> Result<RevokeOutcome, ConnectorError>;
}
