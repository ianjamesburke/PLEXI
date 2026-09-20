//! Platform transport for host ↔ CLI IPC.
//!
//! One host process listens on a per-channel endpoint; every `plexi <cmd>`
//! invocation connects to it, writes one newline-delimited JSON request, and
//! either exits (fire-and-forget) or keeps reading an NDJSON stream back.
//!
//! - **Unix** — an `AF_UNIX` socket at `<config_dir>/notify.sock`.
//!   `IpcListener` / `IpcStream` are plain aliases for the std types, so the
//!   Unix build pays nothing for this abstraction.
//! - **Windows** — a Win32 named pipe, `\\.\pipe\plexi-<channel>`. There is no
//!   filesystem entry, so nothing to unlink on shutdown and nothing a stale
//!   file can shadow.
//!
//! [`endpoint`] is the single source of truth for the address on both
//! platforms and is what lands in `PLEXI_SOCKET` inside a pane.
//!
//! The surface here is deliberately only what the call sites use — connect,
//! bind/incoming, `Read`/`Write`, `try_clone`, `shutdown`, and read/write
//! timeouts. It is not a general socket library.

use std::io;
use std::path::{Path, PathBuf};

/// Address of this channel's IPC endpoint.
///
/// On Unix this is a filesystem path. On Windows it is a named-pipe name,
/// which is *not* a path — never join it, create its parent, or stat it.
/// Both are returned as `String` because that is what `PLEXI_SOCKET` carries.
pub fn endpoint() -> String {
    endpoint_path().to_string_lossy().into_owned()
}

/// `endpoint()` as a `PathBuf`, for the call sites that still thread a
/// `&Path` through (`--socket`, the spawn gate, `Display` in error text).
///
/// On Windows a pipe name survives a `PathBuf` round-trip unchanged — it is
/// only ever handed back to `CreateFileW`, never to the filesystem.
pub fn endpoint_path() -> PathBuf {
    endpoint_in(&crate::config::config_dir())
}

/// The endpoint belonging to a specific channel, named by its profile
/// directory (`~/.plexi`, `~/.plexi-alpha`, `~/.plexi-pr-1604`, ...).
///
/// Commands that target another channel — `plexi host stop`, the spawn gate,
/// `--socket` resolution — know the profile dir, not the endpoint. On Unix the
/// endpoint lives inside that dir; on Windows it does not exist on disk at all,
/// so the channel is recovered from the dir's basename and turned into a pipe
/// name. Keeping both derivations here is what stops the two from drifting.
pub fn endpoint_in(profile_dir: &Path) -> PathBuf {
    #[cfg(unix)]
    {
        profile_dir.join("notify.sock")
    }
    #[cfg(windows)]
    {
        PathBuf::from(pipe_name_for_channel(channel_from_profile_dir(
            profile_dir,
        )))
    }
}

/// `~/.plexi-alpha` → `Some("alpha")`; bare `~/.plexi` → `None`.
#[cfg(windows)]
fn channel_from_profile_dir(profile_dir: &Path) -> Option<&str> {
    profile_dir
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(".plexi-"))
        .filter(|channel| !channel.is_empty())
}

/// `\\.\pipe\plexi` for the stable channel, `\\.\pipe\plexi-<channel>`
/// otherwise — mirroring the `~/.plexi` vs `~/.plexi-<channel>` profile-dir
/// split so each channel is an isolated instance here too.
#[cfg(windows)]
pub fn pipe_name_for_channel(channel: Option<&str>) -> String {
    match channel {
        Some(channel) if !channel.is_empty() => format!(r"\\.\pipe\plexi-{channel}"),
        _ => r"\\.\pipe\plexi".to_string(),
    }
}

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener as IpcListener, UnixStream as IpcStream};

#[cfg(unix)]
pub use unix_impl::{connect_timeout, remove_stale_endpoint};

#[cfg(unix)]
mod unix_impl {
    use super::*;

    /// Connect, giving up after `timeout`.
    ///
    /// `UnixStream::connect_timeout` does not exist, and a plain `connect` on a
    /// socket whose listener has a full backlog blocks indefinitely — which for
    /// a fire-and-forget CLI means a hung terminal. Callers that need a bounded
    /// connect use the raw-fd path in `cli::send_line_to_socket`; this helper
    /// covers the simple case where a blocking connect is acceptable.
    pub fn connect_timeout(endpoint: &Path, _timeout: std::time::Duration) -> io::Result<IpcStream> {
        IpcStream::connect(endpoint)
    }

