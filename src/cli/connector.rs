//! `plexi connector` — connector OAuth sign-in, status, and revoke.
//!
//! Output is a [`CredentialRef`] as JSON or a typed error; a token is never
//! printed. See `src/connectors/` for the flow itself.

use crate::cli::args::ConnectorSurface;
use crate::connectors::{
    desktop::DesktopConnectorAuth, mobile::MobileConnectorAuth, resolve_provider,
    ConnectorAuthFlow, ConnectorError, CredentialRef, RemoteRevoke,
};
use std::time::Duration;

fn print_ref(cref: &CredentialRef) -> i32 {
    match serde_json::to_string_pretty(cref) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(e) => {
            eprintln!("error: serialize credential reference: {e}");
            1
        }
    }
}

fn fail(op: &str, connector: &str, e: &ConnectorError) -> i32 {
    log::warn!("connector_cli: {op} failed connector={connector}: {e}");
    eprintln!("error: {e}");
    match e {
        ConnectorError::Timeout(_) => 2,
        _ => 1,
    }
}

fn desktop() -> DesktopConnectorAuth<'static> {
    DesktopConnectorAuth::new(crate::workspace::secrets::system_store())
}

fn login<F: ConnectorAuthFlow>(
    flow: &F,
    connector: &str,
    issuer: Option<&str>,
    timeout: Duration,
    on_started: impl FnOnce(&F::Pending),
) -> Result<CredentialRef, ConnectorError> {
    let provider = resolve_provider(connector, issuer)?;
    let pending = flow.start(&provider)?;
    on_started(&pending);
    let callback = flow.await_callback(&pending, timeout)?;
    flow.complete(pending, callback)
}

pub fn connector_login_cli(
    connector: &str,
    issuer: Option<&str>,
    no_browser: bool,
    timeout_secs: u64,
    surface: ConnectorSurface,
) -> i32 {
    log::info!("connector_cli: login connector={connector} surface={surface:?}");
    let timeout = Duration::from_secs(timeout_secs);
    let result = match surface {
        // A mobile host has no callback/browser implementation yet. Reject
        // before resolving the desktop-only stub provider so this surface
        // consistently reports its typed Unsupported result, regardless of
        // whether a desktop issuer was supplied.
        ConnectorSurface::Mobile => Err(ConnectorError::Unsupported {
            surface: "mobile",
            operation: "login",
        }),
        ConnectorSurface::Desktop => login(&desktop(), connector, issuer, timeout, |pending| {
            let url = pending.authorize_url();
            eprintln!("Sign in to '{connector}' in your browser:\n  {url}");
            if !no_browser {
                if let Err(e) = crate::app::canvas_bindings::open_http_url(url) {
                    log::warn!("connector_cli: browser open failed: {e}");
                    eprintln!("Could not open a browser ({e}); open the URL above manually.");
                }
            }
            eprintln!("Waiting up to {timeout_secs}s for the sign-in to finish…");
        }),
    };
    match result {
        Ok(cref) => print_ref(&cref),
        Err(e) => fail("login", connector, &e),
    }
}

pub fn connector_status_cli(connector: &str, surface: ConnectorSurface) -> i32 {
    log::info!("connector_cli: status connector={connector} surface={surface:?}");
    let result = match surface {
        ConnectorSurface::Mobile => MobileConnectorAuth.status(connector),
        ConnectorSurface::Desktop => desktop().status(connector),
    };
    match result {
        Ok(Some(cref)) => print_ref(&cref),
        Ok(None) => fail(
            "status",
            connector,
            &ConnectorError::NotConnected(connector.to_string()),
        ),
        Err(e) => fail("status", connector, &e),
    }
}

pub fn connector_revoke_cli(connector: &str, surface: ConnectorSurface) -> i32 {
    log::info!("connector_cli: revoke connector={connector} surface={surface:?}");
    let result = match surface {
        ConnectorSurface::Mobile => MobileConnectorAuth.revoke(connector),
        ConnectorSurface::Desktop => desktop().revoke(connector),
    };
    match result {
        Ok(outcome) => {
            let account = &outcome.credential.account;
            match outcome.remote {
                RemoteRevoke::Revoked => {
                    println!("Revoked '{connector}' at the issuer and deleted {account}.");
                    0
                }
                RemoteRevoke::NotSupported => {
                    println!("Deleted {account}; '{connector}' has no revocation endpoint.");
                    0
                }
                RemoteRevoke::Failed(e) => {
                    log::warn!("connector_cli: remote revoke failed connector={connector}: {e}");
                    eprintln!(
                        "error: deleted {account} locally, but the issuer revoke failed: {e}"
                    );
                    1
                }
            }
        }
        Err(e) => fail("revoke", connector, &e),
    }
}
