//! Plexi library target — exposes `app_protocol` for the `gen_schema` binary.
//! This lib.rs is intentionally minimal: it only declares the modules that
//! `app_protocol` references via `crate::`, using stub types that satisfy
//! the type system without pulling in the full GUI/audio dependency tree.

pub mod app_protocol;

// Stub modules: only the types used by app_protocol via crate:: references.
// The real implementations live in the binary target (src/main.rs).
pub mod midi {
    /// One MIDI port. Stub for the lib target (full impl in binary target).
    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub struct MidiPortInfo {
        pub id: String,
        pub name: String,
        pub default: bool,
    }
}

pub mod audio {
    /// One audio device. Stub for the lib target (full impl in binary target).
    #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub struct AudioDeviceInfo {
        pub id: String,
        pub name: String,
        pub default: bool,
    }
}

pub mod video {
    /// Video playback state. Stub for the lib target (full impl in binary target).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    pub enum VideoState {
        Play,
        Pause,
        /// Absolute position in milliseconds from the start of the video.
        Seek { position_ms: u64 },
    }
}
