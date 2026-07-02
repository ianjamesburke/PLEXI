use clap::{
    builder::styling::{AnsiColor, Effects, Styles},
    builder::ValueHint,
    Args, Parser, Subcommand,
};

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
    after_help = "Get started: plexi ai onboard | Docs: https://plexiapp.com/docs",
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
        /// Extra arguments forwarded to the command as $1, $2, … positional params
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        extra_args: Vec<String>,
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
    #[command(alias = "secrets")]
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
    /// Manage your Plexi marketplace account (only needed to publish or buy paid apps).
    ///
    /// Free apps install without an account. Login/signup require a configured
    /// auth backend; until then they fail closed with a clear message.
    #[command(next_help_heading = "Apps")]
    Account {
        #[command(subcommand)]
        cmd: AccountCmd,
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
    /// Subscribe to a Plexi app's event streams and receive brokered deliveries.
    ///
    /// Apps declare named event streams (e.g. `probe.tick`) and emit events on them.
    /// `plexi events subscribe <app_id> <stream>` opens a long-lived connection and
    /// prints one JSON line per delivered event to stdout (NDJSON) until interrupted.
    /// Subscriptions are brokered: the host stamps your identity from the pane you run
    /// in and checks permission before any event is delivered.
    #[command(next_help_heading = "Events")]
    Events {
        #[command(subcommand)]
        cmd: EventsCmd,
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
        /// Queue choice buttons without waiting for a selected value
        #[arg(long)]
        no_wait: bool,
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
    /// Capture a quick note to the inbox.
    ///
    /// Writes a timestamped note to `<config_dir>/notes/inbox/` with frontmatter
    /// capturing cwd, workspace, and context root. Triage later via Cmd+O, then t.
    ///
    /// Example: plexi note "remember to update the docs"
    Note {
        /// Note text to capture
        text: String,
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
    /// Launch, stop, or check a headless-friendly Plexi host from the CLI.
    ///
    /// `host start` launches this channel's app bundle detached from the
    /// calling shell, optionally seeding panes from a `--layout` TOML file
    /// or repeated `--pane` flags, then blocks until the host confirms it's
    /// ready. Works identically on alpha, beta, main, and PR builds — the
    /// channel is resolved from the running CLI binary's own name.
    Host {
        #[command(subcommand)]
        cmd: HostCmd,
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
    /// List run completions (hidden, used by shell completions)
    #[command(hide = true, name = "_complete-run")]
    CompleteRun,
}

#[derive(Subcommand)]
pub enum EventsCmd {
    /// Subscribe to an app's event stream and print delivered events as NDJSON.
    ///
    /// Opens a long-lived connection to the running Plexi instance and streams
    /// one JSON object per line to stdout: first a `subscribed` acknowledgement,
    /// then one line per delivered event. Runs until interrupted (Ctrl-C), at
    /// which point the host drops the subscription and its queued deliveries.
    ///
    /// Example: plexi events subscribe event-probe probe.tick --payload full
    Subscribe {
        /// App id that publishes the stream (e.g. `event-probe`).
        app_id: String,
        /// Stream name to subscribe to (e.g. `probe.tick`). Omit with --all to
        /// subscribe to every stream the app declares.
        stream: Option<String>,
        /// Subscribe to all of the app's declared streams instead of one.
        #[arg(long, conflicts_with = "stream")]
        all: bool,
        /// How much of each event to deliver: off, summary, full, or state-ref.
        #[arg(long, default_value = "full", value_parser = ["off", "summary", "full", "state-ref"])]
        payload: String,
        /// Trigger mode recorded on the subscription: never, conversation, ambient, or ask.
        #[arg(long, default_value = "conversation", value_parser = ["never", "conversation", "ambient", "ask"])]
        trigger: String,
        /// Only deliver events for this resource id (document/game/pane). Omit for any.
        #[arg(long)]
        resource: Option<String>,
    },
    /// List event streams currently declared by running apps.
    List {
        /// Output as JSON instead of a human-readable table.
        #[arg(long)]
        json: bool,
    },
    /// Print the host event MCP server config for an MCP-aware agent.
    ///
    /// Emits a `mcpServers` JSON block pointing at this instance's host MCP
    /// server (read from `PLEXI_HOST_MCP_PORT` / `PLEXI_HOST_MCP_TOKEN`), so a
    /// Claude Code or Codex agent in this pane can subscribe to app events
    /// natively over MCP.
    McpConfig,
}

#[derive(Subcommand)]
pub enum WorkspaceCmd {
    /// Set up a .plexi/ workspace in the current directory.
    ///
    /// Run this once inside your project folder. It creates a .plexi/workspace.toml
    /// so that secrets and commands are scoped to this project.
    Init,
    /// Remove pane slot files for panes that are no longer open.
    Clean {
        /// Print slot directories that would be removed without deleting them.
        #[arg(long)]
        dry_run: bool,
    },
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
    /// Show stored secrets.
    ///
    /// Inside a workspace, shows project secrets plus user-scope secrets.
    /// Outside a workspace, falls back to user-scope secrets.
    /// Use --global to show only user-scope secrets from any directory.
    List {
        /// Show only globally-stored user-scope secrets
        #[arg(long)]
        global: bool,
    },
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
        /// Open the new pane relative to this pane ID. Defaults to the calling
        /// pane (PLEXI_PANE_ID env), falling back to the focused pane.
        #[arg(long, value_name = "PANE_ID")]
        from: Option<u64>,
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
        /// With --pack: re-extract already-installed `local:` apps from this
        /// binary's embedded tree, replacing the installed copy. The update
        /// path for bundled core apps on stable channels.
        #[arg(long, requires = "pack")]
        refresh: bool,
        /// Pin a local path or package install to a specific version (e.g. --version 1.2.3).
        #[arg(long, value_name = "SEMVER")]
        version: Option<String>,
        /// Skip the trust-sheet confirmation prompt. Required for
        /// non-interactive (scripted) installs — without a terminal the
        /// install fails closed instead of proceeding silently.
        #[arg(long = "yes", short = 'y')]
        yes: bool,
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
        /// App id or local path to render (e.g. "snake" or "./my-app")
        app: String,
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
    /// Check a local app with manifest, scaffold metadata, SDK, and render-size checks.
    ///
    /// This is the compiler-like gate for generated Plexi apps. It checks the
    /// manifest, warns on missing or stale `plexi.scaffold.toml`, inspects
    /// Python SDK usage without importing app code, and renders the app at
    /// small and normal pane sizes. Run it with an explicit alpha or PR
    /// channel so the SDK/profile under test is not ambient.
    #[command(after_long_help = r#"APP CHECK LOOP:

  Use `plexi app check` as the verification gate; there is no `plexi app build`.

    Typical gates:
    PLEXI_CHANNEL=alpha plexi app test .
    PLEXI_CHANNEL=alpha plexi app check . --png-dir render-output/check
    PLEXI_CHANNEL=pr-123 plexi app check . --png-dir render-output/check
    plexi-pr-123 app check . --png-dir render-output/check

  State and behavior probes:
    PLEXI_CHANNEL=alpha plexi app render . --state fixtures/state.json
    PLEXI_CHANNEL=alpha plexi app render . --png --output render-output/shot.png
    PLEXI_SOCKET=$HOME/.plexi-alpha/notify.sock PLEXI_CHANNEL=alpha plexi app action <pane-id> <handler-id>
    PLEXI_SOCKET=$HOME/.plexi-alpha/notify.sock PLEXI_CHANNEL=alpha plexi pane key <pane-id> <key>
    PLEXI_SOCKET=$HOME/.plexi-alpha/notify.sock PLEXI_CHANNEL=alpha plexi pane state <pane-id>

  Logs:
    Inspect ~/.plexi-alpha/plexi.log or the matching ~/.plexi-pr-N/plexi.log.
    App code should use log.debug/info/warn/error intentionally.
"#)]
    Check {
        /// Local app directory to check (default: current directory)
        #[arg(default_value = ".", value_hint = ValueHint::DirPath)]
        path: String,
        /// Render size to check as WxH. Repeat to override the default matrix.
        #[arg(long = "size")]
        sizes: Vec<String>,
        /// Write PNG snapshots for each checked size into this directory.
        #[arg(long, value_hint = ValueHint::DirPath)]
        png_dir: Option<String>,
    },
    /// Run an app's AppHarness tests with `uv run pytest tests/`.
    ///
    /// Runs the Python tests in the app's `tests/` directory (the
    /// `tests/test_app.py` scaffolded by `plexi app init`). AppHarness spawns
    /// the app as a real subprocess and checks it renders without overlap; see
    /// `plexi_sdk/testing.py`. Exits nonzero on failure so CI can gate on it.
    Test {
        /// App directory to test (default: current directory)
        #[arg(default_value = ".", value_hint = ValueHint::DirPath)]
        path: String,
        /// Update stored snapshots instead of comparing against them.
        #[arg(long)]
        snapshot: bool,
    },
    /// Show details about an installed app: id, name, version, and available tools.
    Info { id: String },
    /// Create a new app from a template.
    ///
    /// Scaffolds the folder structure and files you need to build a Plexi app:
    /// manifest.toml, main.py, tests/test_app.py, AGENTS.md, .gitignore, and
    /// plexi.scaffold.toml drift metadata.
    ///
    /// By default, the app is placed in your workspace's app directory. If no
    /// workspace is detected, pass --global to scaffold into the global registry.
    ///
    /// Use --open to launch it in a split-right pane after scaffolding.
    #[command(after_long_help = r#"APP DEVELOPMENT GUIDE:

  Two rendering modes (pick one per app):
    view()                  Declarative UI trees: forms, lists, dashboards
    view() + Canvas(...)    Canvas drawing: games, animations, visualizations

  SDK v3 module functions:
    init(size, args)   Return startup effects such as SetTitle/SetState
    update(event)      Return effects after key, mouse, timer, and render events
    view()             Return the current component tree; keep it pure

  UI components:
    Read plexi_sdk/ui.py for the full API. Key widgets:
    AppBar, ActionBar, Column, Row, Label, Spacer, FooterKeys, SelectList, TextInput,
    Card, Section, Tabs, Grid, Toggle, ScrollLog, ChatBubble, InfoTable,
    FormField, ButtonRow, ProgressBar, Clickable, Divider, Scrollable,
    Canvas, CanvasRect, CanvasText, CanvasCircle

  Key names (use these exact strings in KeyEvent handlers):
    space, return, escape, up, down, left, right, backspace, tab
    a-z (lowercase), plus, minus, equals, f1-f12

  State:
    state.get("key", default)        Read runtime state
    SetState({"key": value})         Update process-local runtime state
    PersistState({"key": value})     Save state across app restarts

  Canvas apps:
    Return Canvas([...]) from view().
    For animation, return SetSchedulerMode("continuous", fps=60) from init()
    and update simulation state from RenderFrame events.

  Effects:
    SetTitle, SetStatus, SetTimer, SetSchedulerMode, SetState, PersistState,
    LogInfo/LogWarn/LogError, HttpRequest, AiQuery, FileRead, FileWrite

  Generated files:
    AGENTS.md              Agent-facing app contract and validation loop
    .gitignore             Ignores runtime/test/render noise, not source/tests/fixtures
    plexi.scaffold.toml    Machine-readable CLI/SDK/schema/runtime/template/profile metadata

  Development loop:
    Read AGENTS.md first.
    Use TDD and extend tests for behavior changes.
    Run `plexi app test .` regularly.
    New Python apps set `watch = true`; after `plexi app open .`, source edits
    should hot-reload into the same pane without reopening. Verify with
    `plexi pane state <pane-id>` and the host log's `hot_reload` lines.
    Use `plexi app check` as the final gate; do not look for `plexi app build`.

  Headless testing:
    PLEXI_CHANNEL=alpha plexi app check <path> --png-dir render-output/check
    PLEXI_CHANNEL=pr-123 plexi app check <path> --png-dir render-output/check
    plexi-pr-123 app check <path> --png-dir render-output/check
    PLEXI_CHANNEL=alpha plexi app render <path>                          JSON frame tree
    PLEXI_CHANNEL=alpha plexi app render <path> --png --output shot.png  PNG image
    Use --state file.json to pre-populate state before init().
    The state file is a plain JSON object, e.g. {"count": 3}.

  Runtime probes:
    PLEXI_SOCKET=$HOME/.plexi-alpha/notify.sock PLEXI_CHANNEL=alpha plexi app open <path>
    PLEXI_SOCKET=$HOME/.plexi-alpha/notify.sock PLEXI_CHANNEL=alpha plexi app action <pane-id> <handler-id>
    PLEXI_SOCKET=$HOME/.plexi-alpha/notify.sock PLEXI_CHANNEL=alpha plexi pane key <pane-id> <key>
    PLEXI_SOCKET=$HOME/.plexi-alpha/notify.sock PLEXI_CHANNEL=alpha plexi pane state <pane-id>
    Inspect ~/.plexi-alpha/plexi.log or ~/.plexi-pr-N/plexi.log for app logs.
"#)]
    Init {
        name: String,
        #[arg(long, default_value = "python")]
        lang: String,
        /// Scaffold into the global app registry instead of the workspace
        #[arg(long)]
        global: bool,
        /// Open the app in a split-right pane after scaffolding
        #[arg(long, conflicts_with = "no_open")]
        open: bool,
        /// Deprecated compatibility flag. App init no longer opens by default.
        #[arg(long)]
        #[arg(hide = true)]
        no_open: bool,
        /// Open the new pane relative to this pane ID. Defaults to the calling
        /// pane (PLEXI_PANE_ID env), falling back to the focused pane.
        #[arg(long, value_name = "PANE_ID")]
        from: Option<u64>,
    },
    /// Check a Plexi app directory or .plexipkg package for errors before publishing or installing.
    ///
    /// A directory is validated in place. A `.plexipkg` file is extracted to a
    /// temp dir with path-safety checks and verified end-to-end: descriptor,
    /// content hashes, manifest, entry point, and capability strings.
    Validate {
        /// App directory or .plexipkg file to check (default: current directory)
        #[arg(default_value = ".", value_hint = ValueHint::AnyPath)]
        path: String,
    },
    /// Show the trust sheet for a local app directory or .plexipkg package.
    ///
    /// Validates first (fail-closed), then prints what the app is, what
    /// runtime it uses with a blunt trust label, and every capability it
    /// declares — the same sheet shown before `plexi app install` proceeds.
    Inspect {
        /// App directory or .plexipkg file to inspect
        #[arg(value_hint = ValueHint::AnyPath)]
        path: String,
    },
    /// Build a distributable .plexipkg package from an app directory.
    ///
    /// Validates the directory first (fail-closed), then writes
    /// `<id>-<version>.plexipkg` containing the app files plus a generated
    /// PACKAGE.toml with per-file sha256 checksums.
    Package {
        /// App directory to package
        #[arg(value_hint = ValueHint::DirPath)]
        path: String,
        /// Output file path (default: ./<id>-<version>.plexipkg)
        #[arg(long, value_hint = ValueHint::FilePath)]
        out: Option<String>,
    },
    /// Export your currently installed apps as a single TOML snapshot for sharing or backup.
    ///
    /// Like `pip freeze` — captures exactly what's installed so you can replay it later with `plexi app install`.
    Freeze {
        /// Destination path for the TOML snapshot file
        #[arg(value_hint = ValueHint::FilePath)]
        path: String,
    },
    /// Validate, package, and submit an app to the Plexi marketplace.
    ///
    /// Reads the `[marketplace]` manifest section (publisher, visibility, price),
    /// validates the directory, builds a `.plexipkg`, and submits it. Without a
    /// configured `[marketplace].submit_url` the package is prepared locally but
    /// not uploaded — the artifact path is printed.
    Publish {
        /// App directory to publish (default: current directory)
        #[arg(default_value = ".", value_hint = ValueHint::DirPath)]
        path: String,
    },
    /// Browse every public app in the hosted marketplace.
    Browse,
    /// Search the public marketplace catalog.
    Search {
        /// Substring matched against app id, name, description, and tags
        query: String,
    },
    /// Inspect paid-app licenses stored on this machine.
    License {
        #[command(subcommand)]
        cmd: LicenseCmd,
    },
    /// Pull git-backed installed apps to their latest source revision.
    ///
    /// Canonical app update command. Resolves workspace-local apps when run
    /// inside a workspace, and skips installed apps that are not git checkouts.
    Update {
        /// App id to update (omit to update all installed apps visible here)
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
    /// Compatibility alias for `plexi app update`.
    ///
    /// Omit the app id to update all installed apps visible from the current workspace.
    Apps {
        /// App id to update (omit to update all installed apps)
        id: Option<String>,
    },
}

/// `plexi host <cmd>` — CLI-driven host launch with declarative boot state.
#[derive(Subcommand)]
pub enum HostCmd {
    /// Launch this channel's app bundle detached and wait for readiness.
    ///
    /// Errors if a host for this channel is already running. Seeds any
    /// declared panes via the spawn-queue before the app boots, so they
    /// appear on its first frame.
    ///
    /// Example: plexi-pr-2357 host start --pane 'cwd=/tmp,cmd=htop'
    Start {
        /// TOML file with `[[pane]]` tables to seed on boot
        #[arg(long)]
        layout: Option<String>,
        /// A pane to seed: 'cwd=<dir>[,cmd=<command>][,tab|window]'. Repeatable.
        #[arg(long = "pane")]
        panes: Vec<String>,
        /// Seconds to wait for the host to confirm readiness (default 15)
        #[arg(long)]
        timeout_secs: Option<u64>,
    },
    /// Stop the running host for this channel.
    ///
    /// Sends a clean shutdown request first, falling back to SIGTERM if the
    /// host doesn't confirm exit in time.
    Stop,
    /// Report whether this channel's host is running, its pid, socket path,
    /// and pane count.
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

/// `plexi account <cmd>` — marketplace account management.
#[derive(Subcommand)]
pub enum AccountCmd {
    /// Show whether you are logged in.
    Status,
    /// Log in to an existing marketplace account.
    Login {
        /// Account email (falls back to [marketplace].account_email in config)
        #[arg(long)]
        email: Option<String>,
    },
    /// Create a new marketplace account.
    Signup {
        /// Account email (falls back to [marketplace].account_email in config)
        #[arg(long)]
        email: Option<String>,
    },
    /// Log out and clear the local session.
    Logout,
}

/// `plexi app license <cmd>` — inspect paid-app licenses on this machine.
#[derive(Subcommand)]
pub enum LicenseCmd {
    /// List every stored paid-app license.
    List,
    /// Show one license in full.
    Show {
        /// App id whose license to show
        id: String,
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
        /// Pane ID to split relative to. Defaults to the calling pane
        /// (PLEXI_PANE_ID env), falling back to the focused pane.
        #[arg(long, value_name = "PANE_ID")]
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
    /// Print details about the current pane (or the previously focused pane) as JSON.
    Info {
        /// Return info for a previously focused pane. With no value, returns the immediately
        /// previous pane (step 1). Provide an integer N to walk back N steps in focus history.
        ///
        /// Examples:
        ///   plexi pane info --previous      # pane focused 1 step ago
        ///   plexi pane info --previous 3    # pane focused 3 steps ago
        #[arg(long, num_args(0..=1), default_missing_value = "1", value_name = "N")]
        previous: Option<u64>,
    },
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
    /// "up", "down", "left", "right", "backspace", "plus", "minus", "equals"),
    /// or chord ("ctrl+c").
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
    /// Manage host-managed named file slots for a pane.
    Slot {
        #[command(subcommand)]
        cmd: PaneSlotCmd,
    },
}

#[derive(Subcommand)]
pub enum PaneSlotCmd {
    /// Write bytes to a named pane slot. If content is omitted, stdin is read fully.
    Write {
        /// Slot name
        name: String,
        /// Optional content. If omitted, stdin is read fully.
        #[arg(allow_hyphen_values = true)]
        content: Option<String>,
        /// Pane id. Defaults to PLEXI_PANE_ID.
        #[arg(long)]
        pane_id: Option<u64>,
        /// Append to an existing slot instead of replacing it.
        #[arg(long, conflicts_with = "replace")]
        append: bool,
        /// Replace an existing slot.
        #[arg(long, conflicts_with = "append")]
        replace: bool,
    },
    /// Print raw bytes from a named pane slot.
    Read {
        /// Slot name
        name: String,
        /// Pane id. Defaults to PLEXI_PANE_ID.
        pane_id: Option<u64>,
    },
    /// List slots for a pane as JSON.
    List {
        /// Pane id. Defaults to PLEXI_PANE_ID.
        pane_id: Option<u64>,
    },
    /// Delete a named pane slot.
    Delete {
        /// Slot name
        name: String,
        /// Pane id. Defaults to PLEXI_PANE_ID.
        pane_id: Option<u64>,
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
    ///
    /// Examples:
    ///   plexi context new "sprint"                          # top-level context
    ///   plexi context new "sprint" --parent                 # child of current context (no-focus)
    ///   plexi context new "sprint" --parent=main -d         # child of "main", portal splits below
    ///   plexi context new "sprint" --parent --window "echo a" --window "echo b"
    New {
        /// Name for the new context. Defaults to the directory basename.
        name: Option<String>,
        /// Root path for the new context. Defaults to current working directory.
        #[arg(long)]
        path: Option<String>,
        /// Create as a child of a context (the new context is its sub-context).
        /// Bare `--parent` uses the current context (reads PLEXI_CONTEXT_NAME from
        /// env); use `--parent=<name>` to target another context by name.
        #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "__current__", value_name = "NAME")]
        parent: Option<String>,
        /// Command to run in each pre-populated window. Repeatable.
        #[arg(long, action = clap::ArgAction::Append)]
        window: Vec<String>,
        /// Focus (zoom into) the new sub-context after creation. Default: stay in current pane.
        #[arg(long)]
        focus: bool,
        /// Pane to anchor the portal split at (requires --parent).
        /// Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the
        /// parent context's focused pane.
        #[arg(long, value_name = "PANE_ID")]
        from: Option<u64>,
        /// Split portal below instead of right (requires --parent).
        #[arg(long, short = 'd', conflicts_with_all = ["left", "up", "right"])]
        down: bool,
        /// Split portal left (requires --parent).
        #[arg(long, short = 'l', conflicts_with_all = ["down", "up", "right"])]
        left: bool,
        /// Split portal above (requires --parent).
        #[arg(long, short = 'u', conflicts_with_all = ["down", "left", "right"])]
        up: bool,
        /// Split portal right — explicit (default, requires --parent).
        #[arg(long, short = 'r', conflicts_with_all = ["down", "left", "up"])]
        right: bool,
    },
    /// Switch the current pane to a context at the given path.
    Open { path: Option<String> },
    /// Change the root folder for the active context.
    SetRoot { path: Option<String> },
    /// Print the id and name of the current pane's context as JSON.
    Current,
    /// Set the description for the active context
    Describe {
        /// Description text
        text: String,
    },
    /// Zoom into a sub-context by its numeric context_id.
    Zoom { context_id: u64 },
    /// Zoom out of the current sub-context to the parent.
    ZoomOut,
    /// Push a pane into a new sub-context.
    ///
    /// Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the
    /// focused pane.
    Push {
        /// Name for the new sub-context. Defaults to the pane name.
        name: Option<String>,
        /// Pane to push. Defaults to the calling pane (PLEXI_PANE_ID env),
        /// falling back to the focused pane.
        #[arg(long, value_name = "PANE_ID")]
        pane_id: Option<u64>,
    },
    /// List all open contexts as a JSON array.
    List,
}

#[derive(Subcommand)]
pub enum ConfigCmd {
    /// Validate your config.toml and report any errors.
    Check {
        #[command(flatten)]
        scope: ConfigScopeArgs,
    },
    /// Open config.toml in your $EDITOR.
    Edit {
        #[command(flatten)]
        scope: ConfigScopeArgs,
    },
    /// Print the resolved value of a config key to stdout.
    ///
    /// Supports dotted keys: agents.low, agents.medium, agents.high.
    /// Returns the effective value (user setting or built-in default).
    Get {
        #[command(flatten)]
        scope: ConfigScopeArgs,
        /// Dotted key to retrieve (e.g. agents.medium).
        key: String,
    },
    /// Overwrite config.toml with the built-in default template.
    ///
    /// Creates a backup at config.toml.bak before overwriting.
    Reset {
        #[command(flatten)]
        scope: ConfigScopeArgs,
    },
    /// Print all known config keys with type, current value, and description.
    ///
    /// Columns: key\ttype\tvalue\tdescription. Use --json for machine-readable output.
    List {
        #[command(flatten)]
        scope: ConfigScopeArgs,
        /// Output as a JSON array instead of tab-separated lines.
        #[arg(long)]
        json: bool,
    },
    /// Set one or more config keys in-place.
    ///
    /// Each argument must be in KEY=VALUE form (e.g. theme.preset=dracula font_size=14).
    /// Scope defaults to workspace when inside a workspace, global otherwise.
    Set {
        #[command(flatten)]
        scope: ConfigScopeArgs,
        /// One or more KEY=VALUE pairs to write.
        #[arg(required = true, value_name = "KEY=VALUE")]
        pairs: Vec<String>,
    },
}

#[derive(Args, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConfigScopeArgs {
    /// Use the global channel config.toml only.
    #[arg(short = 'g', long = "global", conflicts_with = "workspace")]
    pub global: bool,
    /// Use the active workspace's channel-scoped config.toml only.
    #[arg(short = 'w', long = "workspace", visible_alias = "ws")]
    pub workspace: bool,
}

impl ConfigScopeArgs {
    pub fn scope(self) -> ConfigScope {
        if self.global {
            ConfigScope::Global
        } else if self.workspace {
            ConfigScope::Workspace
        } else {
            ConfigScope::Effective
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigScope {
    Effective,
    Global,
    Workspace,
}

pub fn normalize_config_scope_aliases(args: Vec<String>) -> Vec<String> {
    let Some(config_idx) = args.iter().position(|arg| arg == "config") else {
        return args;
    };
    args.into_iter()
        .enumerate()
        .map(|(idx, arg)| {
            if idx > config_idx && arg == "-ws" {
                "--workspace".to_string()
            } else {
                arg
            }
        })
        .collect()
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
    /// List notes in the inbox with frontmatter context.
    Inbox,
    /// Print inbox notes in agent-legible format with configured triage actions.
    Process,
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
        /// Open the new pane relative to this pane ID. Defaults to the calling
        /// pane (PLEXI_PANE_ID env), falling back to the focused pane.
        #[arg(long, value_name = "PANE_ID")]
        from: Option<u64>,
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
    /// Report agent state for this pane to the host.
    ///
    /// Called internally by hook scripts. Requires PLEXI_SOCKET and PLEXI_PANE_ID
    /// to be set in the environment.
    ///
    /// Example: plexi agent report --state working --agent claude-code
    Report {
        /// State to report: working, blocked, or idle
        #[arg(long)]
        state: String,
        /// Agent name (e.g. "claude-code")
        #[arg(long, default_value = "unknown")]
        agent: String,
        /// Active tool detail (optional, from hook event JSON)
        #[arg(long)]
        detail: Option<String>,
        /// Session ID (optional, from hook event JSON)
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Show current agent state for all panes.
    ///
    /// Queries the host for all panes that have reported agent state via hooks.
    /// Formats as a table with pane ID, agent name, state, and session ID.
    ///
    /// Example: plexi agent status
    /// Example: plexi agent status --blocked
    Status {
        /// Show only blocked panes
        #[arg(long)]
        blocked: bool,
        /// Show only working panes
        #[arg(long)]
        working: bool,
        /// Show only idle panes
        #[arg(long)]
        idle: bool,
    },
    /// Install or uninstall agent hook integrations.
    ///
    /// install: patches the selected agent config with lifecycle hook
    /// registrations, routing them to plexi agent report.
    ///
    /// uninstall: removes all PLEXI hook entries from the selected agent config.
    ///
    /// Example: plexi agent hook install --claude-code
    /// Example: plexi agent hook install --codex --pi
    /// Example: plexi agent hook uninstall --claude-code
    Hook {
        #[command(subcommand)]
        action: HookAction,
    },
}

#[derive(Subcommand)]
pub enum HookAction {
    /// Install PLEXI agent-state hook integrations.
    Install {
        /// Install Claude Code hooks (PreToolUse, PostToolUse, SessionStart, UserPromptSubmit, PermissionRequest, Stop, StopFailure, SessionEnd)
        #[arg(long = "claude-code")]
        claude_code: bool,
        /// Install Codex hooks (SessionStart, UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, Stop)
        #[arg(long)]
        codex: bool,
        /// Install Pi extension hooks (session, agent, and tool lifecycle events)
        #[arg(long)]
        pi: bool,
    },
    /// Remove PLEXI agent-state hook integrations.
    Uninstall {
        /// Remove Claude Code hooks
        #[arg(long = "claude-code")]
        claude_code: bool,
        /// Remove Codex hooks
        #[arg(long)]
        codex: bool,
        /// Remove Pi extension hooks
        #[arg(long)]
        pi: bool,
    },
}

#[derive(Subcommand)]
pub enum AiCmd {
    /// Guide first-run AI setup and the next app install step.
    ///
    /// Runs the same checks as `plexi ai doctor`, then prints the shortest path
    /// to usable AI: local Ollama, a user-owned OpenRouter key, or skipping AI
    /// for now. Ends with the app install command to try next.
    ///
    /// Example: plexi ai onboard
    Onboard,

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

#[cfg(test)]
mod tests {
    use super::{
        normalize_config_scope_aliases, AppCmd, Cli, Commands, ConfigCmd, ConfigScope, SecretCmd,
    };
    use clap::Parser;

    #[test]
    fn secret_list_accepts_global_flag() {
        let cli = Cli::try_parse_from(["plexi", "secret", "list", "--global"]).unwrap();

        let Some(Commands::Secret { cmd }) = cli.command else {
            panic!("expected secret command");
        };
        let SecretCmd::List { global } = cmd else {
            panic!("expected secret list command");
        };
        assert!(global);
    }

    #[test]
    fn secrets_alias_routes_to_secret_command() {
        let cli = Cli::try_parse_from(["plexi", "secrets", "list"]).unwrap();

        let Some(Commands::Secret { cmd }) = cli.command else {
            panic!("expected secret command");
        };
        assert!(matches!(cmd, SecretCmd::List { global: false }));
    }

    #[test]
    fn config_get_accepts_global_scope_flag() {
        let cli =
            Cli::try_parse_from(["plexi", "config", "get", "--global", "theme.accent"]).unwrap();

        let Some(Commands::Config { cmd }) = cli.command else {
            panic!("expected config command");
        };
        let ConfigCmd::Get { scope, key } = cmd else {
            panic!("expected config get command");
        };
        assert_eq!(scope.scope(), ConfigScope::Global);
        assert_eq!(key, "theme.accent");
    }

    #[test]
    fn config_check_accepts_workspace_scope_alias() {
        let cli = Cli::try_parse_from(["plexi", "config", "check", "--ws"]).unwrap();

        let Some(Commands::Config { cmd }) = cli.command else {
            panic!("expected config command");
        };
        let ConfigCmd::Check { scope } = cmd else {
            panic!("expected config check command");
        };
        assert_eq!(scope.scope(), ConfigScope::Workspace);
    }

    #[test]
    fn config_check_accepts_single_dash_ws_alias_after_normalization() {
        let args = normalize_config_scope_aliases(
            ["plexi", "config", "check", "-ws"]
                .into_iter()
                .map(String::from)
                .collect(),
        );
        let cli = Cli::try_parse_from(args).unwrap();

        let Some(Commands::Config { cmd }) = cli.command else {
            panic!("expected config command");
        };
        let ConfigCmd::Check { scope } = cmd else {
            panic!("expected config check command");
        };
        assert_eq!(scope.scope(), ConfigScope::Workspace);
    }

    #[test]
    fn app_open_path_accepts_trailing_launch_args() {
        let cli = Cli::try_parse_from([
            "plexi",
            "app",
            "open",
            "tests/wasm-fixtures/sysmon.wasm",
            "--",
            "--sample",
            "96",
        ])
        .unwrap();

        let Some(Commands::App { cmd }) = cli.command else {
            panic!("expected app command");
        };
        let AppCmd::Open {
            type_id,
            extra_args,
            ..
        } = cmd
        else {
            panic!("expected app open command");
        };
        assert_eq!(type_id.as_deref(), Some("tests/wasm-fixtures/sysmon.wasm"));
        assert_eq!(extra_args, ["--sample", "96"]);
    }

    #[test]
    fn app_init_does_not_open_by_default() {
        let cli = Cli::try_parse_from(["plexi", "app", "init", "counter"]).unwrap();

        let Some(Commands::App { cmd }) = cli.command else {
            panic!("expected app command");
        };
        let AppCmd::Init { open, no_open, .. } = cmd else {
            panic!("expected app init command");
        };
        assert!(!open);
        assert!(!no_open);
    }

    #[test]
    fn app_init_open_flag_opts_into_launch() {
        let cli = Cli::try_parse_from(["plexi", "app", "init", "counter", "--open"]).unwrap();

        let Some(Commands::App { cmd }) = cli.command else {
            panic!("expected app command");
        };
        let AppCmd::Init { open, no_open, .. } = cmd else {
            panic!("expected app init command");
        };
        assert!(open);
        assert!(!no_open);
    }

    #[test]
    fn app_init_accepts_legacy_no_open_flag() {
        let cli = Cli::try_parse_from(["plexi", "app", "init", "--no-open", "counter"]).unwrap();

        let Some(Commands::App { cmd }) = cli.command else {
            panic!("expected app command");
        };
        let AppCmd::Init { open, no_open, .. } = cmd else {
            panic!("expected app init command");
        };
        assert!(!open);
        assert!(no_open);
    }
}
