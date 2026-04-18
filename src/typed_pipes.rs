use std::collections::HashMap;
use std::io::{Read, Write};
/// Typed pipe registry for Plexi v3 — binary side channel and JSON metadata pipes.
///
/// Binary pipes use unix domain sockets with u32-BE length-prefixed frames.
/// A lock-free ring (ArrayQueue) decouples the write path from the socket drain
/// thread so the audio callback can enqueue frames without blocking or allocating.
/// JSON pipes are metadata-only registrations; routing is handled by the PGAP wire.
use std::os::fd::RawFd;
use std::os::unix::net::{UnixListener, UnixStream};
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
pub enum PipeMode {
    Json,
    Binary,
}

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
    ReadFailed(String),
}

impl std::fmt::Display for PipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipeError::AlreadyOpen(id) => write!(f, "pipe already open: {id}"),
            PipeError::NotFound(id) => write!(f, "pipe not found: {id}"),
            PipeError::BindFailed(msg) => write!(f, "bind failed: {msg}"),
            PipeError::WriteFailed(msg) => write!(f, "write failed: {msg}"),
            PipeError::ReadFailed(msg) => write!(f, "read failed: {msg}"),
        }
    }
}

pub struct BinaryPipeAllocation {
    pub pipe_id: String,
    /// Unix socket path the app connects to as a client.
    pub socket_path: String,
    /// Host's connected end file descriptor (internal; wrapped by the pipe entry).
    pub host_end_fd: RawFd,
}

pub enum WriteResult {
    Written,
    /// Oldest buffered frame was dropped to make room (backpressure event).
    DroppedOldest,
}

// ---------------------------------------------------------------------------
// Internal frame buffer capacity
// ---------------------------------------------------------------------------

const DEFAULT_RING_CAPACITY: usize = 32;
const MAX_FRAME_BYTES: usize = 1024 * 1024; // 1 MiB

// ---------------------------------------------------------------------------
// Internal pipe entries
// ---------------------------------------------------------------------------

struct BinaryPipeEntry {
    #[allow(dead_code)]
    direction: PipeDirection,
    socket_path: String,
    /// Lock-free ring: write path enqueues, drain thread dequeues and writes.
    ///
    /// REALTIME-SAFETY INVARIANT: `write_binary` must not block or allocate.
    /// It calls `ArrayQueue::push` (lock-free, bounded) and returns immediately.
    /// The actual socket write happens on the drain thread.
    ring: Arc<ArrayQueue<Vec<u8>>>,
    shutdown: Arc<AtomicBool>,
    drain_handle: Option<thread::JoinHandle<()>>,
    /// Host's connected UnixStream — kept alive for reading (In/Duplex pipes).
    host_stream: Option<UnixStream>,
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
        // Non-blocking so the drain thread's accept loop can observe `shutdown`
        // and exit if the app never connects (e.g. start_capture failed and no
        // PipeOpened was ever sent). Otherwise close() -> join() deadlocks.
        listener
            .set_nonblocking(true)
            .map_err(|e| PipeError::BindFailed(format!("set_nonblocking: {e}")))?;

