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
    CommandView,
    ChangeSets,
    Ledger,
    Relay,
    CloudAccount,
    FolderSecrets,
    WasmApps,
    PageGrid,
    Marketplace,
    Daw,
    MediaIo,
    Accessibility,
    McpClient,
    Routines,
}

impl ReleaseFeature {
    pub const ALL: &'static [ReleaseFeature] = &[
        Self::Assistant,
        Self::AiSetup,
        Self::CommandView,
        Self::ChangeSets,
        Self::Ledger,
        Self::Relay,
        Self::CloudAccount,
        Self::FolderSecrets,
        Self::WasmApps,
        Self::PageGrid,
        Self::Marketplace,
        Self::Daw,
        Self::MediaIo,
        Self::Accessibility,
        Self::McpClient,
        Self::Routines,
    ];

    /// The `[experiments]` config key and the name used by `plexi experiments`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Assistant => "assistant",
            Self::AiSetup => "ai_setup",
            Self::CommandView => "command_view",
            Self::ChangeSets => "change_sets",
            Self::Ledger => "ledger",
            Self::Relay => "relay",
            Self::CloudAccount => "cloud_account",
            Self::FolderSecrets => "folder_secrets",
            Self::WasmApps => "wasm_apps",
            Self::PageGrid => "page_grid",
            Self::Marketplace => "marketplace",
            Self::Daw => "daw",
            Self::MediaIo => "media_io",
            Self::Accessibility => "accessibility",
            Self::McpClient => "mcp_client",
            Self::Routines => "routines",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|f| f.key() == key)
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Assistant => "Host AI assistant pane and the `assistant` commands",
            Self::AiSetup => "Guided AI backend setup (`ai setup`, `ai onboard`)",
            Self::CommandView => "Agent runs dashboard pane and `command-view`",
            Self::ChangeSets => "Preview and accept agent edits (`changes`)",
            Self::Ledger => "Token and cost summary (`ledger`)",
            Self::Relay => "Phone pairing and turns through the Plexi relay",
            Self::CloudAccount => "Optional cloud account and history retention",
            Self::FolderSecrets => "Secrets injected per folder, with grants",
            Self::WasmApps => "Native Rust/WASM apps and the GPU canvas",
            Self::PageGrid => "2D page-grid minimap",
            Self::Marketplace => "App marketplace: browse, search, publish",
            Self::Daw => "DAW engine proof of concept",
            Self::MediaIo => "Audio and video player panes",
            Self::Accessibility => "AccessKit accessibility tree",
            Self::McpClient => "MCP client: host MCP config and `app open --mcp`",
            Self::Routines => "Scheduled commands (`routine`)",
        }
    }

    /// True when the host reads this experiment at runtime, so a change only
    /// takes effect after `plexi host restart`. Pure CLI gates re-read the
    /// config on every invocation.
    pub fn requires_restart(self) -> bool {
        !matches!(self, Self::AiSetup | Self::Ledger)
    }

    /// The clap paths this experiment owns. Empty for runtime-only surfaces
    /// that have no command of their own.
    pub fn owned_paths(self) -> &'static [GatePath] {
        use GatePath::{Arg, Sub};
        match self {
            Self::Assistant => &[Sub(&["assistant"])],
            Self::AiSetup => &[Sub(&["ai", "setup"]), Sub(&["ai", "onboard"])],
            Self::CommandView => &[Sub(&["command-view"])],
            Self::ChangeSets => &[Sub(&["changes"])],
            Self::Ledger => &[Sub(&["ledger"])],
            Self::Relay => &[Sub(&["relay"])],
            Self::CloudAccount => &[Sub(&["account"])],
            Self::FolderSecrets => &[
                Sub(&["secret", "rm"]),
                Sub(&["secret", "grant"]),
                Sub(&["secret", "read"]),
                Sub(&["secret", "exec"]),
                Arg(&["secret", "set"], "folder"),
            ],
            Self::Marketplace => &[
                Sub(&["app", "publish"]),
                Sub(&["app", "browse"]),
                Sub(&["app", "search"]),
            ],
            Self::McpClient => &[Sub(&["events", "mcp-config"]), Arg(&["app", "open"], "mcp")],
            Self::Routines => &[Sub(&["routine"])],
            Self::WasmApps | Self::PageGrid | Self::Daw | Self::MediaIo | Self::Accessibility => {
                &[]
            }
        }
    }
}

