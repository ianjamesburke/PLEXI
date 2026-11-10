//! Cloud layer basics.
//!
//! Local use never requires an account. A session ([`crate::app::account`])
//! only links a desktop to cloud features. This module prunes logs and, when
//! configured, local ledger and assistant conversation files.

pub mod retention;
