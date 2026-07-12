//! Marketplace account boundary — sign-up, login, and the locally-stored
//! session that proves who you are (stint `0021`, shared with `0022`).
//!
//! # What an account is for
//!
//! Per `docs/marketplace-hosted.md`, an account is **never** required to
//! install a free app, run an installed app, or browse the public catalog. It
//! is required only to:
//!
//! - **publish** an app,
//! - **buy** a paid app, or
//! - use the **Plexi AI subscription**.
//!
//! So this module is the identity seam those three flows hang off. Two
//! providers exist, chosen by `[marketplace].account_backend`:
//! [`StubAccountProvider`] (default) fails closed on every network operation,
//! and [`PlexiAccountProvider`] (`"plexi"`) talks to plexiapp.com. Because
//! plexiapp.com auth is interactive (a browser magic link, or the device-code
//! flow), the actual sign-in happens in the CLI (`plexi account login`) using
//! the [`device_start`] / [`device_poll`] / [`revoke_token`] client here; the
//! provider itself only reports that accounts are available.
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
use std::sync::LazyLock;
use std::time::Duration;

/// Product default for the accounts service. The device-flow client and the
/// [`PlexiAccountProvider`] talk to this host unless `[marketplace].account_url`
/// overrides it. Always the product domain — never a placeholder.
pub const DEFAULT_ACCOUNT_URL: &str = "https://plexiapp.com";

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
            Err(e) => Err(format!(
                "could not clear session {}: {e}",
                self.path.display()
            )),
        }
    }
}

/// Why an account operation failed. Message prefixes before the first `:` are
/// stable machine-readable tags (`account_login_required`, `login_expired`, …)
/// so callers and tests can match on them.
#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    /// Marketplace accounts are not enabled. The stub returns this for every
    /// signup/login, and CLI login refuses before touching the network. Enable
    /// accounts with `[marketplace].account_backend = "plexi"`.
    #[error(
        "account_backend_disabled: marketplace accounts are off in this profile — set \
         [marketplace].account_backend = \"plexi\" to enable them. Free apps install without an \
         account. See https://plexiapp.com/docs/marketplace"
    )]
    NotConfigured,

    /// The accounts service could not be reached (DNS, TLS, connection).
    #[error("account_network_error: could not reach the accounts service: {0}")]
    Network(String),

    /// The sign-in link expired or was already consumed (server returned 410).
    #[error("login_expired: the sign-in link expired or was already used — run `plexi account login` again")]
    DeviceExpired,

    /// No approval arrived before the device code's own expiry window elapsed.
    #[error("login_timeout: no approval received before the link expired — run `plexi account login` again")]
    Timeout,

    /// The accounts service returned an unexpected status or body.
    #[error("account_service_error: {0}")]
    Server(String),
}

/// One `POST /api/auth/device/start` result: the codes the caller polls with
/// plus the service's human-facing message and the device code's lifetime.
pub struct DeviceFlowStart {
    pub device_code: String,
    pub poll_token: String,
    /// Seconds until the device code expires. Bounds the CLI's poll loop.
    pub expires_in: u64,
    /// Human-facing instruction from the service (names the emailed address).
    pub message: String,
}

/// One `POST /api/auth/device/poll` result.
#[derive(Debug)]
pub enum PollOutcome {
    /// The user has not clicked the emailed link yet — keep polling.
    Pending,
    /// The link was approved; the session is ready to persist.
    Approved(Box<AccountSession>),
    /// The device code expired or was already used — the flow is dead.
    Gone,
}

/// Shared blocking HTTP agent for the accounts service, built once and reused
/// across the whole login flow. The device-code poll loop hits this every few
/// seconds for up to the code's lifetime; a fresh agent per call would pay a new
/// TLS handshake each time.
static AGENT: LazyLock<ureq::Agent> =
    LazyLock::new(|| crate::app::http::agent(Duration::from_secs(30)));

/// `POST {base}/api/auth/device/start` — ask the service to email a sign-in link
/// and hand back the codes to poll with. See `website/src/pages/api/auth/device/start.ts`.
///
/// `ureq` is built without its `json` feature, so JSON bodies are serialized and
/// `Content-Type` set by hand — the AI backends do the same. The `ureq::Error`
/// is matched inline (never returned across a function boundary — it is large).
pub fn device_start(base_url: &str, email: &str) -> Result<DeviceFlowStart, AccountError> {
    #[derive(Deserialize)]
    struct Resp {
        device_code: String,
        poll_token: String,
        expires_in: u64,
        message: String,
    }
    let url = crate::app::http::join_url(base_url, "api/auth/device/start");
    let body = serde_json::json!({ "email": email }).to_string();
    match AGENT
        .post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body)
    {
        Ok(resp) => {
            let text = resp
                .into_string()
                .map_err(|e| AccountError::Network(e.to_string()))?;
            let r: Resp = serde_json::from_str(&text)
                .map_err(|e| AccountError::Server(format!("device/start body: {e}")))?;
            Ok(DeviceFlowStart {
                device_code: r.device_code,
                poll_token: r.poll_token,
                expires_in: r.expires_in,
                message: r.message,
            })
        }
        Err(ureq::Error::Status(code, resp)) => Err(AccountError::Server(format!(
            "device/start returned {code}: {}",
            resp.into_string().unwrap_or_default()
        ))),
        Err(ureq::Error::Transport(t)) => Err(AccountError::Network(t.to_string())),
    }
}

