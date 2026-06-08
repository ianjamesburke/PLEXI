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
    /// Manage workspace agent definitions.
    ///
    /// Install agent definitions from the global registry (`~/.plexi/agents/`) into the
    /// current workspace's `.plexi/agents/` directory, each with scoped memory and logs.
    Agent {
        #[command(subcommand)]
        cmd: AgentCmd,
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

    // ── AI ────────────────────────────────────────────────────────────────────
    /// AI configuration and diagnostics — scan hardware, check integrations, recommend models.
    #[command(next_help_heading = "AI")]
    Ai {
        #[command(subcommand)]
        cmd: AiCmd,
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
    /// Audit all installed apps for capability and config gaps.
    ///
    /// Checks every installed app's declared capabilities against your current config.toml
    /// and reports what's working and what needs to be configured. Use --json for scripting.
    Doctor {
        /// Output results as JSON (for scripting or agent use)
        #[arg(long)]
        json: bool,
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
    /// List completions for prefix-based open (hidden, used by shell completions)
    #[command(hide = true, name = "_complete-open")]
    CompleteOpen {
        /// Prefix to complete: "cli:", "mcp:", "app:", or empty for all
        prefix: String,
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
    ///
    /// Use --global to delete a globally-stored secret (one stored with `secret set --global`).
    /// Without --global, deletes the workspace-scoped entry for the current project.
    Delete {
        friendly_name: String,
        /// Delete from the global store instead of the project-scoped store
        #[arg(long)]
        global: bool,
    },
}

#[derive(Subcommand)]
pub enum AppCmd {
    /// Open an app or tool in a new pane.
    ///
    /// Pass an app id (e.g. `plexi app open snake`) or a path to an app directory
    /// containing a manifest.toml. Use `--mcp` to wrap an MCP server, or `--cli`
    /// to open any CLI tool with a Plexi UI.
    Open {
        /// App id or path to open (mutually exclusive with --mcp and --cli)
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
        /// Split below
        #[arg(long, short = 'd', conflicts_with_all = ["left", "up", "right", "tab", "window"])]
        down: bool,
        /// Split left
        #[arg(long, short = 'l', conflicts_with_all = ["down", "up", "right", "tab", "window"])]
        left: bool,
        /// Split up
        #[arg(long, short = 'u', conflicts_with_all = ["down", "left", "right", "tab", "window"])]
        up: bool,
        /// Split right
        #[arg(long, short = 'r', conflicts_with_all = ["down", "left", "up", "tab", "window"])]
        right: bool,
        /// New tab
        #[arg(long, conflicts_with_all = ["down", "left", "up", "right", "window"])]
        tab: bool,
        /// New window
        #[arg(long, conflicts_with_all = ["down", "left", "up", "right", "tab"])]
        window: bool,
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
    /// Render an app headlessly (JSON frame tree by default, or PNG with --png).
    Render {
        /// App id to render (e.g. "snake")
        id: String,
        /// Image dimensions as WxH (e.g. 500x500)
        #[arg(long, default_value = "800x600")]
        size: String,
        /// Pre-seed the app's state from a JSON file before rendering
        #[arg(long, value_hint = ValueHint::FilePath)]
        state: Option<String>,
        /// Where to save the output (default: stdout)
        #[arg(long, value_hint = ValueHint::FilePath)]
        output: Option<String>,
        /// Render to a PNG image instead of JSON (default: JSON)
        #[arg(long)]
        png: bool,
    },
    /// Show details about an installed app: id, name, version, and available tools.
    Info {
        id: String,
    },
    /// Create a new app from a template.
    ///
    /// Scaffolds the folder structure and files you need to build a Plexi app,
    /// then opens it in a split-right pane so you can edit code alongside the
    /// running app.
    ///
    /// By default, the app is placed in your workspace's app directory. If no
    /// workspace is detected, pass --global to scaffold into the global registry.
    ///
    /// Use --no-open to scaffold without opening.
    #[command(after_long_help = r#"APP DEVELOPMENT GUIDE:

  Two rendering modes (pick one per app):
    view(self)         Declarative UI trees: forms, lists, dashboards
    on_render(self, ctx)  Canvas drawing: games, animations, visualizations

  UI components (view mode):
    Read plexi_sdk/ui.py for the full API. Key widgets:
    AppBar, Column, Row, Label, Spacer, FooterKeys, SelectList, TextInput,
    Card, Section, Tabs, Grid, Toggle, ScrollLog, ChatBubble, InfoTable,
    FormField, ButtonRow, ProgressBar, Clickable, Divider, Scrollable

  Key names (use these exact strings in on_key):
    space, return, escape, up, down, left, right, backspace, tab
    a-z (lowercase), plus, minus, equals, f1-f12

  State persistence:
    self.state.get("key", default)   Load on init
    self.state.save({"key": val})    Persist after changes

  Canvas API (on_render mode only):
    ctx.rect/text/circle/line | self.w, self.h | ctx.elapsed
    self.emit.schedule_render(after_ms=16)  # game loop

  Hooks (override any of these):
    on_init(self)              Called once at startup
    on_key(self, key, mods)    Keyboard input
    on_click(self, x, y, btn)  Mouse input
    on_escape(self) -> bool    Return True to consume
    on_text_submitted(self, id, text)  TextInput submission
    on_path_changed(self, cwd) Working directory changed
    on_shutdown(self)          Cleanup

  Emit methods:
    self.emit.schedule_render()         Request a redraw
    self.emit.notify(title, priority, body)
    self.emit.info/warn/error(msg)      Logging
    self.emit.http_get(url)             Network requests
    self.emit.ai_query(tier, sys, msgs) LLM queries

  Headless testing:
    plexi app render <id>                           JSON frame tree to stdout
    plexi app render <id> --png --output shot.png   PNG image to file
    Use --state file.json to pre-populate app state before on_init.
    The JSON is available via self.state.get() — no special handler needed.
"#)]
    Init {
        name: String,
        #[arg(long, default_value = "python")]
        lang: String,
        /// Scaffold into the global app registry instead of the workspace
        #[arg(long)]
        global: bool,
        /// Scaffold the app without opening it in a pane
        #[arg(long)]
        no_open: bool,
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
    /// Publish an app to the Plexi marketplace.
    ///
    /// The Plexi app marketplace is under development. This command will be available
    /// in a future release.
    Publish,
    /// Check installed apps for available updates.
    ///
    /// Compares each app's recorded installed version against the version in its manifest.
    /// In v1 this is a local check only — no network calls are made.
    /// Use `plexi update apps` for git-checkout apps.
    Update {
        /// App id to check (omit to check all installed apps)
        id: Option<String>,
    },
    /// Send a semantic action to a running app pane.
    ///
    /// Unlike `pane command` (which sends raw text), `app action` delivers a structured
    /// semantic event directly to the app's event handler — no keystroke simulation.
    ///
    /// Example: plexi app action 42 refresh
    /// Example: plexi app action 42 navigate-to /some/path
    #[command(name = "action")]
    Action {
        /// Pane id of the target app pane (from `plexi pane list`)
        pane_id: u64,
        /// Action name to invoke (e.g. "refresh", "navigate-to", "add-item")
        action: String,
        /// Optional arguments forwarded to the action handler
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
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
    /// Open a new terminal pane.
    ///
    /// Examples:
    ///   plexi pane new                          # empty terminal, split right
    ///   plexi pane new "npm run dev" -n "dev"   # terminal with command, named
    ///   plexi pane new -d                       # split below
    ///
    /// For apps use `plexi app open`. For MCP servers use `plexi app open --mcp`.
    New {
        /// Shell command to run in the new terminal
        cmd: Option<String>,
        /// Name the pane
        #[arg(long, short = 'n')]
        name: Option<String>,
        /// Split below instead of right
        #[arg(long, short = 'd', conflicts_with_all = ["left", "up", "right", "tab", "window", "overlay"])]
        down: bool,
        /// Split left
        #[arg(long, short = 'l', conflicts_with_all = ["down", "up", "right", "tab", "window", "overlay"])]
        left: bool,
        /// Split up
        #[arg(long, short = 'u', conflicts_with_all = ["down", "left", "right", "tab", "window", "overlay"])]
        up: bool,
        /// Split right (explicit, same as default)
        #[arg(long, short = 'r', conflicts_with_all = ["down", "left", "up", "tab", "window", "overlay"])]
        right: bool,
        /// New tab
        #[arg(long, conflicts_with_all = ["down", "left", "up", "window", "overlay"])]
        tab: bool,
        /// New window
        #[arg(long, conflicts_with_all = ["down", "left", "up", "tab", "overlay"])]
        window: bool,
        /// Overlay pane
        #[arg(long, conflicts_with_all = ["down", "left", "up", "tab", "window"])]
        overlay: bool,
        /// Pane ID to split relative to (default: focused pane)
        #[arg(long)]
        from: Option<u64>,
        /// Close the pane when the command finishes
        #[arg(long, short = 'e')]
        ephemeral: bool,
        /// Keep focus on the current pane
        #[arg(long)]
        no_focus: bool,
        /// Working directory
        #[arg(long, value_hint = ValueHint::DirPath)]
        cwd: Option<String>,
    },
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
    /// Send a shell command to a terminal pane as if typed from the keyboard.
    ///
    /// Use `--enter` to append a newline so the command is submitted immediately.
    ///
    /// Example: plexi pane command 42 "git status" --enter
    #[command(name = "command")]
    Command {
        /// Pane id to send the command to (from `plexi pane list`)
        pane_id: u64,
        /// Text to send to the pane
        text: String,
        /// Append a newline after the text, submitting it as a command
        #[arg(long, short = 'e')]
        enter: bool,
    },
    /// Return the current UI state of a pane as JSON.
    ///
    /// For app panes: returns a JSON object with a `frame` array of RenderCommands
    /// representing the last-rendered L1 UiNode tree. Agents can use this to inspect
    /// what an app is currently displaying.
    ///
    /// For terminal panes: returns a simple status object (type, title, pane_id).
    ///
    /// Example: plexi pane state 42
    State {
        /// Pane id to query (from `plexi pane list`)
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
    /// Push the focused pane into a new sub-context.
    Push {
        /// Name for the new sub-context. Defaults to the pane name.
        name: Option<String>,
    },
    /// List all open contexts as a JSON array.
    List,
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

#[derive(Subcommand)]
pub enum AgentCmd {
    /// Scaffold a new agent app with ai.query capability and a chat UI.
    ///
    /// Creates the app directory, manifest.toml (with ai.query pre-configured),
    /// and main.py from the agent template. Equivalent to the former
    /// `plexi app init --agent <name>`.
    ///
    /// Example: plexi agent init my-agent
    Init {
        /// App name (used as the directory name and app ID)
        name: String,
        /// Open the new pane relative to this pane ID instead of the focused pane.
        /// Defaults to PLEXI_PANE_ID if set in the environment.
        #[arg(long)]
        from_pane_id: Option<u64>,
    },
    /// Install an agent definition from the global registry into the current workspace.
    ///
    /// Copies `~/.plexi/agents/<name>/AGENT.md` into `.plexi/agents/<name>/AGENT.md`
    /// and creates `memory/` and `logs/` subdirectories for scoped agent state.
    ///
    /// Example: plexi agent add project-manager
    Add {
        /// Agent name (must exist in ~/.plexi/agents/<name>/AGENT.md)
        name: String,
    },
    /// Re-install an agent definition from the global registry, preserving memory and logs.
    ///
    /// Overwrites `.plexi/agents/<name>/AGENT.md` with the latest version from the global
    /// registry while leaving the `memory/` and `logs/` directories untouched.
    ///
    /// Example: plexi agent update project-manager
    Update {
        /// Agent name to update
        name: String,
    },
    /// List agents installed in the current workspace.
    List,
}

#[derive(Subcommand)]
pub enum AiCmd {
    /// Scan hardware and report recommended AI models.
    ///
    /// Detects your CPU, RAM/VRAM, and GPU, then recommends which local or cloud
    /// AI models are a good fit. Also checks whether Ollama is installed and running,
    /// lists any already-pulled models, and verifies OpenRouter configuration.
    ///
    /// Example: plexi ai doctor
    Doctor {
        /// Output results as JSON (for scripting or agent use)
        #[arg(long)]
        json: bool,
    },

    /// Interactive wizard to configure a local AI model via Ollama.
    ///
    /// Walks through Ollama installation detection, model recommendation based on your
    /// hardware, pulling the recommended model, and writing the [ai.ollama] section to
    /// your config.toml so Plexi apps can use it immediately.
    ///
    /// Example: plexi ai setup
    Setup,
}
