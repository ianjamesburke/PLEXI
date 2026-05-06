//! Voice state machine for host-owned mic capture (issue #745).
//!
//! `VoiceState` tracks the listening lifecycle. `HearingSpeech` and
//! `Processing` variants are added in issue #746 when VAD and transcription
//! are wired up.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceState {
    #[default]
    Off,
    /// Mic capture active, waiting for speech (VAD not yet wired — issue #746).
    ListeningIdle,
}
