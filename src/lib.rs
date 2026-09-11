//! Plexi library target — exposes `protocol` for the `gen_schema` binary
//! and `cli_args` for the `gen_cli_docs` binary.
//! This lib.rs is intentionally minimal: it only declares the modules that
//! these tools reference, using stub types that satisfy the type system
//! without pulling in the full GUI/audio dependency tree.

#[path = "cli/args.rs"]
pub mod cli_args;
/// Shared Ferrite-derived editor core (stint 0317). Also compiled into the
/// binary target (`mod editor` in main.rs), where the Notes pane consumes it
/// (stint 0474).
pub mod editor;
pub mod protocol;

// Stub modules: only the types used by `protocol` via crate:: references.
// The real implementations live in the binary target (src/main.rs).
pub mod media {
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
}

pub mod host {
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

        #[derive(
            Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
        )]
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

        #[derive(
            Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
        )]
        pub enum PaneSummaryStatus {
            Idle,
            Active,
            Error,
        }
    }
}