/// `POST {base}/api/auth/device/poll` — check whether the emailed link was
/// approved. 202 → pending, 200 → the session, 410 → gone. See
/// `website/src/pages/api/auth/device/poll.ts`.
pub fn device_poll(
    base_url: &str,
    device_code: &str,
    poll_token: &str,
) -> Result<PollOutcome, AccountError> {
    let url = crate::app::http::join_url(base_url, "api/auth/device/poll");
    let body =
        serde_json::json!({ "device_code": device_code, "poll_token": poll_token }).to_string();
    match AGENT
        .post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body)
    {
        Ok(resp) => {
            if resp.status() == 202 {
                return Ok(PollOutcome::Pending);
            }
            // 200: the approved envelope maps field-for-field onto AccountSession.
            let text = resp
                .into_string()
                .map_err(|e| AccountError::Network(e.to_string()))?;
            let session: AccountSession = serde_json::from_str(&text)
                .map_err(|e| AccountError::Server(format!("device/poll body: {e}")))?;
            Ok(PollOutcome::Approved(Box::new(session)))
        }
        Err(ureq::Error::Status(410, _)) => Ok(PollOutcome::Gone),
        Err(ureq::Error::Status(code, resp)) => Err(AccountError::Server(format!(
            "device/poll returned {code}: {}",
            resp.into_string().unwrap_or_default()
        ))),
        Err(ureq::Error::Transport(t)) => Err(AccountError::Network(t.to_string())),
    }
}

/// `POST {base}/api/auth/revoke` with the bearer token — best-effort server-side
/// logout. See `website/src/pages/api/auth/revoke.ts`.
pub fn revoke_token(base_url: &str, token: &str) -> Result<(), AccountError> {
    let url = crate::app::http::join_url(base_url, "api/auth/revoke");
    match AGENT
        .post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
    {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, _)) => {
            Err(AccountError::Server(format!("revoke returned {code}")))
        }
        Err(ureq::Error::Transport(t)) => Err(AccountError::Network(t.to_string())),
    }
}

/// The identity backend seam. Because plexiapp.com auth is interactive (browser
/// magic link, or the CLI device flow), a provider never logs in
/// programmatically — sign-in is the CLI's job via [`device_start`] /
/// [`device_poll`]. The trait's remaining role is to name the active backend and
/// report whether accounts are enabled, so the login command (and future
/// purchase / publish / subscription seams) can gate on it.
///
/// Factory rule (`CLAUDE.md`): no method may panic.
pub trait AccountProvider: Send + Sync {
    /// Provider name for logs and session records (e.g. `"stub"`, `"plexi"`).
    fn name(&self) -> &'static str;

    /// Whether accounts are enabled for this backend. The stub returns `false`
    /// so callers can give a clear "accounts unavailable" message.
    fn is_configured(&self) -> bool;
}

/// The fail-closed default provider: accounts disabled. Selected when
/// `account_backend` is unset or `"none"`.
pub struct StubAccountProvider;

impl AccountProvider for StubAccountProvider {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn is_configured(&self) -> bool {
        false
    }
}

/// The real plexiapp.com account provider, selected by
/// `account_backend = "plexi"`. It only signals that accounts are available
/// (`is_configured` = true); the interactive device-flow sign-in lives in the
/// free [`device_start`] / [`device_poll`] / [`revoke_token`] functions the CLI
/// drives against [`config::marketplace_account_url`].
///
/// [`config::marketplace_account_url`]: crate::config::marketplace_account_url
pub struct PlexiAccountProvider;

impl AccountProvider for PlexiAccountProvider {
    fn name(&self) -> &'static str {
        "plexi"
    }

    fn is_configured(&self) -> bool {
        true
    }
}

/// Resolve the active account provider from config. `"plexi"` selects the real
/// plexiapp.com provider; anything else fails closed with the stub.
pub fn account_provider() -> Box<dyn AccountProvider> {
    select_account_provider(crate::config::marketplace_account_backend().as_deref())
}

