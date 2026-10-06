//! Desktop connector sign-in: RFC 8252 loopback redirect + RFC 7636 PKCE.
//!
//! `start` binds an ephemeral `127.0.0.1` listener whose address is the
//! redirect URI, so only a process on this machine can deliver the callback,
//! and the PKCE verifier means an intercepted code is useless without it.
//! The token set is written to the injected [`SecretStore`] (production:
//! `system_store()`) under [`credential_account`]; only a [`CredentialRef`]
//! is returned.

use super::{
    credential_account, CallbackParams, ConnectorAuthFlow, ConnectorError, CredentialRef, Provider,
    RemoteRevoke, RevokeOutcome,
};
use crate::workspace::secrets::SecretStore;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

const CALLBACK_PATH: &str = "/callback";
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

pub struct DesktopConnectorAuth<'a> {
    store: &'a dyn SecretStore,
}

impl<'a> DesktopConnectorAuth<'a> {
    pub fn new(store: &'a dyn SecretStore) -> Self {
        Self { store }
    }

    fn load(&self, connector: &str) -> Result<Option<StoredCredential>, ConnectorError> {
        let account = credential_account(connector);
        let Some(raw) = self.store.get(&account) else {
            return Ok(None);
        };
        serde_json::from_str(&raw).map(Some).map_err(|e| {
            ConnectorError::Store(format!("{account} holds an unreadable credential: {e}"))
        })
    }
}

/// An in-flight desktop sign-in.
pub struct DesktopPending {
    provider: Provider,
    listener: TcpListener,
    redirect_uri: String,
    state: String,
    verifier: String,
    authorize_url: String,
}

impl DesktopPending {
    /// The URL the user opens in a browser. Carries the PKCE challenge, never
    /// the verifier.
    pub fn authorize_url(&self) -> &str {
        &self.authorize_url
    }
}

/// The persisted token set. Not `Debug`/`Display`: it must never be printed.
#[derive(Serialize, Deserialize)]
struct StoredCredential {
    connector: String,
    client_id: String,
    revoke_url: Option<String>,
    access_token: String,
    token_type: String,
    refresh_token: Option<String>,
    scope: Option<String>,
    expires_at: Option<i64>,
}