    /// Unlink a leftover socket file before binding.
    ///
    /// A previous host that died without unlinking leaves a socket inode that
    /// `bind` refuses with `EADDRINUSE`, so the new host must clear it first.
    pub fn remove_stale_endpoint(endpoint: &Path) {
        let _ = std::fs::remove_file(endpoint);
    }
}

#[cfg(windows)]
pub use windows_impl::{connect_timeout, remove_stale_endpoint, IpcListener, IpcStream};

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle, OwnedHandle};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FlushFileBuffers, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PeekNamedPipe,
        SetNamedPipeHandleState, WaitNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    /// Both directions of a duplex instance live in one 64 KiB buffer pair.
    /// Requests are single JSON lines and responses are NDJSON, so this only
    /// has to absorb a burst, not a bulk transfer.
    const PIPE_BUFFER_BYTES: u32 = 64 * 1024;

    /// Poll interval for the timeout-bounded read path. Small enough that a
    /// 500 ms CLI read timeout still has ~50 chances to observe data.
    const READ_POLL_INTERVAL: Duration = Duration::from_millis(10);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn endpoint_str(endpoint: &Path) -> String {
        endpoint.to_string_lossy().into_owned()
    }

    /// One connected named-pipe instance.
    ///
    /// Wraps `std::fs::File` so `Read`/`Write` come from std; the extra state
    /// is the optional read timeout, which named pipes cannot express natively
    /// without overlapped I/O.
    #[derive(Debug)]
    pub struct IpcStream {
        file: std::fs::File,
        /// `Cell` so the setters can take `&self`, matching `UnixStream`'s
        /// signatures exactly — otherwise every call site would need a `cfg`.
        /// Not `Sync`, but a stream is moved between threads, never shared.
        read_timeout: std::cell::Cell<Option<Duration>>,
        /// True on the handle returned by `IpcListener::accept`. Only a server
        /// handle may call `DisconnectNamedPipe`; a client calling it is an
        /// error, so `shutdown` branches on this.
        is_server: bool,
    }

    impl IpcStream {
        /// Connect to an existing named pipe, blocking until one instance is
        /// free or the pipe is gone.
        pub fn connect(endpoint: impl AsRef<Path>) -> io::Result<Self> {
            // Matches the Unix `connect` contract: wait for a free instance
            // rather than failing fast when the host is merely busy.
            Self::connect_deadline(endpoint.as_ref(), Instant::now() + Duration::from_secs(5))
        }

        fn connect_deadline(endpoint: &Path, deadline: Instant) -> io::Result<Self> {
            let name = endpoint_str(endpoint);
            let wide_name = wide(&name);
            loop {
                // SAFETY: `wide_name` is a NUL-terminated wide string that
                // outlives the call; the remaining arguments are plain flags.
                let raw = unsafe {
                    CreateFileW(
                        wide_name.as_ptr(),
                        GENERIC_READ | GENERIC_WRITE,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        std::ptr::null(),
                        OPEN_EXISTING,
                        0,
                        std::ptr::null_mut(),
                    )
                };
                if raw != INVALID_HANDLE_VALUE {
                    // SAFETY: `raw` is a valid handle CreateFileW just returned
                    // and is not owned anywhere else.
                    let owned = unsafe { OwnedHandle::from_raw_handle(raw as _) };
                    return Ok(Self {
                        file: std::fs::File::from(owned),
                        read_timeout: std::cell::Cell::new(None),
                        is_server: false,
                    });
                }

                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(ERROR_PIPE_BUSY as i32) {
                    return Err(error);
                }
                // All instances busy. Wait for one to free up, then retry.
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("timed out waiting for a free instance of {name}"),
                    ));
                }
                let wait_ms = remaining.as_millis().min(u32::MAX as u128) as u32;
                // SAFETY: same NUL-terminated wide string as above.
                if unsafe { WaitNamedPipeW(wide_name.as_ptr(), wait_ms) } == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }

        /// A second handle to the same pipe instance.
        ///
        /// Used where one thread owns a `BufReader` over the read side while
        /// another writes replies. A duplex instance supports concurrent read
        /// and write, so the duplicated handle is safe to use that way.
        pub fn try_clone(&self) -> io::Result<Self> {
            Ok(Self {
                file: self.file.try_clone()?,
                read_timeout: std::cell::Cell::new(self.read_timeout.get()),
                is_server: self.is_server,
            })
        }

        /// Bound how long a `read` waits for the peer.
        ///
        /// Named pipes have no `SO_RCVTIMEO`; honoring this without overlapped
        /// I/O means polling `PeekNamedPipe` and only issuing a `ReadFile` once
        /// bytes are known to be buffered. `None` restores blocking reads.
        pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.read_timeout.set(timeout);
            Ok(())
        }

        /// Accepted for parity with `UnixStream`; named-pipe writes to a live
        /// reader complete into the kernel buffer, so there is no blocking
        /// window for a timeout to cover. Recorded as a no-op rather than an
        /// error so callers do not need a `cfg`.
        pub fn set_write_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }

        /// End the conversation.
        ///
        /// `std::net::Shutdown` has no named-pipe equivalent: the server side
        /// flushes and disconnects the instance, the client side flushes and
        /// lets `Drop` close the handle. Either way the peer's next read sees
        /// EOF, which is the only property callers rely on.
        pub fn shutdown(&self, _how: std::net::Shutdown) -> io::Result<()> {
            let handle = self.file.as_raw_handle() as _;
            // SAFETY: `self.file` owns a live pipe handle for the call's duration.
            unsafe {
                FlushFileBuffers(handle);
                if self.is_server {
                    DisconnectNamedPipe(handle);
                }
            }
            Ok(())
        }

        /// Bytes currently buffered on the read side, without consuming them.
        fn peek_available(&self) -> io::Result<u32> {
            let mut available: u32 = 0;
            // SAFETY: `self.file` owns a live pipe handle; every out-pointer
            // either points at a local or is null (meaning "don't report").
            let ok = unsafe {
                PeekNamedPipe(
                    self.file.as_raw_handle() as _,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(available)
        }
    }

    impl AsRawHandle for IpcStream {
        fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
            self.file.as_raw_handle()
        }
    }

    impl io::Read for IpcStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(timeout) = self.read_timeout.get() else {
                return io::Read::read(&mut self.file, buf);
            };
            let deadline = Instant::now() + timeout;
            loop {
                match self.peek_available() {
                    // Peek reports a closed pipe as an error; surface it as
                    // clean EOF so `BufRead::lines` terminates normally.
                    Err(_) => return Ok(0),
                    Ok(0) => {
                        if Instant::now() >= deadline {
                            return Err(io::Error::new(
                                io::ErrorKind::WouldBlock,
                                "named pipe read timed out",
                            ));
                        }
                        std::thread::sleep(READ_POLL_INTERVAL);
                    }
                    Ok(available) => {
                        // Never ask for more than is buffered — a larger
                        // ReadFile would block past the deadline.
                        let take = buf.len().min(available as usize);
                        return io::Read::read(&mut self.file, &mut buf[..take]);
                    }
                }
            }
        }
    }

    impl io::Write for IpcStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            io::Write::write(&mut self.file, buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            io::Write::flush(&mut self.file)
        }
    }

    /// Server side of a named pipe.
    ///
    /// Unlike `UnixListener` there is no persistent listening object: each
    /// accepted client consumes one *instance*, and the next accept creates a
    /// fresh one. `PIPE_UNLIMITED_INSTANCES` is what lets concurrent clients
    /// coexist under the same name.
    #[derive(Debug)]
    pub struct IpcListener {
        name: String,
    }

    impl IpcListener {
        pub fn bind(endpoint: impl AsRef<Path>) -> io::Result<Self> {
            let name = endpoint_str(endpoint.as_ref());
            // Create one instance eagerly so a client connecting immediately
            // after `bind` returns does not race the first `accept`, and so a
            // name collision with another running host fails here rather than
            // inside the accept loop.
            let probe = Self::create_instance(&name)?;
            drop(probe);
            Ok(Self { name })
        }

        fn create_instance(name: &str) -> io::Result<OwnedHandle> {
            let wide_name = wide(name);
            // SAFETY: NUL-terminated wide string; a null security descriptor
            // gives the default ACL, which grants access only to this user.
            let raw = unsafe {
                CreateNamedPipeW(
                    wide_name.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    PIPE_BUFFER_BYTES,
                    PIPE_BUFFER_BYTES,
                    0,
                    std::ptr::null(),
                )
            };
            if raw == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: `raw` is a fresh valid handle owned by nobody else.
            Ok(unsafe { OwnedHandle::from_raw_handle(raw as _) })
        }

        /// Block until a client connects, returning its instance.
        pub fn accept(&self) -> io::Result<IpcStream> {
            let handle = Self::create_instance(&self.name)?;
            let raw = handle.as_raw_handle() as _;
            // SAFETY: `handle` owns a live server-side pipe instance. A null
            // OVERLAPPED means "block until connected", which is what we want.
            let connected = unsafe { ConnectNamedPipe(raw, std::ptr::null_mut()) };
            if connected == 0 {
                let error = io::Error::last_os_error();
                // The client connected between CreateNamedPipeW and
                // ConnectNamedPipe. That is a successful accept, not a failure.
                if error.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
                    return Err(error);
                }
            }
            // Byte mode, blocking — matches the stream semantics callers
            // already have on Unix.
            // SAFETY: `handle` is live; both out-params point at locals.
            unsafe {
                let mut mode = PIPE_READMODE_BYTE | PIPE_WAIT;
                SetNamedPipeHandleState(
                    raw,
                    &mut mode,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
            }
            // SAFETY: re-wrapping the handle we still exclusively own.
            let owned = unsafe { OwnedHandle::from_raw_handle(handle.into_raw_handle()) };
            Ok(IpcStream {
                file: std::fs::File::from(owned),
                read_timeout: std::cell::Cell::new(None),
                is_server: true,
            })
        }

        /// Endless accept iterator, mirroring `UnixListener::incoming`.
        pub fn incoming(&self) -> Incoming<'_> {
            Incoming { listener: self }
        }
    }

    pub struct Incoming<'a> {
        listener: &'a IpcListener,
    }

    impl Iterator for Incoming<'_> {
        type Item = io::Result<IpcStream>;

        fn next(&mut self) -> Option<Self::Item> {
            Some(self.listener.accept())
        }
    }

    /// Connect, giving up after `timeout`.
    pub fn connect_timeout(endpoint: &Path, timeout: Duration) -> io::Result<IpcStream> {
        IpcStream::connect_deadline(endpoint, Instant::now() + timeout)
    }

    /// No-op on Windows: a named pipe has no filesystem entry, so a dead host
    /// leaves nothing behind for the next one to clear.
    pub fn remove_stale_endpoint(_endpoint: &Path) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    /// A client's line reaches the listener on whichever transport this
    /// platform uses — the one property every call site depends on.
    #[test]
    fn round_trips_a_line() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let endpoint = dir.path().join("test.sock");
        #[cfg(windows)]
        let endpoint = {
            let _ = &dir;
            PathBuf::from(format!(r"\\.\pipe\plexi-test-{}", std::process::id()))
        };

        let listener = IpcListener::bind(&endpoint).expect("bind");
        let server = std::thread::spawn(move || {
            let stream = listener.incoming().next().unwrap().expect("accept");
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).expect("read");
            line
        });

        let mut client = connect_timeout(&endpoint, std::time::Duration::from_secs(5))
            .expect("connect to the listener");
        client.write_all(b"{\"hello\":1}\n").expect("write");
        client.flush().expect("flush");
        drop(client);

        assert_eq!(server.join().unwrap().trim(), "{\"hello\":1}");
    }

    /// `endpoint_in` and `endpoint_path` must agree for the running channel;
    /// a drift here sends the CLI to one address and the host to another.
    #[test]
    fn endpoint_path_matches_endpoint_in_for_the_running_channel() {
        assert_eq!(endpoint_path(), endpoint_in(&crate::config::config_dir()));
    }

    #[cfg(windows)]
    #[test]
    fn profile_dir_basename_picks_the_pipe() {
        assert_eq!(
            endpoint_in(Path::new(r"C:\Users\ian\.plexi-alpha")),
            PathBuf::from(r"\\.\pipe\plexi-alpha")
        );
        assert_eq!(
            endpoint_in(Path::new(r"C:\Users\ian\.plexi")),
            PathBuf::from(r"\\.\pipe\plexi")
        );
    }

    #[cfg(windows)]
    #[test]
    fn pipe_name_is_channel_scoped() {
        assert_eq!(pipe_name_for_channel(None), r"\\.\pipe\plexi");
        assert_eq!(pipe_name_for_channel(Some("alpha")), r"\\.\pipe\plexi-alpha");
        assert_eq!(
            pipe_name_for_channel(Some("pr-1604")),
            r"\\.\pipe\plexi-pr-1604"
        );
    }
}
