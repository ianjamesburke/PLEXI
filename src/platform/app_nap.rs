//! macOS App Nap exemption for the host process.
//!
//! An idle, non-frontmost Plexi host is a nappable process: AppKit defers its
//! CFRunLoop timer wakeups and coalesces cross-thread event-loop wakes, so a
//! `request_repaint()` posted by the notify-socket thread can sit undelivered
//! until some unrelated event un-naps the app. The observable symptom is a CLI
//! request against an idle host timing out, then a burst of queued requests all
//! processing at once (stint 0479 installed-host qualification).
//!
//! The host is an IPC server: it must stay wakeable for its whole lifetime, so
//! the activity token is held forever. `UserInitiatedAllowingIdleSystemSleep`
//! exempts the process from App Nap and automatic termination without
//! preventing display or system sleep.

use std::sync::Once;

/// Declare a process-lifetime activity that exempts the host from App Nap.
/// Idempotent; safe to call from any thread.
pub fn disable_app_nap() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};

        let process_info = NSProcessInfo::processInfo();
        let reason = NSString::from_str("plexi host serves pane IPC while idle");
        let token = process_info.beginActivityWithOptions_reason(
            NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
            &reason,
        );
        // The activity spans the process lifetime; the token must never be
        // released, so leaking it is the correct ownership.
        std::mem::forget(token);
        log::info!("app_nap: host exempted from App Nap (UserInitiatedAllowingIdleSystemSleep)");
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn disable_app_nap_is_idempotent() {
        super::disable_app_nap();
        super::disable_app_nap();
    }
}
