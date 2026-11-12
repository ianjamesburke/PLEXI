//! Process-wide Wasmtime trap policy shared by core modules and components.

pub(super) fn create_engine() -> wasmtime::Result<wasmtime::Engine> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    // Wasmtime 43's Mach receiver aborts on a caught signal interrupting its
    // wait (https://github.com/bytecodealliance/wasmtime/issues/14393). Plexi
    // launches/reaps children while running guests, so use Wasmtime's supported
    // Unix signal backend. We do not embed a Mach-port crash reporter. All
    // engines must agree: Wasmtime installs its trap backend process-wide.
    config.macos_use_mach_ports(false);
    let engine = wasmtime::Engine::new(&config).inspect_err(|error| {
        log::error!("wasm engine: failed to initialize shared trap policy: {error}");
    })?;
    #[cfg(target_os = "macos")]
    log::info!("wasm engine: using Unix signal trap handling");
    Ok(engine)
}

#[cfg(test)]
mod tests {
    use super::create_engine;

    fn assert_guest_fault_is_recoverable(engine: &wasmtime::Engine) {
        let module = wasmtime::Module::new(
            engine,
            r#"(module
                (memory 1)
                (func (export "load") (param i32) (result i32)
                    local.get 0
                    i32.load))"#,
        )
        .expect("compile real guest memory access");
        let mut store = wasmtime::Store::new(engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let load = instance
            .get_typed_func::<i32, i32>(&mut store, "load")
            .unwrap();
        let error = load
            .call(&mut store, 65_536)
            .expect_err("out-of-bounds guest load");
        assert_eq!(
            error.downcast_ref::<wasmtime::Trap>(),
            Some(&wasmtime::Trap::MemoryOutOfBounds)
        );
        assert_eq!(
            load.call(&mut store, 0).unwrap(),
            0,
            "guest remains callable after trapping"
        );
    }

    #[test]
    fn guest_memory_fault_returns_trap_and_execution_recovers() {
        assert_guest_fault_is_recoverable(&create_engine().unwrap());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn child_exit_signal_does_not_abort_wasm_host() {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        const CHILD: &str = "PLEXI_WASM_SIGNAL_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "host::wasm_engine::tests::child_exit_signal_does_not_abort_wasm_host",
                    "--test-threads=1",
                    "--nocapture",
                ])
                .env(CHILD, "1");
            // The isolated libtest main thread must inherit a blocked SIGCHLD,
            // so it cannot consume the signal intended for the trap receiver.
            unsafe {
                command.pre_exec(|| {
                    let mut mask: libc::sigset_t = std::mem::zeroed();
                    libc::sigemptyset(&mut mask);
                    libc::sigaddset(&mut mask, libc::SIGCHLD);
                    if libc::sigprocmask(libc::SIG_BLOCK, &mask, std::ptr::null_mut()) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let mut child = command
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let deadline =
                Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(20));
            while child.try_wait().unwrap().is_none() {
                if Instant::now() >= deadline {
                    child.kill().expect("terminate timed-out signal test");
                    child.wait().expect("reap timed-out signal test");
                    panic!("isolated Wasmtime signal test timed out");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "isolated Wasmtime signal test failed: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        extern "C" fn on_child_exit(_: libc::c_int) {}
        // Signal disposition/masks are changed only in the isolated subprocess.
        // With the faulty Mach backend, its receiver is the only thread left
        // accepting SIGCHLD after engine creation, making the abort repeatable.
        let mut mask: libc::sigset_t = unsafe { std::mem::zeroed() };
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_child_exit as *const () as usize;
            libc::sigemptyset(&mut action.sa_mask);
            assert_eq!(
                libc::sigaction(libc::SIGCHLD, &action, std::ptr::null_mut()),
                0
            );
            libc::sigemptyset(&mut mask);
            libc::sigaddset(&mut mask, libc::SIGCHLD);
            assert_eq!(
                libc::pthread_sigmask(libc::SIG_UNBLOCK, &mask, std::ptr::null_mut()),
                0
            );
        }
        let engine = create_engine().unwrap();
        unsafe {
            assert_eq!(
                libc::pthread_sigmask(libc::SIG_BLOCK, &mask, std::ptr::null_mut()),
                0
            );
        }
        for _ in 0..8 {
            assert!(Command::new("/usr/bin/true").status().unwrap().success());
        }
        assert_guest_fault_is_recoverable(&engine);
    }
}
