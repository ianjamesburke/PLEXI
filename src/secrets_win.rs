//! Windows Credential Manager backend for the `secrets` API. Windows analogue
//! of the macOS `security_framework` calls in `secrets.rs` and
//! `workspace_secrets.rs`. All `unsafe` Win32 FFI is encapsulated here.
//!
//! TargetName: every account string is prefixed with `"plexi/"` so PLEXI's
//! credentials group visually in the Credential Manager UI and so
//! `CredEnumerateW("plexi/*")` enumerates the set. Workspace-scoped accounts
//! already begin with `"plexi/"`, producing a redundant
//! `"plexi/plexi/<workspace_root>/<key>"` — kept intentionally so the prefix
//! rule is a one-liner and callers pass their account strings verbatim.

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

fn target_name(account: &str) -> String {
    format!("{TARGET_PREFIX}{account}")
}

/// UTF-16 NUL-terminated buffer for Win32 PCWSTR / PWSTR arguments.
fn to_wide_nul(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

/// Decode a NUL-terminated UTF-16 buffer (e.g. `CREDENTIALW.TargetName`)
/// into a `String`. Length is capped at 4096 to defend against a missing
/// terminator; real TargetNames are well under that.
///
/// # Safety
/// `ptr` must be either null or a valid NUL-terminated UTF-16 string
/// owned by the OS for the duration of the call.
unsafe fn read_wide_nul(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while len < 4096 && unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    String::from_utf16_lossy(slice)
}

/// Returns the secret bytes, or `None` if no credential exists or any
/// other error occurred (errors other than `ERROR_NOT_FOUND` are logged).
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

/// Persists at `CRED_PERSIST_LOCAL_MACHINE`.
pub fn cred_write(account: &str, value: &str) -> Result<(), String> {
    let target = target_name(account);
    // Both buffers must outlive CredWriteW — they're pointed at by `credential`.
    let mut wide_target = to_wide_nul(&target);
    let blob = Zeroizing::new(value.as_bytes().to_vec());

    let mut credential: CREDENTIALW = unsafe { std::mem::zeroed() };
    credential.Type = CRED_TYPE_GENERIC;
    credential.TargetName = wide_target.as_mut_ptr();
    credential.CredentialBlobSize = blob.len() as u32;
    credential.CredentialBlob = blob.as_ptr() as *mut u8;
    credential.Persist = CRED_PERSIST_LOCAL_MACHINE;

    let ok = unsafe { CredWriteW(&credential, 0) };
    // Explicit drops tie the buffer lifetimes past the FFI call (NLL would
    // otherwise release them earlier).
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

/// `ERROR_NOT_FOUND` is treated as success (matches the macOS path's
/// errSecItemNotFound handling in `delete_secret`).
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

/// `filter` is a TargetName prefix wildcard (e.g. `"plexi/*"`). Returns
/// account strings — the `"plexi/"` prefix is stripped before return.
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
    // SAFETY: `credentials` is a single OS-allocated block of `count` CREDENTIALW
    // pointers; freed once below via CredFree.
    for i in 0..(count as usize) {
        unsafe {
            let entry = *credentials.add(i);
            if entry.is_null() {
                continue;
            }
            let target = read_wide_nul((*entry).TargetName);
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
