//! Detached GUI launch with no inherited handles, including anonymous capture
//! pipes inherited by the CLI itself. `Command`'s NUL stdio redirection alone
//! does not prevent those extra handles keeping a caller's ReadToEnd alive.
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::path::Path;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, CREATE_NEW_PROCESS_GROUP, CREATE_UNICODE_ENVIRONMENT, DETACHED_PROCESS,
    PROCESS_INFORMATION, STARTUPINFOW,
};

pub(super) fn spawn(binary: &Path, env: &[(String, String)]) -> io::Result<u32> {
    let mut application: Vec<u16> = binary.as_os_str().encode_wide().collect();
    if application.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "NUL in host executable"));
    }
    application.push(0);
    // CreateProcess requires a sorted, double-NUL-terminated Unicode block.
    let mut entries = env.to_vec();
    entries.sort_by_key(|(key, _)| key.to_uppercase());
    let mut environment = Vec::new();
    for (key, value) in entries {
        if key.is_empty() || key.contains(['\0', '=']) || value.contains('\0') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid host environment"));
        }
        environment.extend(format!("{key}={value}").encode_utf16());
        environment.push(0);
    }
    environment.push(0);
    if environment.len() == 1 {
        environment.push(0);
    }
    // No args: Windows uses lpApplicationName as the command line. Pass the
    // full executable path separately, so spaces cannot select another exe.
    // No STARTF_USESTDHANDLES: this detached GUI host has no standard streams.
    // SAFETY: zero is valid for unused STARTUPINFO / PROCESS_INFORMATION fields.
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: both input buffers remain live and terminated through the call;
    // out-pointers address initialized, writable structs. Inheritance is FALSE.
    let ok = unsafe {
        CreateProcessW(
            application.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            std::ptr::null(),
            &startup,
            &mut process,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful CreateProcess transfers these two distinct handles.
    // Closing our references does not stop the independently running host.
    let _process = unsafe { OwnedHandle::from_raw_handle(process.hProcess.cast()) };
    let _thread = unsafe { OwnedHandle::from_raw_handle(process.hThread.cast()) };
    log::info!("host_start: Windows detached pid={} inherit_handles=false", process.dwProcessId);
    Ok(process.dwProcessId)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::windows::io::AsRawHandle;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Pipes::CreatePipe;

    #[test]
    fn detached_host_does_not_keep_callers_capture_pipe_alive() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("host probe.exe");
        let build = std::process::Command::new("rustc")
            .args(["--edition=2021", "tests/fixtures/host_detached_probe.rs", "-o"])
            .arg(&binary).output().unwrap();
        assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        let mut read = std::ptr::null_mut();
        let mut write = std::ptr::null_mut();
        // SAFETY: valid writable outputs, initialized security attributes.
        assert_ne!(unsafe { CreatePipe(&mut read, &mut write, &mut attributes, 0) }, 0);
        // SAFETY: CreatePipe returned two distinct owned handles.
        let read = unsafe { OwnedHandle::from_raw_handle(read.cast()) };
        let write = unsafe { OwnedHandle::from_raw_handle(write.cast()) };
        let report = dir.path().join("report");
        let mut env: Vec<_> = std::env::vars().collect();
        env.push(("HOST01_CAPTURE_HANDLE".into(), (write.as_raw_handle() as usize).to_string()));
        env.push(("HOST01_REPORT".into(), report.to_string_lossy().into_owned()));
        let pid = spawn(&binary, &env).unwrap();
        // Always release the stand-in, even if an assertion fails.
        struct Stop(std::path::PathBuf);
        impl Drop for Stop {
            fn drop(&mut self) {
                let _ = std::fs::write(&self.0, "stop");
            }
        }
        let _stop = Stop(dir.path().join("report.stop"));
        drop(write);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut file = std::fs::File::from(read);
            let mut bytes = Vec::new();
            let _ = tx.send(file.read_to_end(&mut bytes));
        });
        let timeout = crate::testing::load_aware_timeout(Duration::from_secs(5));
        let deadline = Instant::now() + timeout;
        while !report.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let result = std::fs::read_to_string(report).unwrap();
        assert_eq!(result, format!("{pid}:false"), "extra capture handle inherited");
        assert_eq!(rx.recv_timeout(timeout).expect("EOF while host is alive").unwrap(), 0);
    }
}
