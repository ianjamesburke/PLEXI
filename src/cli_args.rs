use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "plexi", about = "Plexi — the last app you'll ever need", version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    /// Profile name (e.g. alpha, beta)
    #[arg(long, global = true, hide = true)]
    pub profile: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Open a workspace directory (plexi <path>)
    pub workspace_path: Option<String>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run a named command from .plexi/commands.toml
    Run {
        command: String,
    },
    /// Workspace management
    Workspace {
        #[command(subcommand)]
        cmd: WorkspaceCmd,
    },
    /// Secret management
    Secret {
        #[command(subcommand)]
        cmd: SecretCmd,
    },
    /// App management
    App {
        #[command(subcommand)]
        cmd: AppCmd,
    },
    /// Install an app
    Install {
        /// Source spec (e.g. github:user/repo or bare app id)
        spec: Option<String>,
        /// Install from a pack file or 'core'
        #[arg(long)]
        pack: Option<String>,
    },
    /// Uninstall an app
    Uninstall {
        /// App id to remove
        id: String,
        /// Skip confirmation prompt
        #[arg(long = "yes", short = 'y')]
        yes: bool,
    },
    /// Update apps or self
    Update {
        #[command(subcommand)]
        subcommand: Option<UpdateCmd>,
    },
    /// List installed apps
    List,
    /// Validate a Plexi app directory
    Validate {
        /// Path to validate (default: current directory)
        #[arg(default_value = ".")]
        path: String,
    },
    /// Pack management
    Pack {
        #[command(subcommand)]
        cmd: PackCmd,
    },
    /// Send a notification [requires PLEXI_SOCKET — run inside a Plexi pane]
    Notify {
        /// Notification title (required)
        #[arg(long)]
        title: String,
        /// Notification body
        #[arg(long, default_value = "")]
        body: String,
        /// Level: info, warn, or error
        #[arg(long, default_value = "info")]
        level: String,
        /// Choice option. Format: `key:Label` (returns key when selected) or
        /// `Label:pane_focus:<pane_id>` (navigates to pane and returns label).
        /// Repeatable.
        #[arg(long = "choice")]
        choices: Vec<String>,
        /// Timeout in seconds (0 = no timeout)
        #[arg(long, default_value = "0")]
        timeout: u64,
    },
    /// Pane management [requires PLEXI_SOCKET — run inside a Plexi pane]
    Pane {
        #[command(subcommand)]
        cmd: PaneCmd,
    },
    /// Open an app or terminal pane
    Open {
        /// App or pane type id
        type_id: String,
        /// Layout hint
        #[arg(long)]
        layout: Option<String>,
        /// Extra args passed to the app
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },
    /// Descriptor probe
    Descriptor {
        #[command(subcommand)]
        cmd: DescriptorCmd,
    },
    /// CLI registry
    Registry {
        #[command(subcommand)]
        cmd: RegistryCmd,
    },
    /// Context management [requires PLEXI_SOCKET — run inside a Plexi pane]
    Context {
        #[command(subcommand)]
        cmd: ContextCmd,
    },
    /// [deprecated] Shell integration — currently a no-op; prints a notice
    ShellInit {
        /// Shell name (zsh, bash, fish)
        #[arg(long)]
        shell: Option<String>,
        /// Shell name as positional (alternative to --shell)
        shell_pos: Option<String>,
    },
    /// Print shell completion script
    Completions {
        /// Shell name (zsh, bash, fish)
        shell: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum WorkspaceCmd {
    /// Initialise a .plexi/ workspace in the current directory
    Init,
}

#[derive(Subcommand)]
pub enum SecretCmd {
    /// Store a secret (prompts for value with hidden input; walks up to nearest .plexi/ workspace).
    /// Use --from-env to read from an env var, or --global to store cross-workspace.
    Set {
        /// Name of the secret (also the env var name when using --from-env)
        friendly_name: String,
        /// Read value from the environment variable named FRIENDLY_NAME instead of prompting
        #[arg(long = "from-env")]
        from_env: bool,
        /// Store globally (cross-workspace) rather than scoped to the nearest .plexi/ workspace
        #[arg(long)]
        global: bool,
    },
    /// List stored secrets
    List,
    /// Delete a secret
    Delete {
        friendly_name: String,
    },
}

#[derive(Subcommand)]
pub enum AppCmd {
    /// Scaffold a new app
    Init {
        name: String,
        #[arg(long, default_value = "python")]
        lang: String,
    },
    /// Install an app from GitHub
    Install {
        source: String,
    },
    /// Uninstall an app
    Uninstall {
        id: String,
    },
    /// List installed apps
    List,
}

#[derive(Subcommand)]
pub enum UpdateCmd {
    /// Update installed apps
    Apps {
        /// Specific app id to update (omit to update all)
        id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum PackCmd {
    /// Export current apps as a pack file
    Export {
        path: String,
    },
}

#[derive(Subcommand)]
pub enum PaneCmd {
    /// Set the title of the current pane
    SetTitle {
        name: String,
    },
    /// List all open panes as a JSON array
    List,
    /// Move UI focus to a pane by ID
    ///
    /// NOTE: This moves the *user's visual focus* to the target pane — it does NOT
    /// relocate the agent's execution context. An agent calling this from pane A
    /// remains in pane A after the call; only the user sees focus shift to pane B.
    Focus {
        /// Pane ID (from `plexi pane list`)
        pane_id: u64,
    },
    /// Close a pane by ID
    Close {
        /// Pane ID (from `plexi pane list`)
        pane_id: u64,
    },
}

#[derive(Subcommand)]
pub enum DescriptorCmd {
    /// Probe a CLI for its Plexi descriptor
    Probe {
        command: String,
        #[arg(long = "no-registry")]
        no_registry: bool,
        #[arg(long = "no-crawl")]
        no_crawl: bool,
        /// Extra args forwarded to the probed CLI
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum RegistryCmd {
    /// Watch installed CLIs for descriptor drift
    Watch {
        /// Only check this CLI
        cli: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ContextCmd {
    /// Create a new context, optionally opening a path
    New {
        path: Option<String>,
    },
    /// Open a context at a path
    Open {
        path: Option<String>,
    },
    /// Set the root directory for the active context
    SetRoot {
        path: Option<String>,
    },
}