/// Pure provider selection, split out so it is testable without a config file.
fn select_account_provider(backend: Option<&str>) -> Box<dyn AccountProvider> {
    match backend {
        Some("plexi") => Box::new(PlexiAccountProvider),
        Some("none") | None => Box::new(StubAccountProvider),
        Some(other) => {
            log::warn!(
                "account: unknown account_backend='{other}' — falling back to stub \
                 (accounts disabled)"
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

    // The default (no backend configured) is the fail-closed stub.
    #[test]
    fn stub_provider_reports_disabled() {
        let provider = account_provider();
        assert_eq!(provider.name(), "stub");
        assert!(!provider.is_configured());
    }

    #[test]
    fn select_account_provider_by_backend() {
        assert_eq!(select_account_provider(Some("plexi")).name(), "plexi");
        assert!(select_account_provider(Some("plexi")).is_configured());
        assert_eq!(select_account_provider(None).name(), "stub");
        assert!(!select_account_provider(None).is_configured());
        assert_eq!(select_account_provider(Some("none")).name(), "stub");
        // Unknown backend fails closed to the stub.
        assert_eq!(select_account_provider(Some("bogus")).name(), "stub");
    }

    #[test]
    fn plexi_provider_reports_enabled() {
        let p = PlexiAccountProvider;
        assert_eq!(p.name(), "plexi");
        assert!(p.is_configured());
    }

    // ── Device-flow client ──────────────────────────────────────────────────
    //
    // Response bodies below are copied verbatim from the website endpoints in
    // this worktree (`website/src/pages/api/auth/device/{start,poll}.ts` and
    // `revoke.ts`) — the accounts service is ours, so these are the contract.

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// Read a full HTTP request (headers + any `Content-Length` body) so we never
    /// respond while the client is still writing its body — doing so half-closes
    /// the socket and ureq's body write then fails.
    fn drain_request(stream: &mut std::net::TcpStream) {
        let mut buf = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&buf[..end]).to_lowercase();
                        let content_len = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if buf.len() - (end + 4) >= content_len {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    }

    /// A throwaway HTTP server that replies to N sequential requests with the
    /// scripted `(status, json_body)` pairs, in order. Each `ureq` call opens a
    /// fresh connection (`connection: close`), so one script entry == one call.
    fn scripted_server(responses: Vec<(u16, String)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(_) => return,
                };
                drain_request(&mut stream);
                let reason = match status {
                    200 => "OK",
                    202 => "Accepted",
                    410 => "Gone",
                    500 => "Internal Server Error",
                    _ => "Status",
                };
                let headers = format!(
                    "HTTP/1.1 {status} {reason}\r\ncontent-length: {}\r\n\
                     content-type: application/json\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(body.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn device_start_parses_codes() {
        let base = scripted_server(vec![(
            200,
            r#"{"device_code":"dev_abc","poll_token":"pt_xyz","expires_in":900,"message":"We emailed an approval link to a@b.com. Click it to sign in on this device."}"#.to_string(),
        )]);
        let start = device_start(&base, "a@b.com").expect("device/start should parse");
        assert_eq!(start.device_code, "dev_abc");
        assert_eq!(start.poll_token, "pt_xyz");
        assert_eq!(start.expires_in, 900);
        assert!(start.message.contains("a@b.com"));
    }

    #[test]
    fn device_poll_pending_returns_pending() {
        let base = scripted_server(vec![(202, r#"{"status":"pending"}"#.to_string())]);
        assert!(matches!(
            device_poll(&base, "dev_abc", "pt_xyz"),
            Ok(PollOutcome::Pending)
        ));
    }

    #[test]
    fn device_poll_approved_yields_saveable_session() {
        let base = scripted_server(vec![(
            200,
            r#"{"schema_version":1,"token":"tok_live","account_id":"acct_9","email":"a@b.com","provider":"plexi","issued_at":"2026-07-06T00:00:00Z"}"#.to_string(),
        )]);
        let outcome = device_poll(&base, "dev_abc", "pt_xyz").expect("poll should succeed");
        let session = match outcome {
            PollOutcome::Approved(s) => *s,
            other => panic!("expected approved, got {other:?}"),
        };
        assert_eq!(session.account_id, "acct_9");
        assert_eq!(session.token, "tok_live");
        assert_eq!(session.provider, "plexi");

        // The approved session round-trips through the on-disk store.
        let tmp = std::env::temp_dir().join(format!("plexi-acct-{}.toml", uuid::Uuid::new_v4()));
        let store = AccountStore::open_at(tmp.clone());
        store.save(&session).unwrap();
        assert_eq!(store.current(), Some(session));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn device_poll_gone_returns_gone() {
        let base = scripted_server(vec![(
            410,
            r#"{"error":"device code expired or already used"}"#.to_string(),
        )]);
        assert!(matches!(
            device_poll(&base, "dev_abc", "pt_xyz"),
            Ok(PollOutcome::Gone)
        ));
    }

    #[test]
    fn revoke_token_ok_and_error() {
        let ok_base = scripted_server(vec![(200, r#"{"ok":true}"#.to_string())]);
        assert!(revoke_token(&ok_base, "tok_live").is_ok());

        let err_base = scripted_server(vec![(500, r#"{"error":"server error"}"#.to_string())]);
        assert!(matches!(
            revoke_token(&err_base, "tok_live"),
            Err(AccountError::Server(_))
        ));
    }
}