impl StoredCredential {
    fn reference(&self) -> CredentialRef {
        CredentialRef {
            connector: self.connector.clone(),
            account: credential_account(&self.connector),
            scope: self.scope.clone(),
            expires_at: self.expires_at,
            has_refresh_token: self.refresh_token.is_some(),
        }
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    scope: Option<String>,
}

fn random_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn pkce_challenge(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

impl ConnectorAuthFlow for DesktopConnectorAuth<'_> {
    type Pending = DesktopPending;

    fn start(&self, provider: &Provider) -> Result<DesktopPending, ConnectorError> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| ConnectorError::Callback(format!("bind loopback listener: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| ConnectorError::Callback(format!("read listener address: {e}")))?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
        let state = uuid::Uuid::new_v4().simple().to_string();
        let verifier = random_token();
        let mut url = url::Url::parse(&provider.authorize_url).map_err(|e| {
            ConnectorError::InvalidIssuer(format!("{}: {e}", provider.authorize_url))
        })?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &provider.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", &provider.scopes.join(" "))
            .append_pair("state", &state)
            .append_pair("code_challenge", &pkce_challenge(&verifier))
            .append_pair("code_challenge_method", "S256");
        log::info!(
            "connectors::desktop: start connector={} redirect_port={port}",
            provider.connector
        );
        Ok(DesktopPending {
            provider: provider.clone(),
            listener,
            redirect_uri,
            state,
            verifier,
            authorize_url: url.into(),
        })
    }

    fn await_callback(
        &self,
        pending: &DesktopPending,
        timeout: Duration,
    ) -> Result<CallbackParams, ConnectorError> {
        pending
            .listener
            .set_nonblocking(true)
            .map_err(|e| ConnectorError::Callback(format!("listener nonblocking: {e}")))?;
        let deadline = Instant::now() + timeout;
        loop {
            match pending.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(params) = serve_callback_request(stream)? {
                        log::info!(
                            "connectors::desktop: callback received connector={} outcome={}",
                            pending.provider.connector,
                            if params.error.is_some() {
                                "error"
                            } else {
                                "code"
                            }
                        );
                        return Ok(params);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        log::warn!(
                            "connectors::desktop: callback timeout connector={}",
                            pending.provider.connector
                        );
                        return Err(ConnectorError::Timeout(timeout));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(ConnectorError::Callback(format!("accept: {e}"))),
            }
        }
    }

    fn complete(
        &self,
        pending: DesktopPending,
        callback: CallbackParams,
    ) -> Result<CredentialRef, ConnectorError> {
        let connector = pending.provider.connector.clone();
        if callback.state.as_deref() != Some(pending.state.as_str()) {
            log::warn!("connectors::desktop: state mismatch connector={connector}");
            return Err(ConnectorError::StateMismatch);
        }
        if let Some(error) = callback.error {
            log::info!("connectors::desktop: denied connector={connector} error={error}");
            return Err(ConnectorError::Denied {
                error,
                description: callback.error_description,
            });
        }
        let code = callback.code.ok_or_else(|| {
            ConnectorError::Callback("callback carried neither code nor error".into())
        })?;
        let response = ureq::post(&pending.provider.token_url)
            .timeout(HTTP_TIMEOUT)
            .send_form(&[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", &pending.redirect_uri),
                ("client_id", &pending.provider.client_id),
                ("code_verifier", &pending.verifier),
            ]);
        let token: TokenResponse = match response {
            Ok(r) => r
                .into_json()
                .map_err(|e| ConnectorError::Exchange(format!("unreadable token response: {e}")))?,
            Err(ureq::Error::Status(status, r)) => {
                let body = r.into_string().unwrap_or_default();
                log::warn!("connectors::desktop: token exchange rejected connector={connector} status={status}");
                return Err(ConnectorError::Exchange(format!(
                    "issuer returned {status}: {body}"
                )));
            }
            Err(e) => {
                log::warn!("connectors::desktop: token exchange transport error connector={connector}: {e}");
                return Err(ConnectorError::Exchange(e.to_string()));
            }
        };
        let stored = StoredCredential {
            connector: connector.clone(),
            client_id: pending.provider.client_id.clone(),
            revoke_url: pending.provider.revoke_url.clone(),
            access_token: token.access_token,
            token_type: token.token_type,
            refresh_token: token.refresh_token,
            scope: token
                .scope
                .or_else(|| Some(pending.provider.scopes.join(" "))),
            expires_at: token.expires_in.map(|s| chrono::Utc::now().timestamp() + s),
        };
        let account = credential_account(&connector);
        let blob = serde_json::to_string(&stored)
            .map_err(|e| ConnectorError::Store(format!("serialize credential: {e}")))?;
        self.store
            .set(&account, &blob)
            .map_err(|e| ConnectorError::Store(e.to_string()))?;
        log::info!("connectors::desktop: connected connector={connector} account={account}");
        Ok(stored.reference())
    }

    fn status(&self, connector: &str) -> Result<Option<CredentialRef>, ConnectorError> {
        Ok(self.load(connector)?.map(|c| c.reference()))
    }

    fn revoke(&self, connector: &str) -> Result<RevokeOutcome, ConnectorError> {
        let stored = self
            .load(connector)?
            .ok_or_else(|| ConnectorError::NotConnected(connector.to_string()))?;
        let remote = match &stored.revoke_url {
            None => RemoteRevoke::NotSupported,
            Some(url) => {
                let (token, hint) = match &stored.refresh_token {
                    Some(refresh) => (refresh.as_str(), "refresh_token"),
                    None => (stored.access_token.as_str(), "access_token"),
                };
                match ureq::post(url).timeout(HTTP_TIMEOUT).send_form(&[
                    ("token", token),
                    ("token_type_hint", hint),
                    ("client_id", &stored.client_id),
                ]) {
                    Ok(_) => RemoteRevoke::Revoked,
                    Err(e) => RemoteRevoke::Failed(e.to_string()),
                }
            }
        };
        let account = credential_account(connector);
        self.store
            .delete(&account)
            .map_err(|e| ConnectorError::Store(e.to_string()))?;
        log::info!("connectors::desktop: revoked connector={connector} account={account} remote={remote:?}");
        Ok(RevokeOutcome {
            credential: stored.reference(),
            remote,
        })
    }
}

/// Read one HTTP request from the loopback listener. Returns the parsed
/// callback for `GET /callback`, or `None` (after answering 404) for anything
/// else a browser sends first, e.g. `/favicon.ico`.
fn serve_callback_request(mut stream: TcpStream) -> Result<Option<CallbackParams>, ConnectorError> {
    let io = |e: std::io::Error| ConnectorError::Callback(format!("read callback request: {e}"));
    stream.set_nonblocking(false).map_err(io)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(io)?;
    let mut reader = BufReader::new(stream.try_clone().map_err(io)?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).map_err(io)?;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).map_err(io)? == 0 || header.trim().is_empty() {
            break;
        }
    }
    let target = request_line.split_whitespace().nth(1).unwrap_or("/");
    let url = url::Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|e| ConnectorError::Callback(format!("bad request target: {e}")))?;
    if url.path() != CALLBACK_PATH {
        respond(&mut stream, "404 Not Found", "Not found.");
        return Ok(None);
    }
    let mut params = CallbackParams::default();
    for (k, v) in url.query_pairs() {
        let v = Some(v.into_owned());
        match k.as_ref() {
            "code" => params.code = v,
            "state" => params.state = v,
            "error" => params.error = v,
            "error_description" => params.error_description = v,
            _ => {}
        }
    }
    let body = if params.error.is_some() {
        "Sign-in was not completed. You can close this window and return to Plexi."
    } else {
        "Sign-in received. You can close this window and return to Plexi."
    };
    respond(&mut stream, "200 OK", body);
    Ok(Some(params))
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let html = format!("<!doctype html><title>Plexi</title><p>{body}</p>");
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    if let Err(e) = stream.write_all(reply.as_bytes()) {
        log::warn!("connectors::desktop: writing callback response failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::resolve_provider;
    use crate::workspace::secrets::{InMemoryKeychain, NonDestructiveStore};
    use std::sync::{Arc, Mutex};

    /// Minimal in-process issuer: answers `/token` with a fixed token set
    /// (after checking the PKCE verifier) and records `/revoke` calls.
    struct TestIssuer {
        base: String,
        revoked: Arc<Mutex<Vec<String>>>,
    }

    fn form(body: &str) -> std::collections::HashMap<String, String> {
        url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect()
    }

    fn spawn_issuer(expected_challenge: Arc<Mutex<Option<String>>>) -> TestIssuer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let revoked = Arc::new(Mutex::new(Vec::new()));
        let revoked_srv = revoked.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut len = 0usize;
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; len];
                std::io::Read::read_exact(&mut reader, &mut body).unwrap();
                let body = form(&String::from_utf8(body).unwrap());
                let (status, json) = if line.contains("/token") {
                    let challenge = expected_challenge.lock().unwrap().clone();
                    if challenge.as_deref() == Some(&pkce_challenge(&body["code_verifier"]))
                        && body["code"] == "test-code"
                    {
                        (
                            "200 OK",
                            r#"{"access_token":"at-secret","token_type":"Bearer","expires_in":3600,"refresh_token":"rt-secret","scope":"stub.read"}"#,
                        )
                    } else {
                        ("400 Bad Request", r#"{"error":"invalid_grant"}"#)
                    }
                } else {
                    revoked_srv.lock().unwrap().push(body["token"].clone());
                    ("200 OK", "{}")
                };
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
                        json.len()
                    )
                    .as_bytes(),
                );
            }
        });
        TestIssuer { base, revoked }
    }

    fn query(url: &str, key: &str) -> String {
        url::Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == key)
            .unwrap()
            .1
            .into_owned()
    }

    /// Play the browser: follow the issuer's redirect to the loopback URI.
    fn deliver(pending: &DesktopPending, query_string: &str) -> std::thread::JoinHandle<String> {
        let url = format!("{}?{query_string}", pending.redirect_uri);
        std::thread::spawn(move || ureq::get(&url).call().unwrap().into_string().unwrap())
    }

    #[test]
    fn approve_stores_a_credential_reference_and_revoke_deletes_it() {
        let challenge = Arc::new(Mutex::new(None));
        let issuer = spawn_issuer(challenge.clone());
        let store = InMemoryKeychain::new();
        let flow = DesktopConnectorAuth::new(&store);
        let provider = resolve_provider("stub", Some(&issuer.base)).unwrap();

        let pending = flow.start(&provider).unwrap();
        let auth_url = pending.authorize_url().to_string();
        assert_eq!(query(&auth_url, "code_challenge_method"), "S256");
        assert!(
            !auth_url.contains(&pending.verifier),
            "verifier must not be in the URL"
        );
        *challenge.lock().unwrap() = Some(query(&auth_url, "code_challenge"));

        let state = query(&auth_url, "state");
        let browser = deliver(&pending, &format!("code=test-code&state={state}"));
        let callback = flow
            .await_callback(&pending, Duration::from_secs(5))
            .unwrap();
        assert!(browser.join().unwrap().contains("Sign-in received"));

        let cref = flow.complete(pending, callback).unwrap();
        assert_eq!(cref.account, "connector:stub");
        assert_eq!(cref.scope.as_deref(), Some("stub.read"));
        assert!(cref.has_refresh_token);
        let printed = serde_json::to_string(&cref).unwrap();
        assert!(!printed.contains("at-secret") && !printed.contains("rt-secret"));
        assert!(store.get("connector:stub").unwrap().contains("at-secret"));
        assert!(
            store.list_with_prefix("plexi:").is_empty(),
            "connector tokens must stay out of the env-injectable namespace"
        );
        assert_eq!(flow.status("stub").unwrap(), Some(cref.clone()));

        let outcome = flow.revoke("stub").unwrap();
        assert_eq!(outcome.remote, RemoteRevoke::Revoked);
        assert_eq!(issuer.revoked.lock().unwrap().as_slice(), ["rt-secret"]);
        assert!(store.get("connector:stub").is_none());
        assert_eq!(flow.status("stub").unwrap(), None);
        assert_eq!(
            flow.revoke("stub").unwrap_err(),
            ConnectorError::NotConnected("stub".to_string())
        );
    }

    #[test]
    fn deny_stores_nothing() {
        let issuer = spawn_issuer(Arc::new(Mutex::new(None)));
        let store = InMemoryKeychain::new();
        let flow = DesktopConnectorAuth::new(&store);
        let pending = flow
            .start(&resolve_provider("stub", Some(&issuer.base)).unwrap())
            .unwrap();
        let state = query(pending.authorize_url(), "state");
        let browser = deliver(
            &pending,
            &format!("error=access_denied&error_description=user+said+no&state={state}"),
        );
        let callback = flow
            .await_callback(&pending, Duration::from_secs(5))
            .unwrap();
        assert!(browser.join().unwrap().contains("not completed"));
        assert_eq!(
            flow.complete(pending, callback).unwrap_err(),
            ConnectorError::Denied {
                error: "access_denied".to_string(),
                description: Some("user said no".to_string()),
            }
        );
        assert!(store.get("connector:stub").is_none());
    }

    #[test]
    fn forged_state_is_refused_before_any_exchange() {
        let store = InMemoryKeychain::new();
        let flow = DesktopConnectorAuth::new(&store);
        let pending = flow
            .start(&resolve_provider("stub", Some("http://127.0.0.1:9")).unwrap())
            .unwrap();
        let browser = deliver(&pending, "code=test-code&state=forged");
        let callback = flow
            .await_callback(&pending, Duration::from_secs(5))
            .unwrap();
        browser.join().unwrap();
        assert_eq!(
            flow.complete(pending, callback).unwrap_err(),
            ConnectorError::StateMismatch
        );
        assert!(store.get("connector:stub").is_none());
    }

    #[test]
    fn stray_requests_are_answered_404_and_the_wait_times_out() {
        let store = InMemoryKeychain::new();
        let flow = DesktopConnectorAuth::new(&store);
        let pending = flow
            .start(&resolve_provider("stub", Some("http://127.0.0.1:9")).unwrap())
            .unwrap();
        let favicon = pending.redirect_uri.replace(CALLBACK_PATH, "/favicon.ico");
        let probe = std::thread::spawn(move || ureq::get(&favicon).call().unwrap_err());
        assert_eq!(
            flow.await_callback(&pending, Duration::from_millis(400))
                .unwrap_err(),
            ConnectorError::Timeout(Duration::from_millis(400))
        );
        assert!(matches!(probe.join().unwrap(), ureq::Error::Status(404, _)));
    }
}
