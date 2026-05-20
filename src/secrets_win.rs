//! Windows Credential Manager backend for the `secrets` API.
//!
//! This module is the Windows analogue of the macOS `security_framework`
//! calls scattered across `secrets.rs` and `workspace_secrets.rs`. All
//! `unsafe` Win32 FFI is encapsulated here; callers see only safe
//! `Option`/`Result` returns.
//!
//! ## TargetName convention
//!
//! The macOS path uses Keychain `service="plexi"` plus an `account` string
//! built by helpers like `account_key` / `account_key_scoped` /
//! `keychain_workspace_name` / `keychain_user_name`. Win32 has only a
//! single flat `TargetName` namespace per user, so we prefix every account
//! with the literal string `"plexi/"` to (a) keep PLEXI's credentials
//! visually grouped in the Windows Credential Manager UI and (b) enable
//! `CredEnumerateW` filtering with the prefix `"plexi/*"`.
//!
//! For workspace-scoped secrets the account already starts with `"plexi/"`
//! (e.g. `"plexi/<workspace_root>/<key>"`), which produces the redundant
//! `"plexi/plexi/<workspace_root>/<key>"` TargetName. That double-prefix
//! is intentional: the redundancy is invisible to users (they don't see
//! TargetName), and pushing the prefix in only one place keeps the rule
//! trivial — every caller passes its account string verbatim and we
//! prepend `"plexi/"`.

#![cfg(windows)]

use log::warn;
use zeroize::Zeroizing;

use windows_sys::Win32::Foundation::{ERROR_NOT_FOUND, GetLastError};
use windows_sys::Win32::Security::Credentials::{
    CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CredDeleteW, CredEnumerateW,
    CredFree, CredReadW, CredWriteW,
};

/// Prefix applied to every PLEXI account string before it becomes a
/// Win32 Credential Manager TargetName. See the module-level doc.
const TARGET_PREFIX: &str = "plexi/";

/// Build the full TargetName (caller-facing account string with the
/// PLEXI prefix prepended).
fn target_name(account: &str) -> String {
    format!("{TARGET_PREFIX}{account}")
}

