//! macOS Finder Service: "Open in Plexi"
//!
//! Registers Plexi as a macOS Service provider so right-clicking a folder in
//! Finder (Services submenu) offers "Open in Plexi". Selecting it launches or
//! focuses Plexi and opens the folder as a new context.
//!
//! The service is declared in `assets/Info.plist.ext` under the `NSServices`
//! key. macOS invokes `openInPlexi:userData:error:` on our registered provider
//! object, passing an `NSPasteboard` containing the selected file paths.
//!
//! Incoming paths are pushed into a process-wide queue that `PlexiApp::update`
//! drains once per frame on the main thread. We intentionally do NOT mutate
//! application state from the Objective-C callback — all UI work happens in
//! the egui update loop.

use objc2::declare::ClassBuilder;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{sel, ClassType};
use objc2_app_kit::{NSApplication, NSPasteboard};
use objc2_foundation::{MainThreadMarker, NSArray, NSString};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Process-wide queue of folder paths received from the Finder service.
/// Populated from the Objective-C callback thread (always main thread for
/// services) and drained by `PlexiApp::update` each frame.
static PENDING_PATHS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();

fn pending_paths() -> &'static Mutex<Vec<PathBuf>> {
    PENDING_PATHS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Push a path onto the queue. Exposed for the CLI fallback (`plexi <path>`)
/// and for the service callback.
pub fn queue_path(path: PathBuf) {
    match pending_paths().lock() {
        Ok(mut q) => {
            log::info!("finder_service: queued path {}", path.display());
            q.push(path);
        }
        Err(e) => {
            log::error!("finder_service: failed to lock pending_paths queue: {e}");
        }
    }
}

/// Drain and return any paths that have been queued since the last call.
/// Called by `PlexiApp::update` on the main thread each frame.
pub fn drain_pending_paths() -> Vec<PathBuf> {
    match pending_paths().lock() {
        Ok(mut q) => std::mem::take(&mut *q),
        Err(e) => {
            log::error!("finder_service: failed to lock pending_paths queue during drain: {e}");
            Vec::new()
        }
    }
}

/// Register a PlexiServiceHandler NSObject subclass exactly once.
fn register_service_class() -> &'static objc2::runtime::AnyClass {
    static CLASS: OnceLock<&'static objc2::runtime::AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        let superclass = NSObject::class();
        let mut builder = ClassBuilder::new("PlexiServiceHandler", superclass)
            .expect("PlexiServiceHandler class already registered");

        // Objective-C signature:
        //     - (void)openInPlexi:(NSPasteboard *)pboard
        //                userData:(NSString *)userData
        //                   error:(NSString **)error;
        //
        // macOS dispatches to this when the user invokes the
        // "Open in Plexi" service. We read file paths from the pasteboard,
        // queue them for the update loop, and activate the app.
        unsafe extern "C" fn open_in_plexi(
            _this: &AnyObject,
            _cmd: Sel,
            pboard: *mut NSPasteboard,
            _user_data: *mut NSString,
            _error: *mut *mut NSString,
        ) {
            if pboard.is_null() {
                log::warn!("finder_service: openInPlexi received null pasteboard");
                return;
            }

            let pboard_ref: &NSPasteboard = unsafe { &*pboard };
            let paths = unsafe { read_paths_from_pasteboard(pboard_ref) };

            if paths.is_empty() {
                log::warn!("finder_service: openInPlexi received no usable paths");
                return;
            }

            for p in paths {
                queue_path(p);
            }

            // Bring Plexi to the front. The service dispatch path does not
            // activate the app by itself if it was already running in the
            // background.
            if let Some(mtm) = MainThreadMarker::new() {
                let app = NSApplication::sharedApplication(mtm);
                #[allow(deprecated)]
                app.activateIgnoringOtherApps(true);
            }
        }

        unsafe {
            builder.add_method(
                sel!(openInPlexi:userData:error:),
                open_in_plexi
                    as unsafe extern "C" fn(
                        _,
                        _,
                        *mut NSPasteboard,
                        *mut NSString,
                        *mut *mut NSString,
                    ),
            );
        }

        builder.register()
    })
}

/// Read file paths from a service pasteboard.
///
/// Modern AppKit prefers `NSFilenamesPboardType` as a property list (array of
/// NSString paths). Older/newer variants may instead vend `public.file-url`
/// strings. We try both so the service is robust against the system type
/// the caller provides.
unsafe fn read_paths_from_pasteboard(pboard: &NSPasteboard) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();

    // 1) NSFilenamesPboardType -> NSArray<NSString> of absolute paths.
    //    `NSFilenamesPboardType` is an `extern "C" static` of type
    //    `&'static NSPasteboardType` (which is `&'static NSString`), so we
    //    dereference once to get the `&NSPasteboardType` the binding wants.
    let filenames_type: &objc2_app_kit::NSPasteboardType =
        unsafe { objc2_app_kit::NSFilenamesPboardType };
    let plist: Option<Retained<AnyObject>> = pboard.propertyListForType(filenames_type);
    if let Some(obj) = plist {
        // The property list for NSFilenamesPboardType is an NSArray<NSString>.
        let array: &NSArray<NSString> =
            unsafe { &*(Retained::as_ptr(&obj) as *const NSArray<NSString>) };
        let count = array.count();
        for i in 0..count {
            let s = array.objectAtIndex(i);
            let rust_str = s.to_string();
            let pb = PathBuf::from(rust_str);
            if !pb.as_os_str().is_empty() {
                out.push(pb);
            }
        }
        // Keep the retain alive until here.
        let _ = obj;
    }

    // 2) Fall back to public.file-url as a string.
    if out.is_empty() {
        let url_type = NSString::from_str("public.file-url");
        let s: Option<Retained<NSString>> = unsafe { pboard.stringForType(&url_type) };
        if let Some(url_str) = s {
            let rust_str = url_str.to_string();
            if let Some(stripped) = rust_str.strip_prefix("file://") {
                // Strip any trailing newlines or percent encoding.
                let decoded = percent_decode(stripped);
                let pb = PathBuf::from(decoded);
                if !pb.as_os_str().is_empty() {
                    out.push(pb);
                }
            } else {
                let pb = PathBuf::from(rust_str);
                if !pb.as_os_str().is_empty() {
                    out.push(pb);
                }
            }
        }
    }

    out
}

/// Minimal percent-decoder for file:// URLs. Handles the common `%20` etc.
/// without pulling in a new dependency.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

/// Install the Plexi service provider on NSApp. Safe to call multiple times —
/// only the first call does anything.
pub fn register() {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    if REGISTERED.get().is_some() {
        return;
    }

    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("finder_service: register() called off main thread; skipping");
        return;
    };

    let cls = register_service_class();
    let handler: Retained<NSObject> = unsafe { objc2::msg_send_id![cls, new] };

    let app = NSApplication::sharedApplication(mtm);
    unsafe { app.setServicesProvider(Some(&*handler)) };

    // Leak the handler — NSApp holds it weakly, and we want it to live for
    // the entire process lifetime.
    std::mem::forget(handler);

    // Prompt the OS to rescan our Info.plist service registration so the
    // menu item appears without a logout cycle. This is a no-op if the
    // service is already registered.
    unsafe { objc2_app_kit::NSUpdateDynamicServices() };

    let _ = REGISTERED.set(());
    log::info!("finder_service: registered Plexi service provider");
}
