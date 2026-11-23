//! The native terminate request (stint 1538).
//!
//! Dock Quit, a quit Apple event, and logout, restart or shutdown all reach
//! AppKit's `terminate:`, which asks the application delegate
//! `applicationShouldTerminate:`. winit's delegate does not implement it, so
//! those quits skipped the unsaved-work question Cmd+Q asks and went straight
//! to `applicationWillTerminate:`. Plexi adds the method to winit's delegate
//! class: it answers `NSTerminateLater` and posts the request to the UI
//! mailbox, and `PlexiApp::service_system_quit` runs the Cmd+Q decision on the
//! next logic pass. The quit then either finishes in `app::quit::exit_host`
//! or is cancelled with `replyToApplicationShouldTerminate:NO`.
//!
//! Apple: "To abort the termination sequence, implement this method and
//! return `NSTerminateCancel`"; after `NSTerminateLater` the app must reply
//! "within two minutes ... or the procedure will time out".
//! <https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/Lifecycle.html>
//! <https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/AppArchitecture/Tasks/GracefulAppTermination.html>

use crate::app::ui_mailbox::{MailboxReceiver, UiMailbox, UiWake};
use std::sync::Arc;

/// Why the system asked Plexi to quit: the `kAEQuitReason` ('why?')
/// parameter of the quit Apple event. AERegistry.h lists its values as
/// kAEQuitAll, kAEShutDown, kAERestart and kAEReallyLogOut; a quit with no
/// reason (Dock Quit, a scripted quit) is `Quit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SystemQuitReason {
    Quit,
    QuitAll,
    LogOut,
    Restart,
    ShutDown,
    Other(u32),
}

impl SystemQuitReason {
    pub(crate) fn from_code(code: Option<u32>) -> Self {
        match code {
            None => Self::Quit,
            Some(code) => match &code.to_be_bytes() {
                b"quia" => Self::QuitAll,
                b"rlgo" => Self::LogOut,
                b"rest" => Self::Restart,
                b"shut" => Self::ShutDown,
                _ => Self::Other(code),
            },
        }
    }

    pub(crate) fn label(self) -> String {
        match self {
            Self::Quit => "quit".into(),
            Self::QuitAll => "quit all".into(),
            Self::LogOut => "log out".into(),
            Self::Restart => "restart".into(),
            Self::ShutDown => "shut down".into(),
            Self::Other(code) => {
                format!("other '{}'", String::from_utf8_lossy(&code.to_be_bytes()))
            }
        }
    }
}

/// The UI side of the terminate request: requests arrive on `rx`; `pending`
/// is true while AppKit waits for Plexi's answer.
pub(crate) struct SystemQuitInbox {
    pub(crate) rx: MailboxReceiver<SystemQuitReason>,
    pub(crate) pending: bool,
    /// Test seam: the sender the delegate method uses in production.
    #[cfg(test)]
    pub(crate) tx: UiMailbox<SystemQuitReason>,
}

impl SystemQuitInbox {
    /// Install the delegate method and return the inbox it posts to. Call
    /// once, from `PlexiApp::new` on the main thread, after AppKit launched.
    pub(crate) fn install(wake: Arc<dyn UiWake>) -> Self {
        let (tx, rx) = UiMailbox::channel(wake, "system_quit");
        #[cfg(all(target_os = "macos", not(test)))]
        macos::install(tx);
        #[cfg(all(not(target_os = "macos"), not(test)))]
        drop(tx);
        Self {
            rx,
            pending: false,
            #[cfg(test)]
            tx,
        }
    }
}

