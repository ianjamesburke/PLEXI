//! Desktop attention delivery. Tests never call an operating-system backend.
use crate::{agent::fleet::AgentEvent, protocol::AgentState};

pub(crate) fn app_focused(ctx: &egui::Context) -> bool {
    #[cfg(target_os = "macos")]
    if !cfg!(test) {
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            let active = objc2_app_kit::NSApplication::sharedApplication(mtm).isActive();
            static OBSERVED: std::sync::Once = std::sync::Once::new();
            OBSERVED
                .call_once(|| log::info!("attention: native app focus observed active={active}"));
            return active;
        }
    }
    ctx.input(|i| i.viewport().focused.unwrap_or(true))
}

pub(crate) fn raise(id: u64, state: &AgentState, event: &AgentEvent, ctx: &egui::Context) {
    if cfg!(test) {
        return;
    }
    #[cfg(target_os = "macos")]
    mac::raise(id, state, event, ctx);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (event, ctx);
        log::info!("attention: desktop notifications unsupported pane={id} state={state:?}");
    }
}

pub(crate) fn take_clicked() -> Vec<u64> {
    #[cfg(target_os = "macos")]
    {
        mac::take_clicked()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

pub(crate) fn activate() {
    #[cfg(target_os = "macos")]
    if !cfg!(test) {
        if let Some(mtm) = objc2::MainThreadMarker::new() {
            #[allow(deprecated)]
            objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use block2::{DynBlock, RcBlock};
    use objc2::{
        define_class, msg_send,
        rc::Retained,
        runtime::{Bool, ProtocolObject},
        ClassType,
    };
    use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
    use objc2_user_notifications::*;
    use std::sync::{Mutex, OnceLock};

    static CLICKS: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    static WAKE: OnceLock<egui::Context> = OnceLock::new();

    define_class!(
        #[unsafe(super(NSObject))]
        struct AttentionDelegate;

        unsafe impl NSObjectProtocol for AttentionDelegate {}
        unsafe impl UNUserNotificationCenterDelegate for AttentionDelegate {
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn received(
                &self,
                _center: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                completion: &DynBlock<dyn Fn()>,
            ) {
                if response.actionIdentifier().to_string()
                    == "com.apple.UNNotificationDismissActionIdentifier"
                {
                    completion.call(());
                    return;
                }
                let identifier = response.notification().request().identifier().to_string();
                if let Some(id) = identifier
                    .strip_prefix("plexi-attention-")
                    .and_then(|s| s.parse().ok())
                {
                    match CLICKS.lock() {
                        Ok(mut queue) => queue.push(id),
                        Err(error) => log::error!("attention: click queue failed: {error}"),
                    }
                    if let Some(ctx) = WAKE.get() {
                        ctx.request_repaint();
                    }
                }
                completion.call(());
            }
        }
    );

    pub(super) fn take_clicked() -> Vec<u64> {
        match CLICKS.lock() {
            Ok(mut queue) => std::mem::take(&mut *queue),
            Err(error) => {
                log::error!("attention: drain clicks failed: {error}");
                Vec::new()
            }
        }
    }

    pub(super) fn raise(id: u64, state: &AgentState, event: &AgentEvent, ctx: &egui::Context) {
        // UNUserNotificationCenter aborts for an unbundled command-line executable.
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            log::warn!(
                "attention: notification unavailable outside an installed app bundle pane={id}"
            );
            return;
        }
        WAKE.get_or_init(|| ctx.clone());
        let center = UNUserNotificationCenter::currentNotificationCenter();
        static REGISTER: std::sync::Once = std::sync::Once::new();
        REGISTER.call_once(|| {
            let delegate: Retained<AttentionDelegate> =
                unsafe { msg_send![AttentionDelegate::class(), new] };
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            // The center holds a weak delegate; retain it for the host lifetime.
            std::mem::forget(delegate);
        });
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&event.title));
        content.setBody(&NSString::from_str(&event.body));
        content.setSound(None);
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&format!("plexi-attention-{id}")),
            &content,
            None,
        );
        let state = state.as_str();
        // Provisional authorization delivers quietly without an OS permission dialog.
        let authorized = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            if !error.is_null() {
                log::error!(
                    "attention: notification authorization failed pane={id}: {}",
                    unsafe { &*error }
                );
                return;
            }
            if !granted.as_bool() {
                log::warn!("attention: notification authorization denied pane={id}");
                return;
            }

            let submitted = RcBlock::new(move |error: *mut NSError| {
                if error.is_null() {
                    log::info!("attention: os notification pane={id} state={state}");
                } else {
                    log::error!(
                        "attention: notification submit failed pane={id}: {}",
                        unsafe { &*error }
                    );
                }
            });
            UNUserNotificationCenter::currentNotificationCenter()
                .addNotificationRequest_withCompletionHandler(&request, Some(&submitted));
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Provisional,
            &authorized,
        );
    }
}
