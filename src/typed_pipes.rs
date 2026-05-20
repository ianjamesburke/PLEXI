use std::collections::HashMap;
#[cfg(unix)]
use std::io::Write;
use std::path::PathBuf;
#[cfg(unix)]
extern crate libc;
/// Typed pipe registry for Plexi v3 — binary side channel and JSON metadata pipes.
///
/// Binary pipes use a length-prefixed `u32 BE + payload` framing carried over a
/// platform-native transport:
///
/// * Unix: an AF_UNIX listener backed by `UnixListener` at a path under the
///   private `pipes_dir`.
/// * Windows: a Win32 named pipe at `\\.\pipe\plexi-<uuid>` created with
///   `CreateNamedPipeW` (`PIPE_ACCESS_OUTBOUND | FILE_FLAG_OVERLAPPED`,
///   `PIPE_TYPE_BYTE | PIPE_WAIT`). The pipe name IS the value stored in
///   `BinaryPipeAllocation::socket_path` — apps connect to it via `CreateFileW`.
///   There is no filesystem entry to clean up on close.
///
/// A lock-free ring (ArrayQueue) decouples the write path from the drain thread
/// so the audio callback can enqueue frames without blocking or allocating.
/// JSON pipes are metadata-only registrations; routing is handled by the PGAP wire.
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;

use crossbeam_queue::ArrayQueue;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub enum PipeDirection {
    In,
    Out,
    Duplex,
}

#[derive(Debug)]
pub enum PipeError {
    AlreadyOpen(String),
    NotFound(String),
    BindFailed(String),
    WriteFailed(String),
}

impl std::fmt::Display for PipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipeError::AlreadyOpen(id) => write!(f, "pipe already open: {id}"),
            PipeError::NotFound(id) => write!(f, "pipe not found: {id}"),
            PipeError::BindFailed(msg) => write!(f, "bind failed: {msg}"),
            PipeError::WriteFailed(msg) => write!(f, "write failed: {msg}"),
        }
    }
}

pub struct BinaryPipeAllocation {
    /// Unix socket path the app connects to as a client.
    pub socket_path: String,
}

// ---------------------------------------------------------------------------
// Internal frame buffer capacity
// ---------------------------------------------------------------------------

const DEFAULT_RING_CAPACITY: usize = 32;

// ---------------------------------------------------------------------------
// Internal pipe entries
// ---------------------------------------------------------------------------

struct BinaryPipeEntry {
    #[allow(dead_code)]
    direction: PipeDirection,
    socket_path: String,
    shutdown: Arc<AtomicBool>,
    drain_handle: Option<thread::JoinHandle<()>>,
    /// Frame ring shared with the drain thread. Producers (e.g. the audio
    /// capture callback) clone this `Arc` and push frames; the drain thread
    /// pops and writes them to the socket.
    ring: Arc<ArrayQueue<Vec<u8>>>,
    /// Set by the drain thread before it exits due to a write error (e.g.
    /// Broken pipe). The host polls this flag per-frame via `drain_failed()`
    /// and emits an error event so the app is notified promptly rather than
    /// waiting for the next retry attempt.
    error_flag: Arc<AtomicBool>,
}

struct JsonPipeEntry {
    #[allow(dead_code)]
    direction: PipeDirection,
}

enum PipeEntry {
    Binary(BinaryPipeEntry),
    Json(JsonPipeEntry),
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

pub struct TypedPipeRegistry {
    pipes: HashMap<String, PipeEntry>,
    /// Private directory where binary pipe sockets are created (mode 0700).
    pipes_dir: PathBuf,
}

impl TypedPipeRegistry {
    pub fn new(pipes_dir: PathBuf) -> Self {
        Self {
            pipes: HashMap::new(),
            pipes_dir,
        }
    }

