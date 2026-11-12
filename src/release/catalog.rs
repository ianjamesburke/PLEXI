//! Shared experiment command ownership for the host and documentation generators.

/// A clap path owned by an experiment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GatePath {
    /// The subcommand at this path, and everything below it.
    Sub(&'static [&'static str]),
    /// The argument with this id on the subcommand at this path.
    Arg(&'static [&'static str], &'static str),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ReleaseFeature {
    Assistant,
    AiSetup,
    AgentsDashboard,
    ChangeSets,
    Ledger,
    CloudAccount,
    FolderSecrets,
    WasmApps,
    PageGrid,
    TreeAtlas,
    Marketplace,
    Daw,
    MediaIo,
    Accessibility,
    McpClient,
    CliOperations,
    Routines,
    AgentMesh,
}

impl ReleaseFeature {
    pub const ALL: &'static [ReleaseFeature] = &[
        Self::Assistant,
        Self::AiSetup,
        Self::AgentsDashboard,
        Self::ChangeSets,
        Self::Ledger,
        Self::CloudAccount,
        Self::FolderSecrets,
        Self::WasmApps,
        Self::PageGrid,
        Self::TreeAtlas,
        Self::Marketplace,
        Self::Daw,
        Self::MediaIo,
        Self::Accessibility,
        Self::McpClient,
        Self::CliOperations,
        Self::Routines,
        Self::AgentMesh,
    ];

    /// The `[experiments]` config key and the name used by `plexi experiments`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Assistant => "assistant",
            Self::AiSetup => "ai_setup",
            Self::AgentsDashboard => "command_view",
            Self::ChangeSets => "change_sets",
            Self::Ledger => "ledger",
            Self::CloudAccount => "cloud_account",
            Self::FolderSecrets => "folder_secrets",
            Self::WasmApps => "wasm_apps",
            Self::PageGrid => "page_grid",
            Self::TreeAtlas => "tree_atlas",
            Self::Marketplace => "marketplace",
            Self::Daw => "daw",
            Self::MediaIo => "media_io",
            Self::Accessibility => "accessibility",
            Self::McpClient => "mcp_client",
            Self::CliOperations => "cli_operations",
            Self::Routines => "routines",
            Self::AgentMesh => "agent_mesh",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|f| f.key() == key)
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Assistant => "Host AI assistant pane and the `assistant` commands",
            Self::AiSetup => "Guided AI backend setup (`ai setup`, `ai onboard`)",
            Self::AgentsDashboard => "Agent runs dashboard pane and `agents-dashboard`",
            Self::ChangeSets => "Preview and accept agent edits (`changes`)",
            Self::Ledger => "Token and cost summary (`ledger`)",
            Self::CloudAccount => "Optional cloud account and history retention",
            Self::FolderSecrets => "Secrets injected per folder, with grants",
            Self::WasmApps => "Native Rust/WASM plugins and the GPU canvas",
            Self::PageGrid => "Reserved page-grid experiment",
            Self::TreeAtlas => "Context tree overview at root with Cmd+Escape",
            Self::Marketplace => "App marketplace: browse, search, publish",
            Self::Daw => "DAW engine proof of concept",
            Self::MediaIo => "Audio and video player panes",
            Self::Accessibility => "AccessKit accessibility tree",
            Self::McpClient => "MCP client: host MCP config and `view open --mcp`",
            Self::CliOperations => "Approved typed CLI operations for sandboxed plugins",
            Self::Routines => "Scheduled commands (`routine`)",
            Self::AgentMesh => "Agent roles, spawning, teams and fleet control (`agent` beyond status hooks and worker completion)",
        }
    }

    /// True when the host reads this experiment at runtime, so a change only
    /// takes effect after `plexi host restart`. Pure CLI gates re-read the
    /// config on every invocation.
    pub fn requires_restart(self) -> bool {
        !matches!(self, Self::AiSetup | Self::Ledger | Self::AgentMesh)
    }

    /// The clap paths this experiment owns. Empty for runtime-only surfaces
    /// that have no command of their own.
    pub fn owned_paths(self) -> &'static [GatePath] {
        use GatePath::{Arg, Sub};
        match self {
            Self::Assistant => &[Sub(&["assistant"])],
            Self::AiSetup => &[Sub(&["ai", "setup"]), Sub(&["ai", "onboard"])],
            Self::AgentsDashboard => &[Sub(&["agents-dashboard"])],
            Self::ChangeSets => &[Sub(&["changes"])],
            Self::Ledger => &[Sub(&["ledger"])],
            Self::CloudAccount => &[Sub(&["account"])],
            Self::FolderSecrets => &[
                Sub(&["secret", "rm"]),
                Sub(&["secret", "grant"]),
                Sub(&["secret", "read"]),
                Sub(&["secret", "exec"]),
                Arg(&["secret", "set"], "folder"),
            ],
            Self::Marketplace => &[
                Sub(&["plugin", "publish"]),
                Sub(&["plugin", "browse"]),
                Sub(&["plugin", "search"]),
            ],
            Self::McpClient => &[
                Sub(&["events", "mcp-config"]),
                Arg(&["view", "open"], "mcp"),
            ],
            Self::Routines => &[Sub(&["routine"])],
            Self::AgentMesh => &[
                Sub(&["agent", "create"]),
                Sub(&["agent", "flags"]),
                Sub(&["agent", "fleet"]),
                Sub(&["agent", "history"]),
                Sub(&["agent", "next"]),
                Sub(&["agent", "reopen"]),
                Sub(&["agent", "result"]),
                Sub(&["agent", "spawn"]),
                Sub(&["agent", "start"]),
                Sub(&["agent", "team"]),
                Sub(&["agent", "wait"]),
            ],
            Self::CliOperations
            | Self::WasmApps
            | Self::PageGrid
            | Self::TreeAtlas
            | Self::Daw
            | Self::MediaIo
            | Self::Accessibility => &[],
        }
    }
}