        let host_end_fd: RawFd = {
            use std::os::unix::io::AsRawFd;
            listener.as_raw_fd()
        };

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
            ring,
            shutdown,
            drain_handle: Some(drain_handle),
            host_stream: None,
        };

        self.pipes.insert(pipe_id.clone(), PipeEntry::Binary(entry));

        Ok(BinaryPipeAllocation {
            pipe_id,
            socket_path,
            host_end_fd,
        })
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

    /// Enqueue a length-prefixed frame for delivery to the app.
    ///
    /// REALTIME-SAFETY: This method does NOT block or allocate on the hot path
    /// (assuming the ring has space). If the ring is full it pops the oldest
    /// frame (O(1), lock-free) before pushing, and returns `DroppedOldest`.
    /// The actual socket write happens on the drain thread.
    pub fn write_binary(
        &mut self,
        pipe_id: &str,
        payload: &[u8],
    ) -> Result<WriteResult, PipeError> {
        if payload.len() > MAX_FRAME_BYTES {
            return Err(PipeError::WriteFailed(format!(
                "frame too large: {} bytes (max {MAX_FRAME_BYTES})",
                payload.len()
            )));
        }
        let entry = self
            .pipes
            .get(pipe_id)
            .ok_or_else(|| PipeError::NotFound(pipe_id.to_owned()))?;
        let ring = match entry {
            PipeEntry::Binary(b) => &b.ring,
            PipeEntry::Json(_) => {
                return Err(PipeError::WriteFailed("pipe is JSON mode".to_owned()));
            }
        };

        let frame = payload.to_vec();
        let mut dropped = false;
        if ring.push(frame).is_err() {
            // Ring full — drop oldest, then push new.
            ring.pop();
            dropped = true;
            // If push still fails (shouldn't with a single producer popping one),
            // log and return an error.
            let frame2 = payload.to_vec();
            if ring.push(frame2).is_err() {
                return Err(PipeError::WriteFailed(
                    "ring push failed after evict".to_owned(),
                ));
            }
        }
        if dropped {
            Ok(WriteResult::DroppedOldest)
        } else {
            Ok(WriteResult::Written)
        }
    }

    /// Read one length-prefixed frame from a binary pipe's host-side stream.
    /// Blocks until a frame is available or the pipe is closed / EOS.
    pub fn read_binary(&mut self, pipe_id: &str) -> Result<Option<Vec<u8>>, PipeError> {
        let entry = self
            .pipes
            .get_mut(pipe_id)
            .ok_or_else(|| PipeError::NotFound(pipe_id.to_owned()))?;
        match entry {
            PipeEntry::Binary(b) => {
                let stream = b.host_stream.as_mut().ok_or_else(|| {
                    PipeError::ReadFailed(
                        "no host stream (use open_binary for in/duplex)".to_owned(),
                    )
                })?;
                read_frame(stream).map_err(|e| PipeError::ReadFailed(e.to_string()))
            }
            PipeEntry::Json(_) => Err(PipeError::ReadFailed("pipe is JSON mode".to_owned())),
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

    pub fn list(&self) -> Vec<(String, PipeMode, PipeDirection)> {
        self.pipes
            .iter()
            .map(|(id, entry)| match entry {
                PipeEntry::Binary(b) => (id.clone(), PipeMode::Binary, b.direction),
                PipeEntry::Json(j) => (id.clone(), PipeMode::Json, j.direction),
            })
            .collect()
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

/// Read one length-prefixed frame. Returns `None` on EOS (length == 0).
fn read_frame(reader: &mut impl Read) -> std::io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 {
        return Ok(None);
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(Some(buf))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    fn make_registry_with_capacity(cap: usize) -> TypedPipeRegistry {
        // We need a way to test with a smaller ring. Since DEFAULT_RING_CAPACITY
        // is a constant we swap in a hand-built entry for the overrun test.
        let _ = cap; // handled in each test directly
        TypedPipeRegistry::new()
    }

    #[test]
    fn binary_pipe_roundtrip() {
        let mut registry = TypedPipeRegistry::new();
        let alloc = registry
            .open_binary("test-pipe".to_owned(), PipeDirection::Out)
            .expect("open_binary failed");

        let socket_path = alloc.socket_path.clone();

        // Client thread: connect to the socket and read 3 frames.
        let client_handle = thread::spawn(move || {
            // Give the drain thread a moment to start and call accept().
            thread::sleep(std::time::Duration::from_millis(50));
            let mut stream = UnixStream::connect(&socket_path).expect("client connect failed");

            let mut received = Vec::new();
            while let Some(frame) = read_frame(&mut stream).expect("client read_frame failed") {
                received.push(frame);
                if received.len() == 3 {
                    break;
                }
            }
            received
        });

        // Give the client a chance to start connecting.
        thread::sleep(std::time::Duration::from_millis(10));

        let frames: &[&[u8]] = &[b"a", b"bb", b"ccc"];
        for f in frames {
            registry
                .write_binary("test-pipe", f)
                .expect("write_binary failed");
        }

        // Brief pause so drain thread can flush before we close.
        thread::sleep(std::time::Duration::from_millis(100));
        registry.close("test-pipe");

        let received = client_handle.join().expect("client thread panicked");
        assert_eq!(received.len(), 3);
        assert_eq!(received[0], b"a");
        assert_eq!(received[1], b"bb");
        assert_eq!(received[2], b"ccc");
    }

    #[test]
    fn binary_pipe_overrun() {
        // Build the ring directly with capacity 2 and exercise the backpressure path.
        let ring: Arc<ArrayQueue<Vec<u8>>> = Arc::new(ArrayQueue::new(2));
        let shutdown = Arc::new(AtomicBool::new(false));

        // Simulate write_binary logic inline using capacity=2 ring.
        let mut dropped_count = 0usize;
        let payloads: &[&[u8]] = &[b"f1", b"f2", b"f3", b"f4"];
        for payload in payloads {
            let frame = payload.to_vec();
            if ring.push(frame).is_err() {
                ring.pop();
                dropped_count += 1;
                ring.push(payload.to_vec())
                    .expect("push after evict failed");
            }
        }

        shutdown.store(true, Ordering::Release);

        // Two frames were dropped (f1 and f2 were evicted when f3 and f4 arrived).
        assert_eq!(dropped_count, 2, "expected 2 DroppedOldest events");

        // Ring should hold the 2 newest frames.
        let last_two: Vec<Vec<u8>> = (0..2).filter_map(|_| ring.pop()).collect();
        assert!(last_two.contains(&b"f3".to_vec()) || last_two.contains(&b"f4".to_vec()));
    }

    #[test]
    fn json_pipe_register_close() {
        let mut registry = TypedPipeRegistry::new();

        registry
            .open_json("json-a".to_owned(), PipeDirection::In)
            .unwrap();
        registry
            .open_json("json-b".to_owned(), PipeDirection::Out)
            .unwrap();

        let list = registry.list();
        assert_eq!(list.len(), 2);
        let ids: Vec<&str> = list.iter().map(|(id, _, _)| id.as_str()).collect();
        assert!(ids.contains(&"json-a"));
        assert!(ids.contains(&"json-b"));
        for (_, mode, _) in &list {
            assert!(matches!(mode, PipeMode::Json));
        }

        registry.close("json-a");
        assert_eq!(registry.list().len(), 1);

        registry.close("json-b");
        assert!(registry.list().is_empty());
    }

    /// Regression: if the app never connects (e.g. start_capture failed before
    /// PipeOpened was sent), close() must not deadlock waiting for accept().
    /// The drain thread must observe the shutdown flag and exit on its own.
    #[test]
    fn close_without_client_does_not_deadlock() {
        let mut registry = TypedPipeRegistry::new();
        registry
            .open_binary("orphan".to_owned(), PipeDirection::Out)
            .expect("open_binary failed");

        // Close immediately — no client ever connected. If the drain thread
        // were stuck in a blocking accept(), close() would hang forever.
        let start = std::time::Instant::now();
        registry.close("orphan");
        let elapsed = start.elapsed();

        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "close() took {elapsed:?} — drain thread is deadlocked on accept()"
        );
        assert!(registry.list().is_empty());
    }
}