    /// Allocate a binary pipe backed by a unix domain socket.
    ///
    /// The host binds a listener, accepts one client connection (the app), and
    /// starts a drain thread that reads from the lock-free ring and writes
    /// length-prefixed frames to the socket. Returns a `BinaryPipeAllocation`
    /// with the socket path and host-side fd for the caller to hand to the app
    /// via the `PipeOpen` draw command.
    pub fn open_binary(
        &mut self,
        pipe_id: String,
        direction: PipeDirection,
    ) -> Result<BinaryPipeAllocation, PipeError> {
        if self.pipes.contains_key(&pipe_id) {
            return Err(PipeError::AlreadyOpen(pipe_id));
        }

        // Create a private per-user pipes directory with restrictive permissions.
        // Sockets are named by UUID — pipe_id is never embedded in the filesystem path.
        std::fs::create_dir_all(&self.pipes_dir)
            .map_err(|e| PipeError::BindFailed(format!("create pipes dir: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                &self.pipes_dir,
                std::fs::Permissions::from_mode(0o700),
            )
            .map_err(|e| PipeError::BindFailed(format!("secure pipes dir: {e}")))?;
        }

        #[cfg(unix)]
        {
            let socket_name = format!("{}.sock", uuid::Uuid::new_v4());
            let socket_path = self
                .pipes_dir
                .join(&socket_name)
                .to_string_lossy()
                .into_owned();
            log::info!("typed_pipes: opening binary pipe {pipe_id} at {socket_path}");

            let listener = UnixListener::bind(&socket_path)
                .map_err(|e| PipeError::BindFailed(format!("{socket_path}: {e}")))?;
            // Prevent child processes (app subprocesses) from inheriting this socket FD.
            unsafe {
                use std::os::unix::io::AsRawFd;
                libc::fcntl(listener.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
            }
            // Non-blocking so the drain thread's accept loop can observe `shutdown`
            // and exit if the app never connects (e.g. start_capture failed and no
            // PipeOpened was ever sent). Otherwise close() -> join() deadlocks.
            listener
                .set_nonblocking(true)
                .map_err(|e| PipeError::BindFailed(format!("set_nonblocking: {e}")))?;

            let ring: Arc<ArrayQueue<Vec<u8>>> = Arc::new(ArrayQueue::new(DEFAULT_RING_CAPACITY));
            let shutdown = Arc::new(AtomicBool::new(false));
            let error_flag = Arc::new(AtomicBool::new(false));

            let ring_drain = Arc::clone(&ring);
            let shutdown_drain = Arc::clone(&shutdown);
            let error_flag_drain = Arc::clone(&error_flag);
            let socket_path_drain = socket_path.clone();
            let pipe_id_log = pipe_id.clone();

            // Drain thread: blocks waiting for the app to connect, then drains the
            // ring into the socket. Exits when `shutdown` is set and ring is empty.
            let drain_handle = thread::Builder::new()
                .name(format!("pipe-drain-{pipe_id}"))
                .spawn(move || {
                    log::info!("typed_pipes: drain thread started for pipe {pipe_id_log}");
                    // Poll for a client connection. Listener is non-blocking so we
                    // can observe `shutdown` and exit if the app never connects
                    // (e.g. start_capture failed before PipeOpened was sent).
                    let stream = loop {
                        match listener.accept() {
                            Ok((s, _)) => break s,
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                if shutdown_drain.load(Ordering::Acquire) {
                                    let _ = std::fs::remove_file(&socket_path_drain);
                                    return;
                                }
                                thread::sleep(std::time::Duration::from_millis(50));
                            }
                            Err(e) => {
                                log::error!("typed_pipes: accept failed on {socket_path_drain}: {e}");
                                let _ = std::fs::remove_file(&socket_path_drain);
                                return;
                            }
                        }
                    };
                    // Switch to blocking mode for the write loop.
                    let _ = stream.set_nonblocking(false);
                    let mut writer = stream;
                    loop {
                        if let Some(frame) = ring_drain.pop() {
                            if let Err(e) = write_frame(&mut writer, &frame) {
                                log::warn!("typed_pipes: drain write error: {e}");
                                error_flag_drain.store(true, Ordering::Release);
                                break;
                            }
                        } else if shutdown_drain.load(Ordering::Acquire) {
                            break;
                        } else {
                            thread::sleep(std::time::Duration::from_millis(1));
                        }
                    }
                    // Signal end-of-stream.
                    let _ = write_eos(&mut writer);
                    let _ = std::fs::remove_file(&socket_path_drain);
                })
                .map_err(|e| PipeError::BindFailed(format!("thread spawn: {e}")))?;

            let entry = BinaryPipeEntry {
                direction,
                socket_path: socket_path.clone(),
                shutdown,
                drain_handle: Some(drain_handle),
                ring: Arc::clone(&ring),
                error_flag,
            };

            self.pipes.insert(pipe_id.clone(), PipeEntry::Binary(entry));

            Ok(BinaryPipeAllocation { socket_path })
        }

        #[cfg(windows)]
        {
            // Pipe name format: \\.\pipe\plexi-<uuid>. This string IS the
            // `BinaryPipeAllocation.socket_path` — the app connects to it via
            // `CreateFileW(name, GENERIC_READ, 0, null, OPEN_EXISTING, 0, null)`.
            // There is no filesystem entry created on disk.
            let pipe_name = format!("\\\\.\\pipe\\plexi-{}", uuid::Uuid::new_v4());
            log::info!("typed_pipes: opening binary pipe {pipe_id} at {pipe_name}");

            // Wide-string buffer must outlive the CreateNamedPipeW call.
            let wide_name: Vec<u16> = pipe_name.encode_utf16().chain(std::iter::once(0)).collect();

            // SAFETY: `wide_name.as_ptr()` is a valid NUL-terminated UTF-16 pointer.
            // `lpSecurityAttributes` = null requests the default ACL — only the
            // current user can connect under default Windows pipe security.
            let pipe_handle = unsafe {
                windows_sys::Win32::System::Pipes::CreateNamedPipeW(
                    wide_name.as_ptr(),
                    windows_sys::Win32::Storage::FileSystem::PIPE_ACCESS_OUTBOUND
                        | windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED,
                    windows_sys::Win32::System::Pipes::PIPE_TYPE_BYTE
                        | windows_sys::Win32::System::Pipes::PIPE_WAIT,
                    1,            // nMaxInstances: one app per pipe
                    64 * 1024,    // nOutBufferSize: 64 KiB frame buffer
                    64 * 1024,    // nInBufferSize: unused (outbound only) but allocated
                    0,            // nDefaultTimeOut: 0 = default (50ms wait semantics)
                    std::ptr::null(),
                )
            };

            if pipe_handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
                // SAFETY: GetLastError reads thread-local kernel state; always safe.
                let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
                return Err(PipeError::BindFailed(format!(
                    "CreateNamedPipeW({pipe_name}) failed: GetLastError={err}"
                )));
            }

            let ring: Arc<ArrayQueue<Vec<u8>>> = Arc::new(ArrayQueue::new(DEFAULT_RING_CAPACITY));
            let shutdown = Arc::new(AtomicBool::new(false));
            let error_flag = Arc::new(AtomicBool::new(false));

            let ring_drain = Arc::clone(&ring);
            let shutdown_drain = Arc::clone(&shutdown);
            let error_flag_drain = Arc::clone(&error_flag);
            let pipe_id_log = pipe_id.clone();
            let pipe_name_drain = pipe_name.clone();

            // Move the raw HANDLE into the drain thread as a usize. HANDLE is a
            // `*mut c_void` typedef, which Rust treats as !Send. Kernel handles
            // are opaque pointer-sized identifiers safe to transfer between
            // threads — we just need to bypass the auto-trait check. Casting to
            // usize for the move and back inside the thread is the
            // documented-stdlib pattern (see std::os::windows::io::OwnedHandle).
            let pipe_handle_raw = pipe_handle as usize;

            let drain_handle = thread::Builder::new()
                .name(format!("pipe-drain-{pipe_id}"))
                .spawn(move || {
                    let pipe_handle = pipe_handle_raw
                        as windows_sys::Win32::Foundation::HANDLE;
                    log::info!("typed_pipes: drain thread started for pipe {pipe_id_log}");

                    // Create a manual-reset event used as the OVERLAPPED.hEvent.
                    // SAFETY: null security attrs, manual-reset = TRUE, initial = FALSE,
                    // no name — well-defined Win32 calling convention.
                    let event = unsafe {
                        windows_sys::Win32::System::Threading::CreateEventW(
                            std::ptr::null(),
                            1, // bManualReset = TRUE
                            0, // bInitialState = FALSE
                            std::ptr::null(),
                        )
                    };
                    if event == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE
                        || event.is_null()
                    {
                        log::error!(
                            "typed_pipes: CreateEventW failed for pipe {pipe_id_log} at {pipe_name_drain}"
                        );
                        // SAFETY: pipe_handle was returned by CreateNamedPipeW above.
                        unsafe {
                            windows_sys::Win32::Foundation::CloseHandle(pipe_handle);
                        }
                        error_flag_drain.store(true, Ordering::Release);
                        return;
                    }

                    let mut overlapped = windows_sys::Win32::System::IO::OVERLAPPED::default();
                    overlapped.hEvent = event;

                    // Accept loop. Overlapped ConnectNamedPipe returns immediately;
                    // we poll the event with WaitForSingleObject so we can observe
                    // the shutdown flag and exit cleanly if the app never connects.
                    let connected = {
                        // SAFETY: pipe was created OVERLAPPED; overlapped is valid for
                        // the lifetime of the wait. We never move `overlapped` until
                        // the I/O has been observed to complete or has been cancelled.
                        let rc = unsafe {
                            windows_sys::Win32::System::Pipes::ConnectNamedPipe(
                                pipe_handle,
                                &mut overlapped,
                            )
                        };
                        if rc != 0 {
                            // Synchronous success path (rare for OVERLAPPED pipes,
                            // but documented possibility).
                            true
                        } else {
                            // SAFETY: GetLastError reads thread-local kernel state.
                            let err = unsafe {
                                windows_sys::Win32::Foundation::GetLastError()
                            };
                            if err == windows_sys::Win32::Foundation::ERROR_PIPE_CONNECTED {
                                // Client connected between CreateNamedPipeW and
                                // ConnectNamedPipe — this is success, not failure.
                                true
                            } else if err == windows_sys::Win32::Foundation::ERROR_IO_PENDING {
                                // Normal async path: poll until connected or shutdown.
                                let mut connected = false;
                                loop {
                                    // SAFETY: `event` is a valid manual-reset event handle.
                                    let wait = unsafe {
                                        windows_sys::Win32::System::Threading::WaitForSingleObject(
                                            event, 50,
                                        )
                                    };
                                    if wait == windows_sys::Win32::Foundation::WAIT_OBJECT_0 {
                                        connected = true;
                                        break;
                                    }
                                    if wait == windows_sys::Win32::Foundation::WAIT_TIMEOUT {
                                        if shutdown_drain.load(Ordering::Acquire) {
                                            // Cancel pending I/O before we abandon
                                            // the OVERLAPPED. SAFETY: same pipe_handle,
                                            // matching overlapped struct still live.
                                            unsafe {
                                                windows_sys::Win32::System::IO::CancelIoEx(
                                                    pipe_handle,
                                                    &overlapped,
                                                );
                                            }
                                            break;
                                        }
                                        continue;
                                    }
                                    // Any other wait result (WAIT_FAILED, abandoned)
                                    // is a hard error.
                                    log::error!(
                                        "typed_pipes: WaitForSingleObject returned {wait} for pipe {pipe_id_log}"
                                    );
                                    break;
                                }
                                connected
                            } else {
                                log::error!(
                                    "typed_pipes: ConnectNamedPipe failed for {pipe_id_log}: GetLastError={err}"
                                );
                                false
                            }
                        }
                    };

                    if !connected {
                        // SAFETY: handles were created above and are still owned by us.
                        unsafe {
                            windows_sys::Win32::Foundation::CloseHandle(event);
                            windows_sys::Win32::Foundation::CloseHandle(pipe_handle);
                        }
                        if !shutdown_drain.load(Ordering::Acquire) {
                            // Connect failed for a reason other than clean shutdown.
                            error_flag_drain.store(true, Ordering::Release);
                        }
                        return;
                    }

                    // Drain loop. We use synchronous WriteFile (lpOverlapped = null)
                    // even though the pipe was created OVERLAPPED — this is allowed
                    // and the system serializes per-call. Frames are small (<= 64 KiB)
                    // and we already decoupled producers via the ring.
                    let mut write_err: Option<u32> = None;
                    loop {
                        if let Some(frame) = ring_drain.pop() {
                            if let Err(e) = win32_write_frame(pipe_handle, &frame) {
                                log::warn!(
                                    "typed_pipes: drain write error on {pipe_id_log}: GetLastError={e}"
                                );
                                write_err = Some(e);
                                break;
                            }
                        } else if shutdown_drain.load(Ordering::Acquire) {
                            break;
                        } else {
                            thread::sleep(std::time::Duration::from_millis(1));
                        }
                    }

                    if write_err.is_some() {
                        // Any write error (ERROR_BROKEN_PIPE / ERROR_NO_DATA
                        // for peer-gone, or anything else) surfaces via the
                        // same error_flag path the Unix branch uses.
                        error_flag_drain.store(true, Ordering::Release);
                    } else {
                        // Clean shutdown: send the length-0 EOS sentinel so the
                        // client knows the host is done. Ignore write errors here —
                        // the peer may already be gone.
                        let _ = win32_write_frame(pipe_handle, &[]);
                    }

                    // SAFETY: we own both handles; close them on the way out.
                    unsafe {
                        windows_sys::Win32::Foundation::CloseHandle(event);
                        windows_sys::Win32::Foundation::CloseHandle(pipe_handle);
                    }
                })
                .map_err(|e| PipeError::BindFailed(format!("thread spawn: {e}")))?;

            let entry = BinaryPipeEntry {
                direction,
                socket_path: pipe_name.clone(),
                shutdown,
                drain_handle: Some(drain_handle),
                ring: Arc::clone(&ring),
                error_flag,
            };

            self.pipes.insert(pipe_id.clone(), PipeEntry::Binary(entry));

            return Ok(BinaryPipeAllocation { socket_path: pipe_name });
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = (pipe_id, direction);
            log::warn!("typed_pipes: open_binary requested on an unsupported platform");
            Err(PipeError::BindFailed(
                "binary pipes are not supported on this platform".to_owned(),
            ))
        }
    }