/// Encode a `&str` as a UTF-16 null-terminated buffer suitable for the
/// `PCWSTR` / `PWSTR` arguments of the Win32 API.
fn to_wide_nul(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

/// Decode a null-terminated UTF-16 buffer (as returned in `CREDENTIALW`
/// fields like `TargetName`) into a Rust `String`. Reads word-by-word
/// until the first null. Safe to call on any non-null `*const u16`.
///
/// # Safety
/// `ptr` must be either null or a valid null-terminated UTF-16 string
/// owned by the OS for the duration of this call.
unsafe fn read_wide_nul(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    // Cap at 4096 to defend against a missing terminator. Any real
    // TargetName is well under that.
    while len < 4096 && unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    String::from_utf16_lossy(slice)
}

/// Read the credential at `account` (after the `"plexi/"` prefix is
/// applied). Returns the secret as a `Zeroizing<String>` on success, or
/// `None` if no credential exists OR any other error occurred. Errors
/// other than `ERROR_NOT_FOUND` are logged as warnings.
pub fn cred_read(account: &str) -> Option<Zeroizing<String>> {
    let target = target_name(account);
    let wide = to_wide_nul(&target);
    let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
    let ok = unsafe { CredReadW(wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        if err != ERROR_NOT_FOUND {
            warn!(
                "secrets_win::cred_read: CredReadW failed for target='{target}' (GetLastError={err})"
            );
        }
        return None;
    }
    // SAFETY: CredReadW returned TRUE, so `credential` points at a
    // single OS-allocated block we must read from and then CredFree.
    let value = unsafe {
        let cred = &*credential;
        let size = cred.CredentialBlobSize as usize;
        if cred.CredentialBlob.is_null() || size == 0 {
            String::new()
        } else {
            let bytes = std::slice::from_raw_parts(cred.CredentialBlob, size);
            String::from_utf8_lossy(bytes).trim().to_string()
        }
    };
    unsafe { CredFree(credential as *const core::ffi::c_void) };
    Some(Zeroizing::new(value))
}

/// Write `value` to the credential at `account` (with `"plexi/"` prefix
/// applied). Persists at `CRED_PERSIST_LOCAL_MACHINE`. Returns `Ok` on
/// success; otherwise an `Err` with a human-readable description.
pub fn cred_write(account: &str, value: &str) -> Result<(), String> {
    let target = target_name(account);
    // Wide TargetName must live until CredWriteW returns. The function
    // declares the field as a mutable `PWSTR`, but it does not modify
    // the buffer — we just need a stable pointer.
    let mut wide_target = to_wide_nul(&target);
    // CredentialBlob is a `*mut u8` pointing at UTF-8 bytes of `value`.
    // The buffer must outlive CredWriteW.
    let blob: Vec<u8> = value.as_bytes().to_vec();

    let mut credential: CREDENTIALW = unsafe { std::mem::zeroed() };
    credential.Type = CRED_TYPE_GENERIC;
    credential.TargetName = wide_target.as_mut_ptr();
    credential.CredentialBlobSize = blob.len() as u32;
    credential.CredentialBlob = blob.as_ptr() as *mut u8;
    credential.Persist = CRED_PERSIST_LOCAL_MACHINE;

    let ok = unsafe { CredWriteW(&credential, 0) };
    // Tie the buffer lifetimes to here so they cannot be dropped early.
    drop(blob);
    drop(wide_target);
    if ok == 0 {
        let err = unsafe { GetLastError() };
        return Err(format!(
            "CredWriteW failed for target='{target}' (GetLastError={err})"
        ));
    }
    Ok(())
}

/// Delete the credential at `account`. `ERROR_NOT_FOUND` is treated as
/// success (consistent with the macOS path which folds errSecItemNotFound
/// into success in `delete_secret`).
pub fn cred_delete(account: &str) -> Result<(), String> {
    let target = target_name(account);
    let wide = to_wide_nul(&target);
    let ok = unsafe { CredDeleteW(wide.as_ptr(), CRED_TYPE_GENERIC, 0) };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        if err == ERROR_NOT_FOUND {
            return Ok(());
        }
        return Err(format!(
            "CredDeleteW failed for target='{target}' (GetLastError={err})"
        ));
    }
    Ok(())
}

/// Enumerate credentials whose TargetName matches `filter` (a prefix +
/// `*`, e.g. `"plexi/*"`). Returns the matching TargetName strings with
/// the leading `"plexi/"` already stripped — callers work in account
/// space, not TargetName space. Returns an empty `Vec` on
/// `ERROR_NOT_FOUND` or any other failure (failures are logged).
pub fn cred_list(filter: &str) -> Vec<String> {
    let wide = to_wide_nul(filter);
    let mut count: u32 = 0;
    let mut credentials: *mut *mut CREDENTIALW = std::ptr::null_mut();
    let ok = unsafe { CredEnumerateW(wide.as_ptr(), 0, &mut count, &mut credentials) };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        if err != ERROR_NOT_FOUND {
            warn!(
                "secrets_win::cred_list: CredEnumerateW failed for filter='{filter}' (GetLastError={err})"
            );
        }
        return Vec::new();
    }
    let mut out = Vec::with_capacity(count as usize);
    // SAFETY: `credentials` points at an array of `count` pointers to
    // CREDENTIALW. The whole array is a single OS-allocated block and
    // must be freed exactly once via CredFree(credentials).
    for i in 0..(count as usize) {
        unsafe {
            let entry = *credentials.add(i);
            if entry.is_null() {
                continue;
            }
            let target = read_wide_nul((*entry).TargetName);
            // Strip the PLEXI prefix so callers receive account strings.
            let account = target
                .strip_prefix(TARGET_PREFIX)
                .map(str::to_string)
                .unwrap_or(target);
            out.push(account);
        }
    }
    unsafe { CredFree(credentials as *const core::ffi::c_void) };
    out
}
