//! macOS Finder "Open in Plexi" service provider.
//!
//! Registers an NSServicesProvider so the right-click → Services menu in
//! Finder shows "Open in Plexi". Paths received from the pasteboard are
//! queued and drained each frame by `PlexiApp::update()`.

use objc2::declare::ClassBuilder;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{sel, ClassType};
use objc2_app_kit::NSApplication;
use objc2_foundation::{MainThreadMarker, NSString};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static PATH_QUEUE: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// egui context used to wake the UI thread after queueing paths. A fully
/// idle Plexi produces zero frames and `PATH_QUEUE` is only drained during
/// a frame, so without a repaint request a Finder "Open in Plexi" against
/// an idle instance sits queued until the user touches the window.
static EGUI_CTX: OnceLock<egui::Context> = OnceLock::new();

fn register_provider_class() -> &'static objc2::runtime::AnyClass {
    static CLASS: OnceLock<&'static objc2::runtime::AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = NSObject::class();
        let mut builder = ClassBuilder::new(c"PlexiServicesProvider", superclass)
            .expect("class already registered");

        unsafe extern "C" fn open_in_plexi(
            _this: &AnyObject,
            _cmd: Sel,
            pboard: *mut AnyObject,
            _user_data: *mut AnyObject,
            _error: *mut *mut AnyObject,
        ) {
            if pboard.is_null() {
                log::warn!("finder_service: pasteboard is null");
                return;
            }
            let type_str = NSString::from_str("NSFilenamesPboardType");
            let plist: *const AnyObject =
                objc2::msg_send![&*pboard, propertyListForType: &*type_str];
            if plist.is_null() {
                log::warn!("finder_service: no NSFilenamesPboardType on pasteboard");
                return;
            }
            let count: usize = objc2::msg_send![&*plist, count];
            let mut paths = Vec::with_capacity(count);
            for i in 0..count {
                let item: *const AnyObject = objc2::msg_send![&*plist, objectAtIndex: i];
                if item.is_null() {
                    continue;
                }
                let utf8: *const std::ffi::c_char = objc2::msg_send![&*item, UTF8String];
                if utf8.is_null() {
                    continue;
                }
                let path_str = std::ffi::CStr::from_ptr(utf8).to_string_lossy();
                let path = PathBuf::from(path_str.as_ref());
                log::info!("finder_service: received path: {}", path.display());
                paths.push(path);
            }
            if let Ok(mut queue) = PATH_QUEUE.lock() {
                queue.extend(paths);
            }
            if let Some(ctx) = EGUI_CTX.get() {
                log::debug!(
                    "finder_service: paths queued — requesting repaint to wake idle UI thread"
                );
                ctx.request_repaint();
            }
        }

        unsafe {
            builder.add_method(
                sel!(openInPlexi:userData:error:),
                open_in_plexi as unsafe extern "C" fn(_, _, _, _, _),
            );
        }

        builder.register()
    })
}

/// Register the Finder services provider with NSApp. Call once from
/// `PlexiApp::new()` on the main thread. `egui_ctx` is used to wake the
/// (possibly zero-frame idle) UI thread whenever the service queues paths.
pub fn register(egui_ctx: egui::Context) {
    let _ = EGUI_CTX.set(egui_ctx);
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("finder_service: not on main thread, cannot register");
        return;
    };
    let cls = register_provider_class();
    let obj: Retained<NSObject> = unsafe { objc2::msg_send![cls, new] };
    let app = NSApplication::sharedApplication(mtm);
    let ptr = Retained::as_ptr(&obj) as *const AnyObject;
    unsafe {
        let _: () = objc2::msg_send![&*app, setServicesProvider: ptr];
    }
    std::mem::forget(obj);
    log::info!("finder_service: registered NSServicesProvider");
}

/// Drain all queued paths received from the Finder service. Returns an empty
/// vec when nothing is pending.
pub fn drain() -> Vec<PathBuf> {
    match PATH_QUEUE.lock() {
        Ok(mut queue) => std::mem::take(&mut *queue),
        Err(_) => Vec::new(),
    }
}
