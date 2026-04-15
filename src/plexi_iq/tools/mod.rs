//! Tool trait and registry for Plexi IQ.
//!
//! Stage 0: trait surface and empty `ToolRegistry`. Stage 1 will add the
//! built-in tools (`Read`, `Edit`, `Write`, `Bash`, `Grep`, `Glob`,
//! `TodoWrite`, `Task`) and the dynamic slots for app-protocol + MCP
//! tools. See spec §3.3 and §3.4.

use crate::plexi_iq::context::ToolContext;

/// Outcome of a tool invocation. Stage 1 will expand the error side into
/// transport / validation / bus_closed / denied variants (spec §8 risk #5).
#[derive(Debug)]
pub enum ToolResult {
    /// Tool completed and produced a result payload the model should see.
    Ok(serde_json::Value),
    /// Tool failed. The `String` message is echoed back to the model as a
    /// tool_result with `is_error: true`.
    Error(String),
}

/// The core tool contract. Mirrors Claude Code's built-in tool shape so the
/// model's training transfers directly (spec §3.4).
///
/// `description` must be ≥3 sentences covering: when to use, constraints,
/// and at least one example. Use `schemars::JsonSchema` on the input struct
/// and call `schemars::schema_for!(Input)` inside `input_schema()` for the
/// built-ins (spec §3.3).
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    /// Stable tool name — what the model writes in `tool_use.name`.
    fn name(&self) -> &str;

    /// Human- and model-facing description. See trait-level note on
    /// required structure.
    fn description(&self) -> &str;

    /// JSON Schema for the tool's input. Generated via `schemars` for
    /// built-ins; synthesized from the app manifest for app-protocol
    /// tools.
    fn input_schema(&self) -> serde_json::Value;

    /// Execute the tool. `ctx` carries pane ID, directory scope, session
    /// state (Read-before-Edit guard), and the optional app bus handle.
    async fn run(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}

/// Registry of tools available to a single `PlexiIqInstance`.
///
/// Stage 0: empty shell. Stage 1 will populate it from four sources
/// (built-ins, app-protocol, MCP, subagent `Task`) and rebuild it when
/// the pane's companion app changes (spec §3.4: "dynamic per instance").
#[derive(Default)]
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    /// Build an empty registry. Stage 1 will add constructor variants
    /// that seed built-ins, merge app-protocol tools, etc.
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// Number of registered tools. Stage 1 will add real accessors
    /// (by-name lookup, schema export, etc.).
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry has any tools registered.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tool_count", &self.tools.len())
            .finish()
    }
}
