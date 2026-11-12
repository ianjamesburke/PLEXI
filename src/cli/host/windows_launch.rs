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
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "NUL in host executable",
        ));
    }
    let mut command_line = vec![b'"' as u16];
    command_line.extend_from_slice(&application);
    command_line.extend([b'"' as u16, 0]);
    application.push(0);
    // CreateProcess requires a sorted, double-NUL-terminated Unicode block.
    let mut entries = env.to_vec();
    entries.sort_by_key(|(key, _)| key.to_uppercase());
    let mut environment = Vec::new();
    for (key, value) in entries {
        // Windows also exports hidden drive-current-directory keys (=C:).
        if key.is_empty()
            || key.contains('\0')
            || key.strip_prefix('=').unwrap_or(&key).contains('=')
            || value.contains('\0')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid host environment",
            ));
        }
        environment.extend(format!("{key}={value}").encode_utf16());
        environment.push(0);
    }
    environment.push(0);
    if environment.len() == 1 {
        environment.push(0);
    }
    // No args. Quote argv[0] as well as passing lpApplicationName separately:
    // the module path chooses the executable, but the command line is what
    // Rust parses into arguments when the install directory contains spaces.
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
            command_line.as_mut_ptr(),
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
    log::info!(
        "host_start: Windows detached pid={} inherit_handles=false",
        process.dwProcessId
    );
    Ok(process.dwProcessId)
}
