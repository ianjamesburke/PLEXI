use clap::{Parser, Subcommand, builder::ValueHint, builder::styling::{AnsiColor, Effects, Styles}};

fn plexi_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Green.on_default() | Effects::BOLD)
        .usage(AnsiColor::Green.on_default() | Effects::BOLD)
        .literal(AnsiColor::Cyan.on_default() | Effects::BOLD)
        .placeholder(AnsiColor::Yellow.on_default() | Effects::DIMMED)
        .error(AnsiColor::Red.on_default() | Effects::BOLD)
        .valid(AnsiColor::Green.on_default())
        .invalid(AnsiColor::Red.on_default())
}

#[derive(Parser)]
#[command(
    name = "plexi",
    about = "Plexi — the last app you'll ever need",
    version = env!("CARGO_PKG_VERSION"),
    after_help = "Get started: plexi demo | Docs: https://plexiapp.com/docs",
    styles = plexi_styles(),
)]
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
    // ── Workspace ─────────────────────────────────────────────────────────────
    /// Run a named command from your project's .plexi/commands.toml file.
    ///
    /// Define shell commands in .plexi/commands.toml and run them by name here.
    /// Any secrets listed in the command definition are injected as environment variables automatically.
    ///
    /// Example: plexi run dev
    #[command(next_help_heading = "Workspace")]
    Run {
        /// Command name to run (omit to list available commands)
        command: Option<String>,
    },
    /// Set up a .plexi/ workspace in your project folder.
    ///
    /// Run this once inside your project directory to enable workspace-scoped secrets and commands.
    Workspace {
        #[command(subcommand)]
        cmd: WorkspaceCmd,
    },
    /// Store and retrieve secrets (API keys, passwords, tokens) for your project.
    ///
    /// Secrets are saved to your system keychain and injected as environment variables when you run commands.
    /// Use `plexi workspace init` first to scope secrets to a project.
    Secret {
        #[command(subcommand)]
        cmd: SecretCmd,
    },
    /// Manage workspace routines — scheduled shell commands.
    ///
    /// Routines are declared in `.plexi/routines.toml` and run automatically on schedule.
    /// **Requires Plexi to be running** — there is no background daemon. Routines only fire
    /// while the host process is open.
    ///
    /// Use `plexi routine list` to see configured routines, or `plexi routine run <name>` to fire one manually.
    Routine {
        #[command(subcommand)]
        cmd: RoutineCmd,
    },
    /// Manage the active context (the folder and project scope tied to the current pane).
    Context {
        #[command(subcommand)]
        cmd: ContextCmd,
    },

    // ── Apps ──────────────────────────────────────────────────────────────────
    /// Manage your Plexi apps — open, install, list, scaffold, and inspect.
    #[command(next_help_heading = "Apps")]
    App {
        #[command(subcommand)]
        cmd: AppCmd,
    },
    /// Watch installed CLI tools for changes to their available commands and options.
    Registry {
        #[command(subcommand)]
        cmd: RegistryCmd,
    },

    // ── Panes ─────────────────────────────────────────────────────────────────
    /// Control panes — list, focus, send input, capture output, and more.
    #[command(next_help_heading = "Panes")]
    Pane {
        #[command(subcommand)]
        cmd: PaneCmd,
    },
    /// Open a plain terminal pane.
    Terminal {
        /// Optional shell command to run inside the new terminal
        cmd: Option<String>,
        /// Close the pane automatically when the command finishes
        #[arg(long, short = 'e')]
        ephemeral: bool,
        /// Where to place the new pane: split_h (right), split_left (left), split_v (below), split_right, split_below, split_above, tab, or new_window
        #[arg(long, value_parser = ["split_h", "split_left", "split_right", "split_v", "split_below", "split_above", "tab", "new_window"])]
        layout: Option<String>,
        /// Open the new pane relative to this pane ID instead of the focused pane
        #[arg(long)]
        from_pane_id: Option<u64>,
        /// Directory to open the terminal in
        #[arg(long, value_hint = ValueHint::DirPath)]
        cwd: Option<String>,
        /// Keep focus on the current pane instead of jumping to the new one
        #[arg(long)]
        no_focus: bool,
    },
    /// Send a notification to the Plexi UI.
    Notify {
        /// Notification title (required)
        #[arg(long)]
        title: String,
        /// Notification body text
        #[arg(long, default_value = "")]
        body: String,
        /// Severity level: info, warn, or error
        #[arg(long, default_value = "info", value_parser = ["info", "warn", "error"])]
        level: String,
        /// Add a clickable button to the notification. Format: `key:Label` (returns key when clicked) or
        /// `Label:pane_focus:<pane_id>` (switches focus to that pane when clicked).
        /// Repeatable.
        #[arg(long = "choice")]
        choices: Vec<String>,
        /// Action to perform on the host when a button is clicked. Format: `key:action_type:action_arg`.
        /// Repeatable. The host runs this even after the process that sent the notification has exited.
        #[arg(long = "host-action")]
        host_actions: Vec<String>,
        /// How many seconds before the notification disappears (0 = stays until dismissed)
        #[arg(long, default_value = "0")]
        timeout: u64,
        /// Which panes see this notification: window, context, or global (default: global)
        #[arg(long, value_name = "SCOPE", default_value = "global", value_parser = ["window", "context", "global"])]
        scope: Option<String>,
    },

    // ── System ────────────────────────────────────────────────────────────────
    /// Print a shell completion script to stdout.
    ///
    /// Example: plexi completions zsh >> ~/.zshrc
    #[command(next_help_heading = "System")]
    Completions {
        /// Shell name: zsh, bash, or fish
        shell: Option<String>,
    },
    /// Check your Plexi config file for errors.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Browse and open scratchpad notes created with Cmd+Shift+Space.
    ///
    /// Each scratchpad session writes a timestamped file to `<config_dir>/notes/`.
    /// Use `plexi notes list` to print note paths, or `plexi notes open` to pick one with fzf.
    Notes {
        #[command(subcommand)]
        cmd: Option<NotesCmd>,
    },
    /// Interactive keybinding tutorial — learn split and navigate in real time.
    ///
    /// Walk through two fundamental Plexi interactions inside a live pane:
    /// split a pane (⌘D) and navigate between panes (⌘L / ⌘H).
    /// Must be run inside a Plexi pane (PLEXI_PANE_ID must be set).
    Demo,
    /// Update installed apps or Plexi itself.
    ///
    /// Run with the `apps` subcommand to update one or all installed apps.
    /// Run with no subcommand to update the Plexi binary itself.
    Update {
        #[command(subcommand)]
        subcommand: Option<UpdateCmd>,
    },
    /// Uninstalls the app, CLI, and optionally your profile data.
    ///
    /// Removes the current channel's app bundle (/Applications/Plexi.app), CLI binary (/usr/local/bin/plexi),
    /// and shell completions. Your profile directory (~/.plexi/) holds your settings, secrets,
    /// and app configurations — you will be asked whether to keep it.
    ///
    /// Example: plexi uninstall
    Uninstall {
        /// Keep your profile directory (~/.plexi/) — your settings, secrets, and app data stay on disk
        #[arg(long = "keep-data")]
        keep_data: bool,
        /// Skip the confirmation prompt and proceed immediately (removes data unless --keep-data is set)
        #[arg(long = "yes", short = 'y')]
        yes: bool,
    },

    // ── Hidden ────────────────────────────────────────────────────────────────
    /// Descriptor probe
    #[command(hide = true)]
    Descriptor {
        #[command(subcommand)]
        cmd: DescriptorCmd,
    },
}

