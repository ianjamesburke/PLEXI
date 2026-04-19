//! FD isolation helpers — spec I-7 (`docs/specs/releases/plexi-v3.0.md §5`).
//!
//! Every long-lived host FD (UnixListener sockets, JSONL log files, event-log
//! writers) must carry `FD_CLOEXEC` so it disappears from the child FD table
//! across any subsequent `exec()`. Rust's stdlib already sets `O_CLOEXEC` on
//! `File::open` (via `openat(O_CLOEXEC)`) and `SOCK_CLOEXEC` on `UnixListener`
//! on Linux ≥1.80, but **not on macOS** — there the flag must be set
//! explicitly after the underlying syscall returns.
//!
//! Use [`set_cloexec`] immediately after `bind`/`open`; it's a no-op on
//! platforms where the flag is already set, so callers don't have to branch.

use std::io;
use std::os::fd::RawFd;

use nix::fcntl::{fcntl, FcntlArg, FdFlag};

/// Set `FD_CLOEXEC` on the given raw FD. Idempotent — safe to call on an FD
/// that already has the flag.
pub fn set_cloexec(fd: RawFd) -> io::Result<()> {
    // Read current flags so we don't clobber anything else (there is only
    // one other possible flag on FD_FLAGS today, but future-proof the OR).
    let existing = fcntl(fd, FcntlArg::F_GETFD)
        .map_err(|e| io::Error::from_raw_os_error(e as i32))?;
    let flags = FdFlag::from_bits_truncate(existing) | FdFlag::FD_CLOEXEC;
    fcntl(fd, FcntlArg::F_SETFD(flags))
        .map(|_| ())
        .map_err(|e| io::Error::from_raw_os_error(e as i32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::io::AsRawFd;

    #[test]
    fn set_cloexec_is_idempotent_on_tempfile() {
        let tmp = tempfile_new();
        let fd = tmp.as_raw_fd();
        set_cloexec(fd).expect("first call");
        set_cloexec(fd).expect("second call (idempotent)");

        let flags = fcntl(fd, FcntlArg::F_GETFD).expect("read back flags");
        assert!(
            FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC),
            "FD_CLOEXEC must be set after set_cloexec()"
        );
    }

    fn tempfile_new() -> std::fs::File {
        let path = std::env::temp_dir().join(format!(
            "plexi-fd-util-{}.tmp",
            std::process::id()
        ));
        std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .read(true)
            .open(&path)
            .expect("open tempfile")
    }
}