#[cfg(test)]
thread_local! {
    // Test seam: every `cancel_terminate` call, instead of messaging NSApp.
    static CANCELS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Test seam: how many terminate requests were cancelled on this thread.
#[cfg(test)]
pub(crate) fn take_cancels() -> usize {
    CANCELS.with(|cancels| cancels.replace(0))
}

/// Answer the outstanding `NSTerminateLater` with NO: the quit, logout,
/// restart or shutdown that asked is cancelled and Plexi keeps running.
pub(crate) fn cancel_terminate() {
    #[cfg(test)]
    CANCELS.with(|cancels| cancels.set(cancels.get() + 1));
    #[cfg(all(target_os = "macos", not(test)))]
    macos::reply(false);
}

#[cfg(all(target_os = "macos", not(test)))]
mod macos {
    use super::SystemQuitReason;
    use crate::app::ui_mailbox::UiMailbox;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::sel;
    use objc2_app_kit::{NSApplication, NSApplicationTerminateReply};
    use objc2_foundation::{MainThreadMarker, NSAppleEventDescriptor, NSAppleEventManager};
    use std::sync::OnceLock;

    static SENDER: OnceLock<UiMailbox<SystemQuitReason>> = OnceLock::new();

    /// `keyAEQuitReason` ('why?'), AERegistry.h.
    const KEY_AE_QUIT_REASON: u32 = u32::from_be_bytes(*b"why?");

    /// The reason parameter of the Apple event AppKit is handling, if any.
    fn current_quit_reason() -> SystemQuitReason {
        let manager = NSAppleEventManager::sharedAppleEventManager();
        // Sent directly: objc2-foundation binds this method only with its
        // objc2-core-services feature, for the AEKeyword (u32) type alias.
        let code = manager.currentAppleEvent().and_then(|event| {
            let reason: Option<Retained<NSAppleEventDescriptor>> =
                unsafe { objc2::msg_send![&*event, paramDescriptorForKeyword: KEY_AE_QUIT_REASON] };
            reason.map(|reason| reason.enumCodeValue())
        });
        SystemQuitReason::from_code(code)
    }

    unsafe extern "C" fn application_should_terminate(
        _this: &AnyObject,
        _cmd: Sel,
        _sender: *mut AnyObject,
    ) -> NSApplicationTerminateReply {
        let reason = current_quit_reason();
        log::info!(
            "quit: system terminate request reason={}; answering later",
            reason.label()
        );
        let Some(sender) = SENDER.get() else {
            log::error!("quit: system terminate request has no inbox; allowing it");
            return NSApplicationTerminateReply::TerminateNow;
        };
        if let Err(error) = sender.send(reason) {
            log::error!("quit: system terminate request not delivered ({error}); allowing it");
            return NSApplicationTerminateReply::TerminateNow;
        }
        NSApplicationTerminateReply::TerminateLater
    }

    pub(super) fn install(sender: UiMailbox<SystemQuitReason>) {
        let Some(mtm) = MainThreadMarker::new() else {
            log::error!("quit: system terminate hook not installed: not on the main thread");
            return;
        };
        if SENDER.set(sender).is_err() {
            log::error!("quit: system terminate hook already installed");
            return;
        }
        let app = NSApplication::sharedApplication(mtm);
        let Some(delegate) = app.delegate() else {
            log::error!("quit: system terminate hook not installed: NSApp has no delegate");
            return;
        };
        let object: &AnyObject = delegate.as_ref();
        let class: &AnyClass = object.class();
        let selector = sel!(applicationShouldTerminate:);
        if class.instance_method(selector).is_some() {
            log::error!(
                "quit: system terminate hook not installed: {} already answers applicationShouldTerminate:",
                class.name().to_string_lossy()
            );
            return;
        }
        type ShouldTerminate =
            unsafe extern "C" fn(&AnyObject, Sel, *mut AnyObject) -> NSApplicationTerminateReply;
        let imp: ShouldTerminate = application_should_terminate;
        // SAFETY: the signature matches `- (NSApplicationTerminateReply)
        // applicationShouldTerminate:(NSApplication *)sender`, encoded "Q@:@"
        // (NSUInteger return on 64-bit).
        let added = unsafe {
            objc2::ffi::class_addMethod(
                class as *const AnyClass as *mut AnyClass,
                selector,
                std::mem::transmute::<ShouldTerminate, Imp>(imp),
                c"Q@:@".as_ptr(),
            )
        };
        if !added.as_bool() {
            log::error!("quit: system terminate hook not installed: class_addMethod failed");
            return;
        }
        // AppKit may read which delegate methods exist when the delegate is
        // set, so set it again now that it has one more.
        app.setDelegate(None);
        app.setDelegate(Some(&delegate));
        log::info!(
            "quit: system terminate hook installed on {}",
            class.name().to_string_lossy()
        );
    }

    pub(super) fn reply(proceed: bool) {
        let Some(mtm) = MainThreadMarker::new() else {
            log::error!("quit: cannot answer the system terminate request off the main thread");
            return;
        };
        NSApplication::sharedApplication(mtm).replyToApplicationShouldTerminate(proceed);
    }
}

#[cfg(test)]
mod tests {
    use super::SystemQuitReason;

    #[test]
    fn quit_reasons_map_from_the_apple_event_codes() {
        let code = |four: &[u8; 4]| Some(u32::from_be_bytes(*four));
        assert_eq!(SystemQuitReason::from_code(None), SystemQuitReason::Quit);
        assert_eq!(
            SystemQuitReason::from_code(code(b"quia")),
            SystemQuitReason::QuitAll
        );
        assert_eq!(
            SystemQuitReason::from_code(code(b"rlgo")),
            SystemQuitReason::LogOut
        );
        assert_eq!(
            SystemQuitReason::from_code(code(b"rest")),
            SystemQuitReason::Restart
        );
        assert_eq!(
            SystemQuitReason::from_code(code(b"shut")),
            SystemQuitReason::ShutDown
        );
        assert_eq!(
            SystemQuitReason::from_code(code(b"zzzz")).label(),
            "other 'zzzz'"
        );
    }
}
