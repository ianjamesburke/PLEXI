use std::collections::HashMap;
use std::io::Write;
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
    #[allow(dead_code)]
    direction: PipeDirection,
    socket_path: String,
    shutdown: Arc<AtomicBool>,
    drain_handle: Option<thread::JoinHandle<()>>,
    /// Frame ring shared with the drain thread. Producers (e.g. the audio
    /// capture callback) clone this `Arc` and push frames; the drain thread
    /// pops and writes them to the socket.
    ring: Arc<ArrayQueue<Vec<u8>>>,
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
}

impl TypedPipeRegistry {
    pub fn new() -> Self {
        Self {
            pipes: HashMap::new(),
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

        let rand_suffix = {
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        };
        let socket_path = format!("/tmp/plexi-pipe-{rand_suffix}-{pipe_id}.sock");

        // Remove stale socket file if present.
        let _ = std::fs::remove_file(&socket_path);

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

        let ring_drain = Arc::clone(&ring);
        let shutdown_drain = Arc::clone(&shutdown);
        let socket_path_drain = socket_path.clone();

        // Drain thread: blocks waiting for the app to connect, then drains the
        // ring into the socket. Exits when `shutdown` is set and ring is empty.
        let drain_handle = thread::Builder::new()
            .name(format!("pipe-drain-{pipe_id}"))
            .spawn(move || {
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
                            break;
                        }
                    } else if shutdown_drain.load(Ordering::Acquire) {
                        break;
                    } else {
                        // Brief yield to avoid spinning at 100% CPU.
                        thread::yield_now();
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
        let mut sender_reg = TypedPipeRegistry::new();
        let mut target_reg = TypedPipeRegistry::new();

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
        let bystander_reg = TypedPipeRegistry::new();
        assert!(
            !bystander_reg.has_reader("coord-to-worker"),
            "bystander pane never opted in — has_reader must be false"
        );
    }

    #[test]
    fn open_json_rejects_duplicate_pipe_id_on_same_registry() {
        // A single registry must reject opening the same pipe id twice —
        // this is what the host's `register_directed_pipe_on_target`
        // helper has to handle gracefully when the target pane has
        // independently opened the pipe (treats AlreadyOpen as success).
        let mut reg = TypedPipeRegistry::new();
        reg.open_json("dup".to_string(), PipeDirection::Duplex).unwrap();
        let err = reg
            .open_json("dup".to_string(), PipeDirection::Duplex)
            .unwrap_err();
        assert!(matches!(err, PipeError::AlreadyOpen(_)));
    }
}