    /// Register a JSON pipe. No socket — routing is handled by the PGAP wire.
    pub fn open_json(
        &mut self,
        pipe_id: String,
        direction: PipeDirection,
    ) -> Result<(), PipeError> {
        if self.pipes.contains_key(&pipe_id) {
            return Err(PipeError::AlreadyOpen(pipe_id));
        }
        self.pipes
            .insert(pipe_id, PipeEntry::Json(JsonPipeEntry { direction }));
        Ok(())
    }

    /// Close a pipe by id. Signals the drain thread to flush and exit (binary),
    /// then joins it. On Unix this also removes the socket file; on Windows the
    /// named pipe has no filesystem entry — the kernel reclaims the name as
    /// soon as the last handle is closed by the drain thread.
    pub fn close(&mut self, pipe_id: &str) {
        if let Some(PipeEntry::Binary(mut b)) = self.pipes.remove(pipe_id) {
            b.shutdown.store(true, Ordering::Release);
            if let Some(handle) = b.drain_handle.take() {
                let _ = handle.join();
            }
            #[cfg(unix)]
            {
                let _ = std::fs::remove_file(&b.socket_path);
            }
            #[cfg(not(unix))]
            {
                // No filesystem entry on Windows; pipe handle closed by drain thread.
                let _ = &b.socket_path;
            }
        }
    }

