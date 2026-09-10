//! MIDI port description (#320).
//!
//! Only the on-the-wire port row survives here. The CoreMIDI device stack that
//! once backed it (enumeration, input streaming, output sending, UMP packing)
//! was never reachable from any build configuration and was deleted by stint
//! 0750; the `list_midi_devices` / `midi_devices_listed` protocol pair is still
//! declared, still described by the generated schema and the Python SDK, and
//! still has no host handler.

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
