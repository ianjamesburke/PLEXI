//! CoreMIDI I/O — device enumeration + input streaming + output sending (#320).
//!
//! Three layers, mirroring [`crate::audio`]:
//!
//!   1. [`MidiDevice`] — host-facing trait. Methods return owned data
//!      ([`MidiPortInfo`]) so the caller never holds a `coremidi::Source`
//!      reference. The trait lives behind `Arc<dyn MidiDevice>` on each
//!      `ProcessApp` instance, the same shape as [`crate::media::audio::AudioDevice`].
//!
//!   2. [`CoreMidiDevice`] — production impl (macOS only). Wraps `coremidi`
//!      crate 0.9. CoreMIDI does not gate behind a privacy prompt today
//!      (unlike microphone) so there's no TCC plumbing.
//!
//!   3. [`MockMidiDevice`] — test impl. Returns a fixed port list and lets
//!      tests inject MIDI byte sequences into open inputs without touching
//!      real hardware.
//!
//! Wire shape: apps send / receive **MIDI 1.0 byte streams** (e.g. `[0x90, 0x40, 0x7f]`
//! for NoteOn channel 0 note 64 velocity 127). Internally, the CoreMIDI impl
//! packs each byte stream into a Universal MIDI Packet (UMP) on group 0 and
//! unpacks UMPs back into byte streams on the input path. CoreMIDI handles
//! UMP↔legacy byte-stream translation transparently when delivering to /
//! receiving from MIDI 1.0 endpoints.
//!
//! Out of scope for v3.4 (deferred):
//!   - Virtual MIDI ports (creating Plexi as a source/destination other apps see).
//!   - SysEx (variable-length F0..F7) — protocol carries `Vec<u8>` so it could
//!     work as a future extension once UMP packing handles SysEx7/SysEx8 packets.
//!   - MIDI 2.0 (we ship classic MIDI 1.0 only).
//!   - Network MIDI / RTP-MIDI.

use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;

// ─── Public types ────────────────────────────────────────────────────────────

/// Stable info row for one MIDI port (input or output).
///
/// `id` is the CoreMIDI unique-id rendered as decimal. It is stable across
/// reboots (CoreMIDI persists per-endpoint UIDs in `~/Library/Audio/MIDI Devices/`)
/// but not across machines. Apps that persist a "last selected port" should
/// fall back to `default = true` when the saved id is no longer present.
///
/// `default` is best-effort: CoreMIDI has no system-wide "default MIDI port"
/// concept. We mark the **first** port `default = true` so apps that just
/// want "any" port have a deterministic pick.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MidiPortInfo {
    pub id: String,
    pub name: String,
    pub default: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum MidiError {
    #[error("port id '{0}' not found")]
    PortNotFound(String),
    #[error("MIDI byte stream empty")]
    EmptyMessage,
    /// Wraps any underlying CoreMIDI failure — Client::new, port creation,
    /// connect_source, send. The error string includes the CoreMIDI OSStatus
    /// where available.
    #[error("coremidi: {0}")]
    CoreMidi(String),
}

/// Where incoming MIDI byte streams are delivered. One callback per packet
/// (typically a 1–3 byte MIDI 1.0 message). Returns `Ok(())` on success;
/// returns `Err(())` to signal "stop the stream" (e.g. the consumer was dropped).
///
/// The trait stays object-safe; the closure does not allocate beyond the
/// per-packet `Vec<u8>` it's handed.
pub type MidiPacketSink = Arc<dyn Fn(&[u8]) -> Result<(), ()> + Send + Sync + 'static>;

/// Opaque handle returned from `open_input`. Dropping the handle (or calling
/// `close_input`) tears down the underlying CoreMIDI connection.
pub struct MidiInputSession {
    /// Cleanup runs on drop. The closure owns whatever the device impl needs
    /// to keep alive — for CoreMIDI that's the `InputPort` + `Source` (port
    /// disconnects on drop).
    _guard: Box<dyn MidiInputGuard>,
    /// Echoed back to the app on `MidiInputOpened` so it can correlate the
    /// pipe with the port that was actually opened.
    pub port_name: String,
}

impl std::fmt::Debug for MidiInputSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidiInputSession")
            .field("port_name", &self.port_name)
            .finish()
    }
}