#[derive(Subcommand)]
pub enum WorkspaceCmd {
    /// Set up a .plexi/ workspace in the current directory.
    ///
    /// Run this once inside your project folder. It creates a .plexi/workspace.toml
    /// so that secrets and commands are scoped to this project.
    Init,
}

#[derive(Subcommand)]
pub enum SecretCmd {
    /// Save a secret to your keychain.
    ///
    /// Plexi will prompt you to type the value (hidden). The secret is stored in your
    /// system keychain and can be injected into commands automatically.
    ///
    /// Use --from-env to read the value from an existing environment variable instead of typing it.
    /// Use --global to make the secret available across all projects, not just the current one.
    Set {
        /// Name for this secret — also the environment variable name it will be injected as
        friendly_name: String,
        /// Read the value from the environment variable named FRIENDLY_NAME instead of prompting
        #[arg(long = "from-env")]
        from_env: bool,
        /// Store this secret globally so it's available in all projects, not just this one
        #[arg(long)]
        global: bool,
        /// Use a different name for the Keychain entry than the canonical env var name.
        ///
        /// Useful when the Keychain entry already exists under a different name.
        /// Example: plexi secret set OPENAI_API_KEY --alias openai_personal
        #[arg(long)]
        alias: Option<String>,
    },
    /// Print a stored secret's value to stdout.
    ///
    /// Looks up the secret for the current project first, then falls back to the global store.
    /// Use --global to read only from the global store.
    Get {
        /// Name of the secret to read
        friendly_name: String,
        /// Read from the global store only, skipping the project-level lookup
        #[arg(long)]
        global: bool,
    },
    /// Show all secrets stored for this project.
    List,
    /// Delete a stored secret.
    Delete {
        friendly_name: String,
    },
}

