//! Windows analogues of `libc::kill` / `libc::waitpid` for `process_app/`.
//!
//! All `unsafe` Win32 calls live behind the safe `Result<(), String>`
//! wrappers below — keep callers from touching `windows_sys::Win32::*`
//! directly so the unsafe surface stays auditable in one file.
//!
//! There is no Win32 SIGTERM, so the Unix "polite SIGTERM then SIGKILL after
//! 1s" pattern collapses to a single `terminate` call (TerminateProcess is
//! unconditional).

#![cfg(windows)]

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, FALSE, WAIT_FAILED,
};
// SYNCHRONIZE lives under Storage::FileSystem in windows-sys 0.61 (it's an
// ACCESS_MASK constant, grouped there with FILE_ACCESS_RIGHTS).
use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
use windows_sys::Win32::System::Threading::{
    OpenProcess, TerminateProcess, WaitForSingleObject, INFINITE, PROCESS_TERMINATE,
};

/// Terminate a Windows process by pid. Equivalent to `SIGKILL` on Unix —
/// unconditional, no graceful-shutdown handshake.
///
/// Returns `Ok(())` on success or if the process had already exited
/// (`OpenProcess` failing with `ERROR_INVALID_PARAMETER`). Returns `Err`
/// with a descriptive string on any other `OpenProcess` failure or on
/// `TerminateProcess` failure.
pub fn terminate(pid: u32) -> Result<(), String> {
    // SAFETY: OpenProcess returns null on failure; we never deref a null handle.
    let handle = unsafe { OpenProcess(PROCESS_TERMINATE, FALSE, pid) };
    if handle.is_null() {
        let err = unsafe { GetLastError() };
        // pid already gone satisfies the "process is dead" goal.
        if err == ERROR_INVALID_PARAMETER {
            return Ok(());
        }
        return Err(format!("OpenProcess(PROCESS_TERMINATE, pid={pid}) failed: GetLastError={err}"));
    }

    // Exit code 1 mirrors SIGKILL semantics (non-zero = abnormal exit).
    // SAFETY: handle came from a successful OpenProcess.
    let ok = unsafe { TerminateProcess(handle, 1) };
    let terminate_err = if ok == 0 { Some(unsafe { GetLastError() }) } else { None };

    // Close on every path; leaked handles keep the kernel object alive after exit.
    // SAFETY: same valid handle.
    unsafe {
        CloseHandle(handle);
    }

    match terminate_err {
        Some(err) => Err(format!(
            "TerminateProcess(pid={pid}) failed: GetLastError={err}"
        )),
        None => Ok(()),
    }
}

/// Block until the process with `pid` exits. Equivalent to
/// `libc::waitpid(pid, _, 0)` with `options = 0`.
///
/// Returns `Ok(())` on normal exit (the wait completing or the pid already
/// being gone). Returns `Err` on `OpenProcess` failure (other than
/// already-gone) or on `WaitForSingleObject` returning `WAIT_FAILED`.
pub fn wait(pid: u32) -> Result<(), String> {
    // SAFETY: OpenProcess returns null on failure; we never deref a null handle.
    let handle = unsafe { OpenProcess(SYNCHRONIZE, FALSE, pid) };
    if handle.is_null() {
        let err = unsafe { GetLastError() };
        // pid already gone satisfies the wait.
        if err == ERROR_INVALID_PARAMETER {
            return Ok(());
        }
        return Err(format!(
            "OpenProcess(SYNCHRONIZE, pid={pid}) failed: GetLastError={err}"
        ));
    }

    // SAFETY: handle came from a successful OpenProcess with SYNCHRONIZE.
    let result = unsafe { WaitForSingleObject(handle, INFINITE) };
    let wait_err = if result == WAIT_FAILED {
        Some(unsafe { GetLastError() })
    } else {
        None
    };

    // SAFETY: same valid handle.
    unsafe {
        CloseHandle(handle);
    }

    match wait_err {
        Some(err) => Err(format!(
            "WaitForSingleObject(pid={pid}) failed: GetLastError={err}"
        )),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Terminating a pid that doesn't exist must be a no-op success — the
    /// goal is "process is dead", which is trivially true.
    #[test]
    fn terminate_nonexistent_pid_is_ok() {
        // pid 0xFFFF_FFF0 is well above any realistic process and the
        // kernel will refuse OpenProcess with ERROR_INVALID_PARAMETER.
        assert!(terminate(0xFFFF_FFF0).is_ok());
    }

    /// Same idea for wait — waiting on a gone pid is satisfied immediately.
    #[test]
    fn wait_nonexistent_pid_is_ok() {
        assert!(wait(0xFFFF_FFF0).is_ok());
    }

    /// End-to-end smoke: spawn a `cmd /c exit 0`, terminate it (race with
    /// natural exit is fine — both end states satisfy the contract), then
    /// wait should also succeed.
    #[test]
    fn terminate_and_wait_round_trip() {
        let child = std::process::Command::new("cmd")
            .args(["/c", "ping -n 5 127.0.0.1 > NUL"])
            .spawn()
            .expect("spawn cmd");
        let pid = child.id();
        // Don't wait on the child handle directly; we want the helpers
        // exercised by themselves. Leak the Child intentionally and rely
        // on `wait()` below.
        std::mem::forget(child);
        terminate(pid).expect("terminate");
        wait(pid).expect("wait");
    }
}
