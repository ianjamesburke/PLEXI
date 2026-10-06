//! Mobile connector sign-in — not yet supported.
//!
//! Names the same [`ConnectorAuthFlow`] operations as the desktop surface so a
//! mobile host can drop in behind one interface. A mobile implementation will
//! open the authorize URL in a system auth session (ASWebAuthenticationSession
//! / Custom Tabs), receive the callback on a claimed redirect URI instead of a
//! loopback listener, and keep the token in the device keystore. Until then
//! every operation returns [`ConnectorError::Unsupported`]; none panics.

use super::{
    CallbackParams, ConnectorAuthFlow, ConnectorError, CredentialRef, Provider, RevokeOutcome,
};
use std::convert::Infallible;
use std::time::Duration;

const SURFACE: &str = "mobile";

pub struct MobileConnectorAuth;

fn unsupported<T>(operation: &'static str) -> Result<T, ConnectorError> {
    log::info!("connectors::mobile: {operation} refused: not yet supported");
    Err(ConnectorError::Unsupported {
        surface: SURFACE,
        operation,
    })
}

impl ConnectorAuthFlow for MobileConnectorAuth {
    /// No mobile sign-in can be pending: `start` never succeeds.
    type Pending = Infallible;

    fn start(&self, _provider: &Provider) -> Result<Infallible, ConnectorError> {
        unsupported("start")
    }

    fn await_callback(
        &self,
        pending: &Infallible,
        _timeout: Duration,
    ) -> Result<CallbackParams, ConnectorError> {
        match *pending {}
    }

    fn complete(
        &self,
        pending: Infallible,
        _callback: CallbackParams,
    ) -> Result<CredentialRef, ConnectorError> {
        match pending {}
    }

    fn status(&self, _connector: &str) -> Result<Option<CredentialRef>, ConnectorError> {
        unsupported("status")
    }

    fn revoke(&self, _connector: &str) -> Result<RevokeOutcome, ConnectorError> {
        unsupported("revoke")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::resolve_provider;

    #[test]
    fn every_mobile_operation_reports_not_yet_supported() {
        let flow = MobileConnectorAuth;
        let provider = resolve_provider("stub", Some("http://127.0.0.1:9")).unwrap();
        let expect = |operation| ConnectorError::Unsupported {
            surface: "mobile",
            operation,
        };
        assert_eq!(flow.start(&provider).unwrap_err(), expect("start"));
        assert_eq!(flow.status("stub").unwrap_err(), expect("status"));
        assert_eq!(flow.revoke("stub").unwrap_err(), expect("revoke"));
    }
}