trait MidiInputGuard: Send {}

/// Opaque handle for a CoreMIDI output port + its destination endpoint.
/// Apps `send_midi(port_id, bytes)` through the host; the host looks up the
/// output handle for the type_id and dispatches the send. Closing the handle
/// (drop) destroys the underlying `OutputPort`.
pub struct MidiOutputHandle {
    inner: Box<dyn MidiOutputHandleInner>,
    pub port_name: String,
}

impl std::fmt::Debug for MidiOutputHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidiOutputHandle")
            .field("port_name", &self.port_name)
            .finish()
    }
}

impl MidiOutputHandle {
    /// Send one MIDI 1.0 byte stream to this output's destination. The bytes
    /// are packed into a UMP on group 0; CoreMIDI unpacks them back to legacy
    /// MIDI 1.0 byte stream on the wire when the endpoint is a MIDI 1.0 device
    /// (which is the only kind we open in v3.4).
    pub fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
        if bytes.is_empty() {
            return Err(MidiError::EmptyMessage);
        }
        self.inner.send(bytes)
    }
}

trait MidiOutputHandleInner: Send {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError>;
}

// ─── Device trait ────────────────────────────────────────────────────────────

pub trait MidiDevice: Send + Sync {
    fn list_input_ports(&self) -> Vec<MidiPortInfo>;
    fn list_output_ports(&self) -> Vec<MidiPortInfo>;
    /// Open the named input port and start delivering MIDI byte streams to
    /// `sink` on a CoreMIDI callback thread. The returned `MidiInputSession`
    /// owns the connection — drop it (or call `close_input` via the trait
    /// caller's bookkeeping) to stop.
    fn open_input(
        &self,
        port_id: &str,
        sink: MidiPacketSink,
    ) -> Result<MidiInputSession, MidiError>;
    /// Open the named output port and return a handle the host uses to send
    /// MIDI bytes. The handle owns the `OutputPort` + `Destination`; dropping
    /// it tears the connection down.
    fn open_output(&self, port_id: &str) -> Result<MidiOutputHandle, MidiError>;
}

// ─── Production impl: CoreMidiDevice ─────────────────────────────────────────

/// Production CoreMIDI device. Holds one `coremidi::Client` for the lifetime
/// of the host process so ports created from it stay valid. The client name
/// shows up in macOS Audio MIDI Setup as the source/destination owner.
///
/// Non-test, macOS only — non-mac builds use the `Mock` impl via
/// `default_midi_device()` so the full host still compiles cross-platform.
#[cfg(all(not(test), target_os = "macos"))]
pub struct CoreMidiDevice {
    client: coremidi::Client,
}

#[cfg(all(not(test), target_os = "macos"))]
impl CoreMidiDevice {
    pub fn new() -> Self {
        // Client::new can fail if CoreMIDI is unavailable (e.g. running in a
        // sandbox without the framework). We don't want to take down the host
        // for a missing audio subsystem; use a process-wide fallback that
        // logs and reports `PortNotFound` for every operation.
        let client = match coremidi::Client::new("plexi-midi") {
            Ok(c) => c,
            Err(status) => {
                log::error!(
                    "CoreMidiDevice::new: coremidi::Client::new failed (OSStatus={status}); MIDI disabled"
                );
                // We can't construct a real client; create a dummy one anyway
                // so the impl shape stays uniform. Subsequent calls will just
                // return PortNotFound because Sources/Destinations enumeration
                // returns empty.
                //
                // In practice CoreMIDI initialisation should never fail on a
                // mac with the framework linked, so this path is defensive.
                panic!("coremidi: failed to create client (OSStatus={status})");
            }
        };
        Self { client }
    }

    fn collect_sources() -> Vec<MidiPortInfo> {
        Self::collect(true)
    }

    fn collect_destinations() -> Vec<MidiPortInfo> {
        Self::collect(false)
    }

