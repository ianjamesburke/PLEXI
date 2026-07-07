//! `plexi account` — marketplace account CLI (stints `0021`, `0340`).
//!
//! `login` runs the plexiapp.com device-code flow directly over HTTPS: it asks
//! the service to email a sign-in link, then polls until the user clicks it.
//! This is a plain network client — it never touches the host socket. `logout`
//! revokes the token server-side (best effort) and clears the local session;
//! `status` reads the on-disk session. An account is only ever needed to
//! publish, buy a paid app, or use the AI subscription — never to install a
//! free app.

use crate::app::account::{
    account_provider, device_poll, device_start, revoke_token, AccountError, AccountSession,
    AccountStore, PollOutcome,
};
use std::time::{Duration, Instant};

/// How often `login` polls the service for approval.
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// `plexi account status` — show whether you are logged in.
pub fn account_status_cli() -> i32 {
    log::info!("account: status");
    for line in status_lines(AccountStore::open().current().as_ref()) {
        println!("{line}");
    }
    0
}

/// Human-readable status lines for the current session (or lack of one). Pure so
/// both branches are unit-testable without capturing stdout.
fn status_lines(session: Option<&AccountSession>) -> Vec<String> {
    match session {
        Some(s) => vec![
            format!("Logged in as {} (account {})", s.email, s.account_id),
            format!("  provider: {}", s.provider),
            format!("  since:    {}", s.issued_at),
        ],
        None => vec![
            "Not logged in. Free apps install without an account.".to_string(),
            "Run `plexi account login` to publish or buy paid apps.".to_string(),
        ],
    }
}

/// `plexi account login [--email X]` — run the device-code flow and store the
/// session on success.
pub fn account_login_cli(email: Option<&str>) -> i32 {
    let email = match resolve_email(email) {
        Some(e) => e,
        None => {
            eprintln!("error: an email is required — pass `--email you@example.com`");
            return 1;
        }
    };

    // The account backend must be enabled. The device flow only makes sense
    // against a real accounts service; the stub fails closed here.
    let provider = account_provider();
    if !provider.is_configured() {
        eprintln!("error: {}", AccountError::NotConfigured);
        return 1;
    }
    let base = crate::config::marketplace_account_url();
    log::info!(
        "account: login email={email} via provider '{}' at {base}",
        provider.name()
    );

    let start = match device_start(&base, &email) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    println!("{}", start.message);
    println!("Check your email ({email}) and click the link — waiting...");

    let deadline = Instant::now() + Duration::from_secs(start.expires_in);
    loop {
        if Instant::now() >= deadline {
            eprintln!("error: {}", AccountError::Timeout);
            return 1;
        }
        std::thread::sleep(POLL_INTERVAL);
        match device_poll(&base, &start.device_code, &start.poll_token) {
            Ok(PollOutcome::Pending) => continue,
            Ok(PollOutcome::Approved(session)) => {
                log::info!(
                    "account: logged in {} (provider {})",
                    session.email,
                    session.provider
                );
                return persist(&session);
            }
            Ok(PollOutcome::Gone) => {
                eprintln!("error: {}", AccountError::DeviceExpired);
                return 1;
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    }
}

/// `plexi account logout` — revoke the token server-side (best effort) and clear
/// the local session.
pub fn account_logout_cli() -> i32 {
    log::info!("account: logout");
    let store = AccountStore::open();
    let was = store.current();

    // Best-effort server-side revoke — a network failure must not block clearing
    // the local session.
    if let Some(session) = &was {
        let base = crate::config::marketplace_account_url();
        if let Err(e) = revoke_token(&base, &session.token) {
            log::warn!("account: server-side token revoke failed (clearing locally anyway): {e}");
        }
    }

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

fn persist(session: &AccountSession) -> i32 {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> AccountSession {
        AccountSession {
            schema_version: 1,
            account_id: "acct_9".to_string(),
            email: "a@b.com".to_string(),
            token: "tok".to_string(),
            provider: "plexi".to_string(),
            issued_at: "2026-07-06T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn status_lines_logged_in() {
        let s = session();
        let lines = status_lines(Some(&s));
        assert!(lines[0].contains("a@b.com"));
        assert!(lines[0].contains("acct_9"));
        assert!(lines.iter().any(|l| l.contains("provider: plexi")));
    }

    #[test]
    fn status_lines_logged_out() {
        let lines = status_lines(None);
        assert!(lines[0].contains("Not logged in"));
        assert!(lines.iter().any(|l| l.contains("plexi account login")));
    }
}