    /// Route a JSON payload on a JSON pipe (host side helper).
    pub fn send_json(
        &mut self,
        pipe_id: &str,
        _payload: serde_json::Value,
    ) -> Result<(), PipeError> {
        match self.pipes.get(pipe_id) {
            Some(PipeEntry::Json(_)) => Ok(()),
            Some(PipeEntry::Binary(_)) => {
                Err(PipeError::WriteFailed("pipe is binary mode".to_owned()))
            }
            None => Err(PipeError::NotFound(pipe_id.to_owned())),
        }
    }

    /// Borrow a clone of the binary pipe's frame ring, suitable for handing
    /// to a real-time producer (e.g. the cpal capture callback). Returns
    /// `None` if the pipe doesn't exist or is JSON-mode.
    ///
    /// The producer pushes raw payload bytes via `Arc<ArrayQueue<Vec<u8>>>::push`;
    /// the drain thread length-prefixes and writes them to the unix socket.
    /// On a full ring `push` returns `Err(rejected)` so producers should not
    /// block — drop the frame and (optionally) emit a `PipeOverrun` event.
    pub fn binary_ring(&self, pipe_id: &str) -> Option<Arc<ArrayQueue<Vec<u8>>>> {
        match self.pipes.get(pipe_id) {
            Some(PipeEntry::Binary(b)) => Some(Arc::clone(&b.ring)),
            _ => None,
        }
    }

