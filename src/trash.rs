//! macOS trash helper.
//!
//! Uses `NSFileManager` via `objc2` to move files to the system Trash, and to
//! restore them from Trash (for undo). All API entry points are safe; the
//! `unsafe` blocks are confined to the Objective-C message sends.
//!
//! Also exposes a fire-and-forget helper to play the system trash sound.

use objc2::msg_send;
use objc2::runtime::AnyObject;
use objc2_foundation::NSString;
use std::path::{Path, PathBuf};

/// Move a file or directory to the user's Trash using the standard macOS
/// `NSFileManager trashItemAtURL:resultingItemURL:error:` API.
///
/// Returns the path the item was moved to inside Trash, which is what you
/// need to restore it later.
pub fn trash_file(path: &Path) -> Result<PathBuf, String> {
    unsafe {
        // NSFileManager *fm = [NSFileManager defaultManager];
        let fm_class = objc2::runtime::AnyClass::get("NSFileManager")
            .ok_or_else(|| "NSFileManager class not found".to_string())?;
        let fm: *mut AnyObject = msg_send![fm_class, defaultManager];
        if fm.is_null() {
            return Err("NSFileManager defaultManager returned nil".into());
        }

        // NSURL *url = [NSURL fileURLWithPath:<path>];
        let url_class = objc2::runtime::AnyClass::get("NSURL")
            .ok_or_else(|| "NSURL class not found".to_string())?;
        let path_str = NSString::from_str(path.to_string_lossy().as_ref());
        let url: *mut AnyObject = msg_send![url_class, fileURLWithPath: &*path_str];
        if url.is_null() {
            return Err(format!("NSURL fileURLWithPath returned nil for {:?}", path));
        }

        // NSURL *resultingURL = nil;
        // NSError *error = nil;
        // BOOL ok = [fm trashItemAtURL:url resultingItemURL:&resultingURL error:&error];
        let mut resulting: *mut AnyObject = std::ptr::null_mut();
        let mut error: *mut AnyObject = std::ptr::null_mut();
        let ok: bool = msg_send![
            fm,
            trashItemAtURL: url,
            resultingItemURL: &mut resulting as *mut _,
            error: &mut error as *mut _,
        ];

        if !ok {
            let msg = if error.is_null() {
                "NSFileManager trashItemAtURL failed".to_string()
            } else {
                let desc: *mut AnyObject = msg_send![error, localizedDescription];
                if desc.is_null() {
                    "NSFileManager trashItemAtURL failed (no description)".to_string()
                } else {
                    let ns: &NSString = &*(desc as *const NSString);
                    ns.to_string()
                }
            };
            return Err(msg);
        }

        if resulting.is_null() {
            // The file was trashed but no resulting URL was returned.
            // We can't undo, but the trash succeeded. Return the original path
            // so the caller at least knows the op succeeded.
            return Ok(path.to_path_buf());
        }

        // NSString *resultingPath = [resultingURL path];
        let resulting_path: *mut AnyObject = msg_send![resulting, path];
        if resulting_path.is_null() {
            return Ok(path.to_path_buf());
        }
        let ns: &NSString = &*(resulting_path as *const NSString);
        Ok(PathBuf::from(ns.to_string()))
    }
}

/// Move a file back from Trash (or anywhere) to its original location.
/// Used for undo.
pub fn restore_file(from: &Path, to: &Path) -> Result<(), String> {
    unsafe {
        let fm_class = objc2::runtime::AnyClass::get("NSFileManager")
            .ok_or_else(|| "NSFileManager class not found".to_string())?;
        let fm: *mut AnyObject = msg_send![fm_class, defaultManager];
        if fm.is_null() {
            return Err("NSFileManager defaultManager returned nil".into());
        }

        let url_class = objc2::runtime::AnyClass::get("NSURL")
            .ok_or_else(|| "NSURL class not found".to_string())?;

        let from_str = NSString::from_str(from.to_string_lossy().as_ref());
        let to_str = NSString::from_str(to.to_string_lossy().as_ref());
        let src_url: *mut AnyObject = msg_send![url_class, fileURLWithPath: &*from_str];
        let dst_url: *mut AnyObject = msg_send![url_class, fileURLWithPath: &*to_str];

        let mut error: *mut AnyObject = std::ptr::null_mut();
        let ok: bool = msg_send![
            fm,
            moveItemAtURL: src_url,
            toURL: dst_url,
            error: &mut error as *mut _,
        ];
        // Explicitly keep these alive through the msg_send so the compiler
        // doesn't drop them before the NSURL has extracted their contents.
        drop(from_str);
        drop(to_str);

        if !ok {
            let msg = if error.is_null() {
                "NSFileManager moveItemAtURL failed".to_string()
            } else {
                let desc: *mut AnyObject = msg_send![error, localizedDescription];
                if desc.is_null() {
                    "NSFileManager moveItemAtURL failed (no description)".to_string()
                } else {
                    let ns: &NSString = &*(desc as *const NSString);
                    ns.to_string()
                }
            };
            return Err(msg);
        }
        Ok(())
    }
}

/// Play the macOS trash sound. Fire-and-forget — never blocks, never errors.
/// Uses `afplay` on the system Purr sound since it's simpler and avoids
/// NSSound lifetime headaches.
pub fn play_trash_sound() {
    let _ = std::process::Command::new("afplay")
        .arg("/System/Library/Sounds/Purr.aiff")
        .spawn();
}
