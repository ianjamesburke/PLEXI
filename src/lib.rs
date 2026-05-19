//! Plexi library target — exposes `app_protocol` for the `gen_schema` binary
//! and `cli_args` for the `gen_cli_docs` binary.
//! This lib.rs is intentionally minimal: it only declares the modules that
//! these tools reference, using stub types that satisfy the type system
//! without pulling in the full GUI/audio dependency tree.

pub mod app_protocol;
pub mod cli_args;

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

pub mod context_state {
    /// Rolled-up status for a single context. Stub for lib target.
    #[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    pub struct ContextState {
        pub context_id: u64,
        pub label: String,
        pub pane_count: u32,
        pub active_agents: u32,
        pub status: ContextStatus,
        pub pane_summaries: Vec<PaneSummary>,
        pub children: Vec<ContextState>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    pub enum ContextStatus {
        Idle,
        Working,
        Error,
        Done,
    }

    #[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    pub struct PaneSummary {
        pub pane_id: u64,
        pub label: String,
        pub status: PaneSummaryStatus,
    }

    #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    pub enum PaneSummaryStatus {
        Idle,
        Active,
        Error,
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
