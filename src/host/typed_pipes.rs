use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
extern crate libc;
/// Typed pipe registry for Plexi v3 — binary side channel and JSON metadata pipes.
///
/// Binary pipes use unix domain sockets with u32-BE length-prefixed frames.
/// A lock-free ring (ArrayQueue) decouples the write path from the socket drain
/// thread so the audio callback can enqueue frames without blocking or allocating.
/// JSON pipes are metadata-only registrations; routing is handled by the PGAP wire.
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
            std::fs::set_permissions(&self.pipes_dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| PipeError::BindFailed(format!("secure pipes dir: {e}")))?;
        }
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
    /// then joins it. Cleans up the socket file.
    pub fn close(&mut self, pipe_id: &str) {
        if let Some(PipeEntry::Binary(mut b)) = self.pipes.remove(pipe_id) {
            b.shutdown.store(true, Ordering::Release);
            if let Some(handle) = b.drain_handle.take() {
                let _ = handle.join();
            }
            let _ = std::fs::remove_file(&b.socket_path);
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
// Frame I/O helpers
// ---------------------------------------------------------------------------

/// Write a length-prefixed frame: `u32 BE length || payload`.
fn write_frame(writer: &mut impl Write, payload: &[u8]) -> std::io::Result<()> {
    let len = payload.len() as u32;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(payload)?;
    Ok(())
}

/// Write a length-0 EOS sentinel.
fn write_eos(writer: &mut impl Write) -> std::io::Result<()> {
    writer.write_all(&0u32.to_be_bytes())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
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
        reg.open_json("dup".to_string(), PipeDirection::Duplex)
            .unwrap();
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
            alloc.socket_path.starts_with(dir.path().to_str().unwrap()),
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
