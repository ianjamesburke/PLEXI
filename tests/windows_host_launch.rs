//! Native Windows regression isolated from the host suite's Unix-only tests.
//! Run: cargo test --test windows_host_launch
#![cfg(windows)]
#[path = "../src/testing/timeout.rs"]
mod timeouts;
#[path = "../src/cli/host/windows_launch.rs"]
mod windows_launch;

#[cfg(test)]
mod tests {
    use super::windows_launch::spawn;
    use std::io::Read;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Pipes::CreatePipe;

    #[test]
    fn detached_host_does_not_keep_callers_capture_pipe_alive() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("host probe.exe");
        let build = std::process::Command::new("rustc")
            .args([
                "--edition=2021",
                "tests/fixtures/host_detached_probe.rs",
                "-o",
            ])
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        let mut read = std::ptr::null_mut();
        let mut write = std::ptr::null_mut();
        // SAFETY: valid writable outputs, initialized security attributes.
        assert_ne!(
            unsafe { CreatePipe(&mut read, &mut write, &attributes, 0) },
            0
        );
        // SAFETY: CreatePipe returned two distinct owned handles.
        let read = unsafe { OwnedHandle::from_raw_handle(read.cast()) };
        let write = unsafe { OwnedHandle::from_raw_handle(write.cast()) };
        let report = dir.path().join("report");
        let mut env: Vec<_> = std::env::vars().collect();
        env.push((
            "HOST01_CAPTURE_HANDLE".into(),
            (write.as_raw_handle() as usize).to_string(),
        ));
        env.push(("=C:".into(), dir.path().to_string_lossy().into_owned()));
        env.push((
            "HOST01_REPORT".into(),
            report.to_string_lossy().into_owned(),
        ));
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
        let timeout = super::timeouts::load_aware_timeout(Duration::from_secs(5));
        let deadline = Instant::now() + timeout;
        while !report.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let result = std::fs::read_to_string(report).unwrap();
        assert_eq!(
            result,
            format!("{pid}:false"),
            "extra capture handle inherited"
        );
        assert_eq!(
            rx.recv_timeout(timeout)
                .expect("EOF while host is alive")
                .unwrap(),
            0
        );
    }
}