#[derive(Subcommand)]
pub enum AppCmd {
    /// Open an app or tool in a new pane.
    ///
    /// Pass an app id (e.g. `plexi app open snake`) to open an installed app.
    /// Use `--mcp` to wrap an MCP server, or `--cli` to open any CLI tool with a Plexi UI.
    Open {
        /// App id to open (mutually exclusive with --mcp and --cli)
        #[arg(conflicts_with_all = ["mcp", "cli"])]
        type_id: Option<String>,
        /// Wrap a stdio MCP server in a Plexi pane.
        ///
        /// Example: plexi app open --mcp npx @modelcontextprotocol/server-filesystem /tmp
        #[arg(long, num_args = 1.., value_name = "CMD", allow_hyphen_values = true, conflicts_with = "cli")]
        mcp: Vec<String>,
        /// Wrap a CLI tool in a Plexi pane with a visual UI.
        ///
        /// Example: plexi app open --cli git
        #[arg(long, value_name = "BINARY", conflicts_with = "mcp")]
        cli: Option<String>,
        /// Where to place the new pane: split_h (right), split_left (left), split_v (below), split_right, split_below, split_above, tab, new_window, or overlay
        #[arg(long, value_parser = ["split_h", "split_left", "split_right", "split_v", "split_below", "split_above", "tab", "new_window", "overlay"])]
        layout: Option<String>,
        /// Open the new pane relative to this pane ID instead of the focused pane
        #[arg(long)]
        from_pane_id: Option<u64>,
        /// Extra arguments passed through to the app (only valid with an app id)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, conflicts_with_all = ["mcp", "cli"])]
        extra_args: Vec<String>,
    },
    /// Install an app from a local path, a remote source, or a pack file.
    ///
    /// Local path: `plexi app install ./my-app` — copies the app dir into Plexi's store.
    /// Remote source: `plexi app install github:owner/repo` — fetches and installs from GitHub.
    /// Pack file: `plexi app install --pack core` — installs from a pack file or the built-in core pack.
    /// Workspace pack: `plexi app install` (no args) — installs from .plexi/apps.toml.
    Install {
        /// Source to install: a local path, GitHub spec (github:owner/repo), or bare app id.
        /// Omit to install from the workspace pack (.plexi/apps.toml).
        spec_or_path: Option<String>,
        /// Install from a pack file or 'core'
        #[arg(long)]
        pack: Option<String>,
        /// Pin the app to a specific version (e.g. --version 1.2.3).
        /// The pinned version is recorded and shown by `plexi app update`.
        #[arg(long, value_name = "SEMVER")]
        version: Option<String>,
    },
    /// Remove an installed app by id.
    ///
    /// Example: plexi app uninstall github-tree
    #[command(aliases = ["remove", "delete"])]
    Uninstall {
        /// App id to remove (use `plexi app list` to see installed ids)
        id: String,
        /// Skip the confirmation prompt
        #[arg(long = "yes", short = 'y')]
        yes: bool,
    },
    /// Show all installed apps with their versions.
    List,
    /// Render an app to a PNG image without opening the UI (useful for screenshots and testing).
    Render {
        /// App id to render (e.g. "snake")
        id: String,
        /// Image dimensions as WxH (e.g. 500x500)
        #[arg(long, default_value = "800x600")]
        size: String,
        /// Pre-seed the app's state from a JSON file before rendering
        #[arg(long, value_hint = ValueHint::FilePath)]
        state: Option<String>,
        /// Where to save the PNG (default: stdout)
        #[arg(long, value_hint = ValueHint::FilePath)]
        output: Option<String>,
    },
    /// Show details about an installed app: id, name, version, and available tools.
    Info {
        id: String,
    },
    /// Create a new app from a template.
    ///
    /// Scaffolds the folder structure and files you need to build a Plexi app.
    /// Use --lang to pick the language (default: python).
    Init {
        name: String,
        #[arg(long, default_value = "python")]
        lang: String,
        /// Open the new pane relative to this pane ID instead of the focused pane.
        /// Defaults to PLEXI_PANE_ID if set in the environment.
        #[arg(long)]
        from_pane_id: Option<u64>,
    },
    /// Run an app directly from a local directory without installing or linking.
    ///
    /// Opens the app in a pane immediately. Edits to the app take effect on next launch.
    /// Replaces `plexi app link` for development workflows.
    Run {
        /// Path to the app folder containing manifest.toml
        path: String,
        /// Open the new pane relative to this pane ID instead of the focused pane.
        /// Defaults to PLEXI_PANE_ID if set in the environment.
        #[arg(long)]
        from_pane_id: Option<u64>,
    },
    /// Check a Plexi app directory for errors before publishing or installing.
    Validate {
        /// Path to check (default: current directory)
        #[arg(default_value = ".", value_hint = ValueHint::DirPath)]
        path: String,
    },
    /// Export your currently installed apps as a single TOML snapshot for sharing or backup.
    ///
    /// Like `pip freeze` — captures exactly what's installed so you can replay it later with `plexi app install`.
    Freeze {
        /// Destination path for the TOML snapshot file
        #[arg(value_hint = ValueHint::FilePath)]
        path: String,
    },
    /// Package the app in the current directory and print the registry submission payload.
    ///
    /// Validates all required fields, checks that the entry file exists, and prints the
    /// JSON payload that would be submitted to the Plexi app registry.
    ///
    /// The actual HTTP submission is future work — this command proves the manifest is
    /// correct and ready for publishing.
    Publish {
        /// Print the payload without submitting (currently the only mode — actual registry upload is future work)
        #[arg(long)]
        dry_run: bool,
    },
    /// Check installed apps for available updates.
    ///
    /// Compares each app's recorded installed version against the version in its manifest.
    /// In v1 this is a local check only — no network calls are made.
    /// Use `plexi update apps` for git-checkout apps.
    Update {
        /// App id to check (omit to check all installed apps)
        id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum UpdateCmd {
    /// Pull the latest version of your installed apps.
    ///
    /// Omit the app id to update all installed apps at once.
    Apps {
        /// App id to update (omit to update all installed apps)
        id: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum PaneCmd {
    /// Rename a pane.
    ///
    /// With one argument, renames the current pane: plexi pane name "My Project"
    /// With two arguments, renames any pane by id: plexi pane name 42 "My Project"
    Name {
        /// Pane id (from `plexi pane list`) or the new name if renaming the current pane
        first: String,
        /// New name when a pane id is given as the first argument
        second: Option<String>,
    },
    /// Deprecated: use `plexi pane name` instead.
    #[command(hide = true)]
    SetTitle {
        /// Pane id (from `plexi pane list`) or title when used alone
        first: String,
        /// Title when pane-id is given as the first argument
        second: Option<String>,
    },
    /// List all open panes as a JSON array.
    ///
    /// Filter by context: `--context` (no value) returns panes in the caller's context
    /// (reads PLEXI_CONTEXT_ID from env). `--context <id>` filters to a specific context ID.
    List {
        /// Filter by context. With no argument, reads PLEXI_CONTEXT_ID from env (caller's context).
        /// With a numeric argument, returns panes in that specific context.
        #[arg(long, value_name = "ID", num_args = 0..=1, default_missing_value = "current")]
        context: Option<String>,
    },
    /// Move the visible focus to a specific pane.
    ///
    /// This moves what the user sees on screen — it does not change which pane an agent is running in.
    /// An agent calling this from pane A remains in pane A; the user just sees pane B highlighted.
    Focus {
        /// Pane id to focus (from `plexi pane list`)
        pane_id: u64,
    },
    /// Close a pane. Omit the pane id to close the pane you are currently in.
    Close {
        /// Pane id to close (from `plexi pane list`). Defaults to the current pane if not given.
        pane_id: Option<u64>,
    },
    /// Type text into another pane as if it came from the keyboard.
    ///
    /// Use `\n` in the text to press Enter (which submits a command).
    ///
    /// Example: plexi pane send 42 "git status\n"
    Send {
        /// Pane id to send text to (from `plexi pane list`)
        pane_id: u64,
        /// Text to type into the pane (use `\n` for Enter)
        text: String,
    },
    /// Print the id of the pane you are currently in.
    ///
    /// Useful in scripts: MY_PANE=$(plexi pane self)
    #[command(name = "self")]
    Self_,
    /// Print details about the current pane as JSON.
    Info,
    /// Capture the last N lines of a pane's output as a JSON array.
    ///
    /// Defaults to the current pane when no pane id is given.
    ///
    /// Example: plexi pane capture --lines 50 42
    Capture {
        /// Pane id to capture output from. Defaults to the current pane.
        pane_id: Option<u64>,
        /// How many lines to read from the end of the output
        #[arg(long, default_value = "50")]
        lines: usize,
        /// Preserve trailing empty lines (by default they are stripped)
        #[arg(long)]
        full_output: bool,
        /// Read only lines written after this cursor value. Get the cursor from a
        /// previous capture response. When set, the response is always JSON object format.
        #[arg(long, value_name = "CURSOR")]
        from_cursor: Option<u64>,
    },
    /// Send a key press to a pane.
    ///
    /// For terminal panes, injects the keystroke into the terminal.
    /// For app panes, delivers a structured key event.
    ///
    /// Key formats: single character ("h"), named key ("enter", "escape", "space",
    /// "up", "down", "left", "right", "backspace"), or chord ("ctrl+c").
    ///
    /// Example: plexi pane key 42 enter
    Key {
        /// Pane id to send the key to (from `plexi pane list`)
        pane_id: u64,
        /// Key to press
        key: String,
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
        /// Output raw descriptor JSON (fresh --help parse, no cache)
        #[arg(long = "json")]
        json: bool,
        /// Extra args forwarded to the probed CLI
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum RegistryCmd {
    /// Check installed CLI tools for changes to their help output and update Plexi's knowledge of them.
    Watch {
        /// Only check this one CLI tool instead of all of them
        cli: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ContextCmd {
    /// Open a new context with an optional name.
    New {
        /// Name for the new context. Defaults to the directory basename.
        name: Option<String>,
        /// Root path for the new context. Defaults to current working directory.
        #[arg(long)]
        path: Option<String>,
        /// Create as a child of the named context. Defaults to current context if inside one.
        #[arg(long)]
        parent: Option<String>,
    },
    /// Switch the current pane to a context at the given path.
    Open {
        path: Option<String>,
    },
    /// Change the root folder for the active context.
    SetRoot {
        path: Option<String>,
    },
    /// Print the id and name of the current pane's context as JSON.
    Current,
    /// Set the description for the active context
    Describe {
        /// Description text
        text: String,
    },
    /// Zoom into a sub-context by its numeric context_id.
    Zoom {
        context_id: u64,
    },
    /// Zoom out of the current sub-context to the parent.
    ZoomOut,
}

#[derive(Subcommand)]
pub enum ConfigCmd {
    /// Validate your config.toml and report any errors.
    Check,
    /// Open config.toml in your $EDITOR.
    Edit,
    /// Print the resolved value of a config key to stdout.
    ///
    /// Supports dotted keys: agents.low, agents.medium, agents.high.
    /// Returns the effective value (user setting or built-in default).
    Get {
        /// Dotted key to retrieve (e.g. agents.medium).
        key: String,
    },
    /// Overwrite config.toml with the built-in default template.
    ///
    /// Creates a backup at config.toml.bak before overwriting.
    Reset,
}

#[derive(Subcommand)]
pub enum RoutineCmd {
    /// List routines defined in .plexi/routines.toml with their schedule and next fire time.
    List,
    /// Manually trigger a named routine from .plexi/routines.toml.
    Run {
        /// Name of the routine to run
        name: String,
    },
}

#[derive(Subcommand)]
pub enum NotesCmd {
    /// Print paths of all scratchpad notes, newest first.
    List,
    /// Open a note picker with fzf in the focused terminal pane.
    ///
    /// Requires fzf to be installed. Falls back to printing the notes directory when fzf
    /// is not available or PLEXI_SOCKET is not set.
    Open,
}