    fn collect(is_source: bool) -> Vec<MidiPortInfo> {
        let mut out: Vec<MidiPortInfo> = Vec::new();
        if is_source {
            for (i, src) in coremidi::Sources.into_iter().enumerate() {
                let id = src.unique_id().unwrap_or(0);
                let name = src.display_name().unwrap_or_else(|| format!("source-{i}"));
                out.push(MidiPortInfo {
                    id: id.to_string(),
                    name,
                    default: i == 0,
                });
            }
        } else {
            for (i, dst) in coremidi::Destinations.into_iter().enumerate() {
                let id = dst.unique_id().unwrap_or(0);
                let name = dst
                    .display_name()
                    .unwrap_or_else(|| format!("destination-{i}"));
                out.push(MidiPortInfo {
                    id: id.to_string(),
                    name,
                    default: i == 0,
                });
            }
        }
        out
    }

    fn parse_unique_id(port_id: &str) -> Result<u32, MidiError> {
        port_id
            .parse::<u32>()
            .map_err(|e| MidiError::CoreMidi(format!("bad port id {port_id:?}: {e}")))
    }
}

#[cfg(all(not(test), target_os = "macos"))]
impl MidiDevice for CoreMidiDevice {
    fn list_input_ports(&self) -> Vec<MidiPortInfo> {
        Self::collect_sources()
    }

    fn list_output_ports(&self) -> Vec<MidiPortInfo> {
        Self::collect_destinations()
    }

