//! Cloud layer basics.
//!
//! Local use never requires an account. A session ([`crate::app::account`])
//! only links a desktop to relay and cloud features.
//!
//! Relay registry rows and queued-envelope metadata are deleted by the relay
//! process (`PairingRegistry.retain` in `services/relay/relay.py`). This module
//! does not open that database and does not connect the desktop to the phone
//! relay. It prunes logs Plexi controls, and local ledger and assistant
//! conversation files when `[cloud] retain_local_history` is set.
//!
//! `plexi cloud agent` hosts one packaged agent in a local container. That
//! runner calls [`retention::run`] on the tenant volume. It does not mount
//! host secrets and it does not approve tool calls. The hosting rules are
//! `docs/security/cloud-hosting-guardrails.md`.

pub mod retention;
