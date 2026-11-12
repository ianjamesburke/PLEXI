//! Observe-only tap on the bytes read from a PTY, ahead of the terminal parser.
use std::io;
use std::sync::{Arc, Mutex};

/// Sees every chunk of PTY output before it reaches the terminal emulator.
/// The tap only observes: the bytes are forwarded to the parser unchanged.
/// Returned bytes are written back to the PTY (e.g. a feature-query reply).
pub trait PtyByteTap: Send {
    fn on_bytes(&mut self, bytes: &[u8]) -> Vec<u8>;
}

/// Cloneable, debuggable handle so `BackendSettings` keeps its derives.
#[derive(Clone)]
pub struct ByteTapHandle(Arc<Mutex<dyn PtyByteTap>>);

impl ByteTapHandle {
    pub fn new<T: PtyByteTap + 'static>(tap: T) -> Self {
        Self(Arc::new(Mutex::new(tap)))
    }
}

impl std::fmt::Debug for ByteTapHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ByteTapHandle")
    }
}

/// `io::Read` wrapper that passes each successful read to an optional tap.
pub struct TapReader<R: io::Read> {
    inner: R,
    tap: Option<ByteTapHandle>,
    reply: Box<dyn FnMut(Vec<u8>) + Send>,
}

impl<R: io::Read> TapReader<R> {
    pub fn new(
        inner: R,
        tap: Option<ByteTapHandle>,
        reply: Box<dyn FnMut(Vec<u8>) + Send>,
    ) -> Self {
        Self { inner, tap, reply }
    }
}

impl<R: io::Read> io::Read for TapReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            if let Some(tap) = &self.tap {
                let reply = tap
                    .0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .on_bytes(&buf[..n]);
                if !reply.is_empty() {
                    (self.reply)(reply);
                }
            }
        }
        Ok(n)
    }
}
