//! `plexi account` — marketplace account CLI (stint `0021`).
//!
//! Login/signup go through `crate::app::account::account_provider()`, which is
//! the fail-closed stub today. Logout and status are fully local (they only
//! touch the on-disk session), so they work now. An account is only ever needed
//! to publish, buy a paid app, or use the AI subscription — never to install a
//! free app.

use crate::app::account::{account_provider, AccountStore};

/// `plexi account status` — show whether you are logged in.
pub fn account_status_cli() -> i32 {
    log::info!("account: status");
    match AccountStore::open().current() {
        Some(s) => {
            println!("Logged in as {} (account {})", s.email, s.account_id);
            println!("  provider: {}", s.provider);
            println!("  since:    {}", s.issued_at);
            0
        }
        None => {
            println!("Not logged in. Free apps install without an account.");
            println!("Run `plexi account login` to publish or buy paid apps.");
            0
        }
    }
}

/// `plexi account login [--email X]` — authenticate and store a session.
pub fn account_login_cli(email: Option<&str>) -> i32 {
    let email = match resolve_email(email) {
        Some(e) => e,
        None => {
            eprintln!("error: an email is required — pass `--email you@example.com`");
            return 1;
        }
    };
    log::info!("account: login email={email}");
    let provider = account_provider();
    log::info!(
        "account: provider '{}' configured={}",
        provider.name(),
        provider.is_configured()
    );
    match provider.login(&email) {
        Ok(session) => persist(&session),
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `plexi account signup [--email X]` — create an account and store a session.
pub fn account_signup_cli(email: Option<&str>) -> i32 {
    let email = match resolve_email(email) {
        Some(e) => e,
        None => {
            eprintln!("error: an email is required — pass `--email you@example.com`");
            return 1;
        }
    };
    log::info!("account: signup email={email}");
    let provider = account_provider();
    log::info!(
        "account: provider '{}' configured={}",
        provider.name(),
        provider.is_configured()
    );
    match provider.signup(&email) {
        Ok(session) => persist(&session),
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `plexi account logout` — clear the local session.
pub fn account_logout_cli() -> i32 {
    log::info!("account: logout");
    let store = AccountStore::open();
    let was = store.current();
    match store.clear() {
        Ok(()) => {
            match was {
                Some(s) => println!("Logged out {}.", s.email),
                None => println!("Already logged out."),
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Use the explicit `--email`, else the config default, else `None`.
fn resolve_email(flag: Option<&str>) -> Option<String> {
    flag.map(str::to_string)
        .or_else(crate::config::marketplace_account_email)
        .filter(|e| !e.is_empty())
}

fn persist(session: &crate::app::account::AccountSession) -> i32 {
    match AccountStore::open().save(session) {
        Ok(()) => {
            println!("Logged in as {}.", session.email);
            0
        }
        Err(e) => {
            eprintln!("error: authenticated but could not save session: {e}");
            1
        }
    }
}