    /// Returns true if the drain thread for `pipe_id` exited due to a write
    /// error (e.g. Broken pipe). The host polls this per-frame to detect pipe
    /// failures promptly rather than waiting for the app to retry.
    pub fn drain_failed(&self, pipe_id: &str) -> bool {
        match self.pipes.get(pipe_id) {
            Some(PipeEntry::Binary(b)) => b.error_flag.load(Ordering::Acquire),
            _ => false,
        }
    }

    /// Returns true if the pipe with `pipe_id` has direction `In` or `Duplex`
    /// (i.e. it can receive messages). Used by peer routing to decide which
    /// panes should receive a `PipeSend`.
    pub fn has_reader(&self, pipe_id: &str) -> bool {
        match self.pipes.get(pipe_id) {
            Some(PipeEntry::Json(j)) => {
                matches!(j.direction, PipeDirection::In | PipeDirection::Duplex)
            }
            Some(PipeEntry::Binary(b)) => {
                matches!(b.direction, PipeDirection::In | PipeDirection::Duplex)
            }
            None => false,
        }
    }

}

impl Drop for TypedPipeRegistry {
    fn drop(&mut self) {
        let ids: Vec<String> = self.pipes.keys().cloned().collect();
        for id in ids {
            self.close(&id);
        }
    }
}

// ---------------------------------------------------------------------------
// Frame I/O helpers (Unix-only — used by the AF_UNIX drain thread)
// ---------------------------------------------------------------------------

/// Write a length-prefixed frame: `u32 BE length || payload`.
#[cfg(unix)]
fn write_frame(writer: &mut impl Write, payload: &[u8]) -> std::io::Result<()> {
    let len = payload.len() as u32;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(payload)?;
    Ok(())
}

/// Write a length-0 EOS sentinel.
#[cfg(unix)]
fn write_eos(writer: &mut impl Write) -> std::io::Result<()> {
    writer.write_all(&0u32.to_be_bytes())
}

/// Write a length-prefixed frame to a Win32 named-pipe HANDLE.
///
/// Mirrors the Unix `write_frame`: emits `u32 BE length || payload` (length-0
/// for the EOS sentinel). On failure returns the raw `GetLastError()` value so
/// the caller can distinguish `ERROR_BROKEN_PIPE` / `ERROR_NO_DATA` (peer gone)
/// from other failures.
///
/// Uses synchronous `WriteFile` (lpOverlapped = null). The pipe was created
/// OVERLAPPED so the accept loop could poll without blocking, but per-frame
/// writes serialize cleanly without overlapped state.
#[cfg(windows)]
fn win32_write_frame(
    handle: windows_sys::Win32::Foundation::HANDLE,
    payload: &[u8],
) -> Result<(), u32> {
    let len_be = (payload.len() as u32).to_be_bytes();
    win32_write_all(handle, &len_be)?;
    if !payload.is_empty() {
        win32_write_all(handle, payload)?;
    }
    Ok(())
}

/// Loop `WriteFile` until every byte of `buf` has been written. Short writes
/// on byte-mode pipes are rare but possible under buffer pressure; mirroring
/// `write_all` is the correct way to handle them.
#[cfg(windows)]
fn win32_write_all(
    handle: windows_sys::Win32::Foundation::HANDLE,
    buf: &[u8],
) -> Result<(), u32> {
    let mut offset = 0usize;
    while offset < buf.len() {
        let mut written: u32 = 0;
        let remaining = (buf.len() - offset).min(u32::MAX as usize) as u32;
        // SAFETY: `buf` is a valid slice for `remaining` bytes from `offset`.
        // `written` is a valid mut u32. `handle` is a kernel pipe HANDLE owned
        // by the calling thread. `lpOverlapped` = null requests synchronous I/O.
        let rc = unsafe {
            windows_sys::Win32::Storage::FileSystem::WriteFile(
                handle,
                buf.as_ptr().add(offset),
                remaining,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        if rc == 0 {
            // SAFETY: GetLastError reads thread-local kernel state.
            let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            return Err(err);
        }
        if written == 0 {
            // Defensive: WriteFile reported success but wrote nothing. Treat as
            // a broken pipe rather than spinning forever.
            return Err(windows_sys::Win32::Foundation::ERROR_BROKEN_PIPE);
        }
        offset += written as usize;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// The Unix tests exercise the AF_UNIX drain path directly via
// `std::os::unix::net::UnixStream`. The Windows tests below exercise the
// `\\.\pipe\plexi-<uuid>` drain path via `CreateFileW`.
#[cfg(all(test, unix))]
mod tests {
    //! Unit tests for the typed-pipe registry primitives that the directed
    //! inter-agent pipe routing (#286) builds on.
    use super::*;

    fn test_registry() -> (TypedPipeRegistry, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let reg = TypedPipeRegistry::new(dir.path().to_path_buf());
        (reg, dir)
    }

    #[test]
    fn directed_pipe_routes_to_target_pane_only() {
        // Two panes' registries are independent. The sender pane registers
        // a JSON duplex pipe under id "x"; the target pane registers the
        // same id under its own registry. Each registry's `has_reader`
        // reflects only its own subscribers — they do NOT collide.
        //
        // This is the substrate for `directed_pipes` scoping in
        // `app/mod.rs`: the host maintains a `(sender, target)` pair keyed
        // on pipe_id, and `DeliverPipeMessage` consults that pair to route
        // ONLY to the target — never broadcasting to other panes that
        // coincidentally opened the same id. The registry's job is simply
        // to track per-pane subscription state; the scoping is upstream.
        let (mut sender_reg, _d1) = test_registry();
        let (mut target_reg, _d2) = test_registry();

        sender_reg
            .open_json("coord-to-worker".to_string(), PipeDirection::Duplex)
            .expect("sender opens JSON pipe");
        target_reg
            .open_json("coord-to-worker".to_string(), PipeDirection::Duplex)
            .expect("target opens same id on its independent registry");

        assert!(
            sender_reg.has_reader("coord-to-worker"),
            "sender side has_reader true (duplex)"
        );
        assert!(
            target_reg.has_reader("coord-to-worker"),
            "target side has_reader true (duplex)"
        );

        // A third bystander registry without the pipe must NOT read.
        let (bystander_reg, _d3) = test_registry();
        assert!(
            !bystander_reg.has_reader("coord-to-worker"),
            "bystander pane never opted in — has_reader must be false"
        );
    }

    #[test]
    fn drain_failed_set_after_broken_pipe() {
        // Open a binary pipe, connect a client, then immediately drop it to
        // simulate the app-side socket closing unexpectedly (Broken pipe).
        // After the drain thread attempts a write and fails, drain_failed()
        // must return true.
        let (mut reg, _dir) = test_registry();
        let alloc = reg
            .open_binary("test-broken-pipe".to_string(), PipeDirection::In)
            .expect("open_binary");

        // Connect and immediately close to give the drain thread a peer that
        // will produce Broken pipe / Connection reset on the first write.
        {
            let _client =
                std::os::unix::net::UnixStream::connect(&alloc.socket_path).expect("connect");
            // `_client` drops here, closing the far end.
        }

        // Give the drain thread time to accept the connection.
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Push a frame — drain thread will attempt to write to the closed socket.
        let ring = reg.binary_ring("test-broken-pipe").expect("ring");
        let _ = ring.push(vec![1, 2, 3]);

        // Give the drain thread time to attempt the write and set error_flag.
        std::thread::sleep(std::time::Duration::from_millis(200));

        assert!(
            reg.drain_failed("test-broken-pipe"),
            "drain_failed should be true after Broken pipe"
        );
    }

    #[test]
    fn drain_failed_false_for_healthy_pipe() {
        let (mut reg, _dir) = test_registry();
        reg.open_binary("test-healthy".to_string(), PipeDirection::In)
            .expect("open_binary");
        assert!(
            !reg.drain_failed("test-healthy"),
            "drain_failed should be false before any write error"
        );
        assert!(
            !reg.drain_failed("nonexistent"),
            "drain_failed should be false for unknown pipe"
        );
    }

    #[test]
    fn open_json_rejects_duplicate_pipe_id_on_same_registry() {
        // A single registry must reject opening the same pipe id twice —
        // this is what the host's `register_directed_pipe_on_target`
        // helper has to handle gracefully when the target pane has
        // independently opened the pipe (treats AlreadyOpen as success).
        let (mut reg, _dir) = test_registry();
        reg.open_json("dup".to_string(), PipeDirection::Duplex).unwrap();
        let err = reg
            .open_json("dup".to_string(), PipeDirection::Duplex)
            .unwrap_err();
        assert!(matches!(err, PipeError::AlreadyOpen(_)));
    }

    #[test]
    fn malicious_pipe_id_not_in_socket_path() {
        // App-controlled pipe_id values with path traversal sequences, slashes,
        // and shell metacharacters must NOT appear in the generated socket path.
        let (mut reg, dir) = test_registry();
        let long_id = "a".repeat(300);
        let malicious_ids = [
            "../../etc/passwd",
            "../secret",
            "foo/bar/baz",
            "id;rm -rf /",
            long_id.as_str(),
        ];
        for id in malicious_ids {
            let alloc = reg
                .open_binary(id.to_string(), PipeDirection::In)
                .expect("open_binary should succeed regardless of pipe_id content");
            assert!(
                !alloc.socket_path.contains(id),
                "socket path must not contain raw pipe_id {:?}: got {}",
                id,
                alloc.socket_path
            );
            // Socket must be inside the private pipes_dir, not /tmp.
            assert!(
                alloc.socket_path.starts_with(dir.path().to_str().unwrap()),
                "socket path must be inside pipes_dir: got {}",
                alloc.socket_path
            );
            reg.close(id);
        }
    }

    #[test]
    fn socket_allocated_inside_private_pipes_dir() {
        let (mut reg, dir) = test_registry();
        let alloc = reg
            .open_binary("normal-id".to_string(), PipeDirection::In)
            .expect("open_binary");
        assert!(
            alloc
                .socket_path
                .starts_with(dir.path().to_str().unwrap()),
            "socket must be inside pipes_dir, not in /tmp or elsewhere: got {}",
            alloc.socket_path
        );
        assert!(
            alloc.socket_path.ends_with(".sock"),
            "socket path must end with .sock: got {}",
            alloc.socket_path
        );
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    //! Windows transport tests — exercise the Win32 named-pipe drain path.
    //! Mirrors the Unix `drain_failed_*` coverage and adds an end-to-end
    //! round trip via `CreateFileW`.
    use super::*;

    fn test_registry() -> (TypedPipeRegistry, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let reg = TypedPipeRegistry::new(dir.path().to_path_buf());
        (reg, dir)
    }

    fn pipe_name_to_wide(name: &str) -> Vec<u16> {
        name.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[test]
    fn pipe_name_uses_kernel_namespace() {
        let (mut reg, _dir) = test_registry();
        let alloc = reg
            .open_binary("normal-id".to_string(), PipeDirection::In)
            .expect("open_binary");
        assert!(
            alloc.socket_path.starts_with(r"\\.\pipe\plexi-"),
            "Windows pipe name must live in the kernel namespace: got {}",
            alloc.socket_path
        );
    }

    #[test]
    fn end_to_end_frame_round_trip() {
        // Open a binary pipe, connect a client via CreateFileW from another
        // thread, push a frame, and verify the client sees `u32 BE length`
        // followed by the payload bytes.
        let (mut reg, _dir) = test_registry();
        let alloc = reg
            .open_binary("test-roundtrip".to_string(), PipeDirection::In)
            .expect("open_binary");

        let pipe_name = alloc.socket_path.clone();
        let client_thread = std::thread::spawn(move || {
            // Give the host drain thread a moment to enter ConnectNamedPipe.
            // The pipe was created OVERLAPPED so this isn't strictly required
            // (ERROR_PIPE_CONNECTED handles the race), but it avoids a
            // tight-loop retry on the client.
            std::thread::sleep(std::time::Duration::from_millis(50));

            let wide = pipe_name_to_wide(&pipe_name);
            // SAFETY: wide is NUL-terminated UTF-16; null security attrs;
            // OPEN_EXISTING because the server (us) already created it.
            let client = unsafe {
                windows_sys::Win32::Storage::FileSystem::CreateFileW(
                    wide.as_ptr(),
                    windows_sys::Win32::Foundation::GENERIC_READ,
                    0,
                    std::ptr::null(),
                    windows_sys::Win32::Storage::FileSystem::OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            assert_ne!(
                client,
                windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
                "client CreateFileW failed: GetLastError={}",
                // SAFETY: thread-local kernel state.
                unsafe { windows_sys::Win32::Foundation::GetLastError() }
            );

            // Read 4-byte BE length prefix.
            let mut len_buf = [0u8; 4];
            let mut total = 0usize;
            while total < len_buf.len() {
                let mut read: u32 = 0;
                // SAFETY: client handle live, len_buf valid for writes.
                let rc = unsafe {
                    windows_sys::Win32::Storage::FileSystem::ReadFile(
                        client,
                        len_buf.as_mut_ptr().add(total),
                        (len_buf.len() - total) as u32,
                        &mut read,
                        std::ptr::null_mut(),
                    )
                };
                assert_ne!(rc, 0, "ReadFile (length prefix) failed");
                assert!(read > 0, "ReadFile (length prefix) returned 0 bytes");
                total += read as usize;
            }
            let payload_len = u32::from_be_bytes(len_buf) as usize;

            // Read the payload.
            let mut payload = vec![0u8; payload_len];
            let mut total = 0usize;
            while total < payload_len {
                let mut read: u32 = 0;
                // SAFETY: payload valid for writes through payload_len.
                let rc = unsafe {
                    windows_sys::Win32::Storage::FileSystem::ReadFile(
                        client,
                        payload.as_mut_ptr().add(total),
                        (payload_len - total) as u32,
                        &mut read,
                        std::ptr::null_mut(),
                    )
                };
                assert_ne!(rc, 0, "ReadFile (payload) failed");
                assert!(read > 0, "ReadFile (payload) returned 0 bytes");
                total += read as usize;
            }

            // SAFETY: we own the client handle.
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(client);
            }

            payload
        });

        // Producer side: push a frame into the ring.
        let ring = reg.binary_ring("test-roundtrip").expect("ring");
        let frame: Vec<u8> = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x42];
        // Ring may briefly be unavailable if the drain thread is still
        // initialising; spin a few times.
        let mut attempt = 0;
        let mut last = Some(frame.clone());
        while let Some(f) = last.take() {
            match ring.push(f) {
                Ok(()) => break,
                Err(f) => {
                    last = Some(f);
                    attempt += 1;
                    if attempt > 100 {
                        panic!("ring full after 100 retries");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }

        let received = client_thread
            .join()
            .expect("client thread should complete");
        assert_eq!(received, vec![0xDE, 0xAD, 0xBE, 0xEF, 0x42]);

        // Clean shutdown: close() signals the drain thread to exit and joins it.
        reg.close("test-roundtrip");
    }

    #[test]
    fn drain_failed_false_for_healthy_pipe() {
        let (mut reg, _dir) = test_registry();
        reg.open_binary("test-healthy".to_string(), PipeDirection::In)
            .expect("open_binary");
        assert!(
            !reg.drain_failed("test-healthy"),
            "drain_failed should be false before any write error"
        );
        assert!(
            !reg.drain_failed("nonexistent"),
            "drain_failed should be false for unknown pipe"
        );
    }
}