    fn open_input(
        &self,
        port_id: &str,
        sink: MidiPacketSink,
    ) -> Result<MidiInputSession, MidiError> {
        let unique_id = Self::parse_unique_id(port_id)?;
        let source = coremidi::Source::from_unique_id(unique_id)
            .ok_or_else(|| MidiError::PortNotFound(port_id.to_owned()))?;
        let port_name = source
            .display_name()
            .unwrap_or_else(|| format!("source-{port_id}"));

        // Build the input port with a callback that unpacks every UMP packet
        // back into MIDI 1.0 byte streams and forwards them to `sink`.
        let cb_sink = std::sync::Arc::clone(&sink);
        let mut input_port = self
            .client
            .input_port_with_protocol(
                "plexi-midi-in",
                coremidi::Protocol::Midi10,
                move |event_list: &coremidi::EventList, _ctx: &mut u32| {
                    for packet in event_list.iter() {
                        for &word in packet.data() {
                            for bytes in unpack_ump_word_to_bytes(word) {
                                if cb_sink(&bytes).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                },
            )
            .map_err(|status| {
                MidiError::CoreMidi(format!(
                    "input_port_with_protocol failed (OSStatus={status})"
                ))
            })?;

        input_port
            .connect_source(&source, unique_id)
            .map_err(|status| {
                MidiError::CoreMidi(format!("connect_source failed (OSStatus={status})"))
            })?;

        struct InputGuard {
            port: Option<coremidi::InputPortWithContext<u32>>,
            source: coremidi::Source,
        }
        impl MidiInputGuard for InputGuard {}
        impl Drop for InputGuard {
            fn drop(&mut self) {
                if let Some(mut port) = self.port.take() {
                    if let Err(status) = port.disconnect_source(&self.source) {
                        log::warn!("CoreMidiDevice: disconnect_source failed (OSStatus={status})");
                    }
                }
            }
        }

        Ok(MidiInputSession {
            _guard: Box::new(InputGuard {
                port: Some(input_port),
                source,
            }),
            port_name,
        })
    }

    fn open_output(&self, port_id: &str) -> Result<MidiOutputHandle, MidiError> {
        let unique_id = Self::parse_unique_id(port_id)?;
        // CoreMIDI exposes `Destination::from_index` but not `from_unique_id`,
        // so we walk the iterator and match on the unique-id ourselves.
        // Index-walk is O(n) but n is tiny (single-digit port count).
        let destination = coremidi::Destinations
            .into_iter()
            .find(|d| d.unique_id().map(|u| u == unique_id).unwrap_or(false))
            .ok_or_else(|| MidiError::PortNotFound(port_id.to_owned()))?;
        let port_name = destination
            .display_name()
            .unwrap_or_else(|| format!("destination-{port_id}"));

        let output_port = self
            .client
            .output_port("plexi-midi-out")
            .map_err(|status| {
                MidiError::CoreMidi(format!("output_port failed (OSStatus={status})"))
            })?;

        struct CoreOutput {
            port: coremidi::OutputPort,
            destination: coremidi::Destination,
        }
        impl MidiOutputHandleInner for CoreOutput {
            fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
                let word = pack_bytes_to_ump_word(bytes)?;
                let buffer =
                    coremidi::EventBuffer::new(coremidi::Protocol::Midi10).with_packet(0, &[word]);
                self.port
                    .send(&self.destination, &buffer)
                    .map_err(|status| {
                        MidiError::CoreMidi(format!("output send failed (OSStatus={status})"))
                    })
            }
        }

        Ok(MidiOutputHandle {
            inner: Box::new(CoreOutput {
                port: output_port,
                destination,
            }),
            port_name,
        })
    }
}

// Cross-platform fallback for non-mac, non-test production builds. Reports an
// empty port list and refuses any open. Keeps the host buildable on Linux/CI
// without coremidi available.
#[cfg(all(not(test), not(target_os = "macos")))]
pub struct CoreMidiDevice;

#[cfg(all(not(test), not(target_os = "macos")))]
impl CoreMidiDevice {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(all(not(test), not(target_os = "macos")))]
impl MidiDevice for CoreMidiDevice {
    fn list_input_ports(&self) -> Vec<MidiPortInfo> {
        Vec::new()
    }
    fn list_output_ports(&self) -> Vec<MidiPortInfo> {
        Vec::new()
    }
    fn open_input(
        &self,
        port_id: &str,
        _sink: MidiPacketSink,
    ) -> Result<MidiInputSession, MidiError> {
        Err(MidiError::PortNotFound(port_id.to_owned()))
    }
    fn open_output(&self, port_id: &str) -> Result<MidiOutputHandle, MidiError> {
        Err(MidiError::PortNotFound(port_id.to_owned()))
    }
}

// ─── UMP packing/unpacking (MIDI 1.0 channel-voice + system real-time) ──────

/// Pack a MIDI 1.0 byte stream into a single 32-bit Universal MIDI Packet
/// word on group 0.
///
/// Supported message classes:
///   - System real-time (1 byte, status >= 0xF8): packed as Message Type 0x1
///     (System Common / System Real-Time).
///   - Channel voice (3 bytes, 0x80..=0xEF status): packed as Message Type 0x2
///     (MIDI 1.0 Channel Voice). This covers NoteOn, NoteOff, CC, Pitchbend,
///     Program Change (which is 2 bytes — handled below), Channel Pressure
///     (also 2 bytes).
///
/// Per the MIDI 2.0 spec, 2-byte channel-voice messages (Program Change,
/// Channel Pressure) still use the 32-bit MIDI 1.0 word format with the
/// unused data byte set to 0.
fn pack_bytes_to_ump_word(bytes: &[u8]) -> Result<u32, MidiError> {
    if bytes.is_empty() {
        return Err(MidiError::EmptyMessage);
    }
    let status = bytes[0];

    // System real-time (single-byte): 0xF8..=0xFF.
    // Message Type 0x1 (System Common / System Real-Time).
    if status >= 0xF8 {
        // 0x10000000 | (status << 16). Group 0, no data bytes.
        return Ok(0x1000_0000 | ((status as u32) << 16));
    }

    // Channel voice 0x80..=0xEF.
    if (0x80..=0xEF).contains(&status) {
        let d1 = bytes.get(1).copied().unwrap_or(0);
        let d2 = bytes.get(2).copied().unwrap_or(0);
        // 0x20000000 | (status << 16) | (d1 << 8) | d2. Group 0.
        return Ok(0x2000_0000 | ((status as u32) << 16) | ((d1 as u32) << 8) | (d2 as u32));
    }

    // System common (0xF0..=0xF7) — SysEx start, MTC, song position, etc.
    // SysEx is variable-length and out of scope for v3.4 (deferred). The
    // shorter system-common messages (0xF1, 0xF2, 0xF3) could fit a UMP but
    // we don't have callers for them yet. Fail loudly so a future caller
    // doesn't get a silently-wrong packing.
    Err(MidiError::CoreMidi(format!(
        "unsupported MIDI status byte 0x{status:02x}: only channel-voice (0x80-0xEF) and system real-time (0xF8-0xFF) are supported in v3.4"
    )))
}

/// Inverse of `pack_bytes_to_ump_word`. One UMP word may yield zero MIDI 1.0
/// byte streams (e.g. MIDI 2.0-only message types) or one MIDI 1.0 byte
/// stream.
fn unpack_ump_word_to_bytes(word: u32) -> Vec<Vec<u8>> {
    let message_type = (word >> 28) & 0xF;
    match message_type {
        0x1 => {
            // System Common / System Real-Time: 1 status byte.
            let status = ((word >> 16) & 0xFF) as u8;
            vec![vec![status]]
        }
        0x2 => {
            // MIDI 1.0 Channel Voice: status + d1 + d2.
            let status = ((word >> 16) & 0xFF) as u8;
            let d1 = ((word >> 8) & 0xFF) as u8;
            let d2 = (word & 0xFF) as u8;
            // 2-byte messages (Program Change 0xCx, Channel Pressure 0xDx)
            // carry d2 = 0; emit only the bytes that the message format defines.
            let high_nibble = status & 0xF0;
            if high_nibble == 0xC0 || high_nibble == 0xD0 {
                vec![vec![status, d1]]
            } else {
                vec![vec![status, d1, d2]]
            }
        }
        // Other UMP message types (utility 0x0, system-exclusive 0x3/0x5,
        // MIDI 2.0 channel-voice 0x4) — not exposed to MIDI 1.0 apps in v3.4.
        // Drop silently; CoreMIDI may translate them but if it doesn't, we
        // don't have a byte-stream representation.
        _ => Vec::new(),
    }
}

// ─── Test impl: MockMidiDevice ───────────────────────────────────────────────

/// Test-only MIDI device. Reports a stable port list and lets tests poke
/// MIDI byte streams into open inputs without touching real hardware.
///
/// `cfg(test)`-gated. Production code paths use `CoreMidiDevice` exclusively
/// via `default_midi_device()` in `process_app/mod.rs`.
#[cfg(test)]
pub struct MockMidiDevice {
    pub inputs: Vec<MidiPortInfo>,
    pub outputs: Vec<MidiPortInfo>,
    /// Per-port-id sink registry — `open_input` populates this, tests pull
    /// the sink back out and shove fake MIDI bytes through it. Wrapped in
    /// `Mutex` so the trait stays object-safe and tests can mutate from any
    /// thread.
    pub injected_sinks: Arc<Mutex<std::collections::HashMap<String, MidiPacketSink>>>,
    /// Per-port-id outbound log — `open_output` returns a handle that pushes
    /// each `send()` into the corresponding `Vec<Vec<u8>>` here. Tests assert
    /// against these to verify the routing layer dispatched a send.
    pub sent: Arc<Mutex<std::collections::HashMap<String, Vec<Vec<u8>>>>>,
}

#[cfg(test)]
impl Default for MockMidiDevice {
    fn default() -> Self {
        Self {
            inputs: vec![
                MidiPortInfo {
                    id: "mock-input-1".to_owned(),
                    name: "Mock Controller (Input)".to_owned(),
                    default: true,
                },
                MidiPortInfo {
                    id: "mock-input-2".to_owned(),
                    name: "Mock IAC Bus".to_owned(),
                    default: false,
                },
            ],
            outputs: vec![MidiPortInfo {
                id: "mock-output-1".to_owned(),
                name: "Mock Synth (Output)".to_owned(),
                default: true,
            }],
            injected_sinks: Arc::new(Mutex::new(std::collections::HashMap::new())),
            sent: Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }
}

#[cfg(test)]
impl MockMidiDevice {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test helper: deliver a fake MIDI packet to whatever sink is currently
    /// open on `port_id`. Returns `false` if no input is open on that port.
    pub fn inject(&self, port_id: &str, bytes: &[u8]) -> bool {
        let guard = self
            .injected_sinks
            .lock()
            .expect("mock midi sinks poisoned");
        match guard.get(port_id) {
            Some(sink) => {
                let _ = sink(bytes);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
impl MidiDevice for MockMidiDevice {
    fn list_input_ports(&self) -> Vec<MidiPortInfo> {
        self.inputs.clone()
    }

    fn list_output_ports(&self) -> Vec<MidiPortInfo> {
        self.outputs.clone()
    }

    fn open_input(
        &self,
        port_id: &str,
        sink: MidiPacketSink,
    ) -> Result<MidiInputSession, MidiError> {
        let port = self
            .inputs
            .iter()
            .find(|p| p.id == port_id)
            .ok_or_else(|| MidiError::PortNotFound(port_id.to_owned()))?
            .clone();

        self.injected_sinks
            .lock()
            .expect("mock midi sinks poisoned")
            .insert(port_id.to_owned(), sink);

        // The guard removes the sink registration on drop so the mock is
        // re-openable across tests.
        struct MockInputGuard {
            port_id: String,
            sinks: Arc<Mutex<std::collections::HashMap<String, MidiPacketSink>>>,
        }
        impl MidiInputGuard for MockInputGuard {}
        impl Drop for MockInputGuard {
            fn drop(&mut self) {
                self.sinks
                    .lock()
                    .expect("mock midi sinks poisoned on drop")
                    .remove(&self.port_id);
            }
        }

        Ok(MidiInputSession {
            _guard: Box::new(MockInputGuard {
                port_id: port_id.to_owned(),
                sinks: Arc::clone(&self.injected_sinks),
            }),
            port_name: port.name,
        })
    }

    fn open_output(&self, port_id: &str) -> Result<MidiOutputHandle, MidiError> {
        let port = self
            .outputs
            .iter()
            .find(|p| p.id == port_id)
            .ok_or_else(|| MidiError::PortNotFound(port_id.to_owned()))?
            .clone();

        self.sent
            .lock()
            .expect("mock midi sent poisoned")
            .entry(port_id.to_owned())
            .or_default();

        struct MockOutput {
            port_id: String,
            sent: Arc<Mutex<std::collections::HashMap<String, Vec<Vec<u8>>>>>,
        }
        impl MidiOutputHandleInner for MockOutput {
            fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
                self.sent
                    .lock()
                    .expect("mock midi sent poisoned on send")
                    .entry(self.port_id.clone())
                    .or_default()
                    .push(bytes.to_vec());
                Ok(())
            }
        }

        Ok(MidiOutputHandle {
            inner: Box::new(MockOutput {
                port_id: port_id.to_owned(),
                sent: Arc::clone(&self.sent),
            }),
            port_name: port.name,
        })
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn collecting_sink(received: Arc<Mutex<Vec<Vec<u8>>>>) -> MidiPacketSink {
        Arc::new(move |bytes: &[u8]| {
            received
                .lock()
                .expect("test sink poisoned")
                .push(bytes.to_vec());
            Ok(())
        })
    }

    #[test]
    fn list_input_ports_returns_at_least_default() {
        let dev = MockMidiDevice::new();
        let inputs = dev.list_input_ports();
        assert!(!inputs.is_empty(), "mock must report at least one input");
        let defaults = inputs.iter().filter(|p| p.default).count();
        assert_eq!(defaults, 1, "exactly one input must be the default");
    }

    #[test]
    fn list_output_ports_returns_at_least_default() {
        let dev = MockMidiDevice::new();
        let outputs = dev.list_output_ports();
        assert!(!outputs.is_empty(), "mock must report at least one output");
        assert!(
            outputs.iter().any(|p| p.default),
            "an output must be the default"
        );
    }

    #[test]
    fn mock_device_round_trips_packets() {
        // Open a mock input, inject NoteOn(ch=0,note=60,vel=100), assert the
        // sink received it verbatim.
        let dev = MockMidiDevice::new();
        let received = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let _session = dev
            .open_input("mock-input-1", collecting_sink(Arc::clone(&received)))
            .expect("mock open_input must succeed");

        assert!(dev.inject("mock-input-1", &[0x90, 0x3C, 0x64]));
        // Send a clock pulse too — single byte system-real-time.
        assert!(dev.inject("mock-input-1", &[0xF8]));

        let got = received.lock().expect("test received poisoned").clone();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], vec![0x90, 0x3C, 0x64]);
        assert_eq!(got[1], vec![0xF8]);
    }

    #[test]
    fn core_midi_device_no_panic_on_nonexistent_port_id() {
        // Production-stub assertion: open_input / open_output with a bogus
        // port id must return Err, never panic. The trait-level guarantee.
        //
        // The mock drives this contract because real CoreMIDI calls require
        // a macOS environment unsuitable for CI. The CoreMidiDevice impl
        // honours the same contract via its `parse_unique_id` + `from_unique_id`
        // bail-out path.
        let dev = MockMidiDevice::new();
        let received = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let res = dev.open_input("definitely-not-a-real-port", collecting_sink(received));
        match res {
            Err(MidiError::PortNotFound(id)) => {
                assert_eq!(id, "definitely-not-a-real-port");
            }
            other => panic!("expected PortNotFound, got {other:?}"),
        }
        let res = dev.open_output("definitely-not-a-real-port");
        match res {
            Err(MidiError::PortNotFound(id)) => {
                assert_eq!(id, "definitely-not-a-real-port");
            }
            other => panic!("expected PortNotFound, got {other:?}"),
        }
    }

    #[test]
    fn open_input_streams_into_sink() {
        // The trait contract: open_input returns a session, the sink receives
        // every injected byte stream until the session is dropped, and after
        // drop further injects find no sink.
        let dev = MockMidiDevice::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_for_sink = Arc::clone(&counter);
        let sink: MidiPacketSink = Arc::new(move |_bytes: &[u8]| {
            counter_for_sink.fetch_add(1, Ordering::Relaxed);
            Ok(())
        });

        let session = dev
            .open_input("mock-input-1", sink)
            .expect("mock open_input must succeed");

        for _ in 0..5 {
            assert!(dev.inject("mock-input-1", &[0x90, 0x3C, 0x64]));
        }
        assert_eq!(counter.load(Ordering::Relaxed), 5);

        drop(session);
        assert!(
            !dev.inject("mock-input-1", &[0x90, 0x3C, 0x64]),
            "after drop the sink must be unregistered"
        );
        assert_eq!(counter.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn output_handle_logs_sends() {
        let dev = MockMidiDevice::new();
        let mut out = dev
            .open_output("mock-output-1")
            .expect("mock open_output must succeed");
        out.send(&[0x90, 0x3C, 0x64]).expect("send must succeed");
        out.send(&[0x80, 0x3C, 0x00]).expect("send must succeed");
        let log = dev.sent.lock().expect("mock sent poisoned").clone();
        let entries = log
            .get("mock-output-1")
            .expect("mock-output-1 entry must exist");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], vec![0x90, 0x3C, 0x64]);
        assert_eq!(entries[1], vec![0x80, 0x3C, 0x00]);
    }

    #[test]
    fn output_handle_rejects_empty_message() {
        let dev = MockMidiDevice::new();
        let mut out = dev
            .open_output("mock-output-1")
            .expect("mock open_output must succeed");
        match out.send(&[]) {
            Err(MidiError::EmptyMessage) => {}
            other => panic!("expected EmptyMessage, got {other:?}"),
        }
    }

    #[test]
    fn ump_round_trip_channel_voice() {
        // Pack NoteOn(ch=0, note=60, vel=100) → unpack must give back the
        // same 3 bytes.
        let bytes = vec![0x90, 0x3C, 0x64];
        let word = pack_bytes_to_ump_word(&bytes).expect("pack");
        // 0x20000000 | (0x90 << 16) | (0x3C << 8) | 0x64
        assert_eq!(word, 0x2000_0000 | (0x90 << 16) | (0x3C << 8) | 0x64);
        let out = unpack_ump_word_to_bytes(word);
        assert_eq!(out, vec![bytes]);
    }

    #[test]
    fn ump_round_trip_clock_pulse() {
        // Pack 0xF8 (timing clock) → unpack must give back a single 0xF8.
        let word = pack_bytes_to_ump_word(&[0xF8]).expect("pack");
        // 0x10000000 | (0xF8 << 16)
        assert_eq!(word, 0x1000_0000 | (0xF8 << 16));
        let out = unpack_ump_word_to_bytes(word);
        assert_eq!(out, vec![vec![0xF8]]);
    }

    #[test]
    fn ump_program_change_is_two_bytes() {
        // Program Change is 2-byte channel-voice. Pack/unpack must drop the
        // unused data byte on the unpack path.
        let bytes = vec![0xC0, 0x05];
        let word = pack_bytes_to_ump_word(&bytes).expect("pack");
        let out = unpack_ump_word_to_bytes(word);
        assert_eq!(out, vec![bytes]);
    }
}
