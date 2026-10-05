//! Registered connector OAuth providers.
//!
//! Providers are public PKCE clients: a client id and endpoints, never a
//! client secret. The only registered provider is `stub`, a local test issuer
//! (`scripts/oauth_stub_issuer.py`) that must run on loopback. Adding a real
//! provider means adding an arm to [`resolve_provider`] with its published
//! endpoints and public client id.

use super::ConnectorError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    pub connector: String,
    pub authorize_url: String,
    pub token_url: String,
    pub revoke_url: Option<String>,
    pub client_id: String,
    pub scopes: Vec<String>,
}

/// Resolve the provider for `connector`. `issuer` is the base URL of the
/// stub issuer and is required (and only accepted) for `stub`.
pub fn resolve_provider(connector: &str, issuer: Option<&str>) -> Result<Provider, ConnectorError> {
    match connector {
        "stub" => {
            let issuer = issuer.ok_or_else(|| {
                ConnectorError::InvalidIssuer(
                    "the stub connector needs --issuer <loopback base URL>".to_string(),
                )
            })?;
            let base = loopback_issuer(issuer)?;
            Ok(Provider {
                connector: connector.to_string(),
                authorize_url: format!("{base}/authorize"),
                token_url: format!("{base}/token"),
                revoke_url: Some(format!("{base}/revoke")),
                client_id: "plexi-desktop-stub".to_string(),
                scopes: vec!["stub.read".to_string()],
            })
        }
        other => {
            if issuer.is_some() {
                return Err(ConnectorError::InvalidIssuer(format!(
                    "--issuer is only accepted for the stub connector, not '{other}'"
                )));
            }
            Err(ConnectorError::UnknownConnector(other.to_string()))
        }
    }
}

/// The stub issuer must be on this machine: nothing it is handed (codes,
/// verifiers, tokens) may leave loopback.
fn loopback_issuer(issuer: &str) -> Result<String, ConnectorError> {
    let url = url::Url::parse(issuer)
        .map_err(|e| ConnectorError::InvalidIssuer(format!("{issuer:?}: {e}")))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(ConnectorError::InvalidIssuer(format!(
            "{issuer:?}: scheme must be http or https"
        )));
    }
    let loopback = match url.host() {
        Some(url::Host::Domain(d)) => d == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if !loopback {
        return Err(ConnectorError::InvalidIssuer(format!(
            "{issuer:?}: the stub issuer must be on loopback"
        )));
    }
    Ok(issuer.trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_provider_derives_endpoints_from_a_loopback_issuer() {
        let p = resolve_provider("stub", Some("http://127.0.0.1:8123/")).unwrap();
        assert_eq!(p.authorize_url, "http://127.0.0.1:8123/authorize");
        assert_eq!(p.token_url, "http://127.0.0.1:8123/token");
        assert_eq!(
            p.revoke_url.as_deref(),
            Some("http://127.0.0.1:8123/revoke")
        );
    }

    #[test]
    fn stub_provider_refuses_a_non_loopback_issuer() {
        for issuer in [
            "https://accounts.google.com",
            "http://10.0.0.5:80",
            "file:///tmp",
        ] {
            assert!(
                matches!(
                    resolve_provider("stub", Some(issuer)),
                    Err(ConnectorError::InvalidIssuer(_))
                ),
                "{issuer}"
            );
        }
        assert!(matches!(
            resolve_provider("stub", None),
            Err(ConnectorError::InvalidIssuer(_))
        ));
    }

    #[test]
    fn unregistered_connectors_are_refused() {
        assert_eq!(
            resolve_provider("google", None),
            Err(ConnectorError::UnknownConnector("google".to_string()))
        );
        assert!(matches!(
            resolve_provider("google", Some("http://127.0.0.1:1")),
            Err(ConnectorError::InvalidIssuer(_))
        ));
    }
}
