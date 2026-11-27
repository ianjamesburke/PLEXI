//! Plexi library target — exposes `protocol` for the `gen_schema` binary
//! and `cli_args` for the `gen_cli_docs` binary.
//! This lib.rs is intentionally minimal: it only declares the modules that
//! these tools reference, using stub types that satisfy the type system
//! without pulling in the full GUI/audio dependency tree.

#[path = "cli/args.rs"]
pub mod cli_args;
#[path = "release/catalog.rs"]
pub mod release_catalog;
/// Shared Ferrite-derived editor core (stint 0317). Also compiled into the
/// binary target (`mod editor` in main.rs), where the Notes pane consumes it
/// (stint 0474).
pub mod editor;
pub mod protocol;
