//! System-prompt assembly for Plexi IQ.
//!
//! Stage 0: empty. Stage 1 will implement the five-layer assembly
//! described in spec §4:
//!
//!   1. Base harness prompt (static, part of cache prefix)
//!   2. Plexi-specific addendum (static per instance)
//!   3. Environment block (pane ID, directory scope, active app,
//!      available MCP servers — cacheable for pane lifetime)
//!   4. `CLAUDE.md` contents from `directory_scope` or any ancestor
//!   5. Inherited scrollback (when tabbing shell → agent; spec §6)
//!
//! Cache breakpoints: one `cache_control` marker after block 4, one on
//! the last stable user turn during the loop (spec §4).
//!
//! TODO (Stage 1): implement `build_system_prompt(&PlexiIqInstance) ->
//! String` and a helper that walks upward from `directory_scope` to find
//! the nearest `CLAUDE.md`.
