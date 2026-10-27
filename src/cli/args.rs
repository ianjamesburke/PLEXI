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

/// A CLI context target: accepts a numeric context id or a context name
/// (resolved by exact match, then unique prefix match).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextTarget(pub String);

impl ContextTarget {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn numeric_id(&self) -> Option<u64> {
        self.0.parse::<u64>().ok()
    }
}

impl std::fmt::Display for ContextTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for ContextTarget {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err("context target cannot be empty".to_string());
        }
        Ok(Self(trimmed.to_string()))
    }
}

impl From<&str> for ContextTarget {
    fn from(s: &str) -> Self {
        Self(s.trim().to_string())
    }
}

impl From<String> for ContextTarget {
    fn from(s: String) -> Self {
        Self(s.trim().to_string())
    }
}

impl From<u64> for ContextTarget {
    fn from(id: u64) -> Self {
        Self(id.to_string())
    }
}

impl PartialEq<u64> for ContextTarget {
    fn eq(&self, other: &u64) -> bool {
        self.0.parse::<u64>() == Ok(*other)
    }
}

impl PartialEq<&str> for ContextTarget {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// A CLI pane target: accepts a numeric pane id or a pane name
/// (resolved by exact match, then unique prefix match).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneTarget(pub String);

impl PaneTarget {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn numeric_id(&self) -> Option<u64> {
        self.0.parse::<u64>().ok()
    }
}

impl std::fmt::Display for PaneTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for PaneTarget {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err("pane target cannot be empty".to_string());
        }
        Ok(Self(trimmed.to_string()))
    }
}

impl From<&str> for PaneTarget {
    fn from(s: &str) -> Self {
        Self(s.trim().to_string())
    }
}

impl From<String> for PaneTarget {
    fn from(s: String) -> Self {
        Self(s.trim().to_string())
    }
}

impl From<u64> for PaneTarget {
    fn from(id: u64) -> Self {
        Self(id.to_string())
    }
}

impl PartialEq<u64> for PaneTarget {
    fn eq(&self, other: &u64) -> bool {
        self.0.parse::<u64>() == Ok(*other)
    }
}

impl PartialEq<&str> for PaneTarget {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

fn format_candidate_list(candidates: &[(u64, String)]) -> String {
    candidates
        .iter()
        .map(|(id, name)| format!("{name} ({id})"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = usize::from(a_chars[i - 1] != b_chars[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

/// Shared target resolver for context and pane targets.
///
/// Order: numeric id → exact name match → unique prefix match → on ambiguity,
/// error listing candidates (`name (id)`); on no match, error listing nearby names.
pub fn resolve_target_from_candidates(
    kind: &str,
    raw: &str,
    candidates: &[(u64, String)],
) -> Result<u64, String> {
    let query = raw.trim();
    if let Ok(id) = query.parse::<u64>() {
        return Ok(id);
    }

    let exact: Vec<(u64, String)> = candidates
        .iter()
        .filter(|(_, name)| name == query)
        .cloned()
        .collect();
    if exact.len() == 1 {
        return Ok(exact[0].0);
    }
    if exact.len() > 1 {
        return Err(format!(
            "ambiguous {kind} '{query}': matches {}",
            format_candidate_list(&exact)
        ));
    }

    let prefix: Vec<(u64, String)> = candidates
        .iter()
        .filter(|(_, name)| name.starts_with(query))
        .cloned()
        .collect();
    if prefix.len() == 1 {
        return Ok(prefix[0].0);
    }
    if prefix.len() > 1 {
        return Err(format!(
            "ambiguous {kind} '{query}': matches {}",
            format_candidate_list(&prefix)
        ));
    }

    if candidates.is_empty() {
        return Err(format!("unknown {kind} '{query}' (no {kind}s open)"));
    }

    let query_lower = query.to_ascii_lowercase();
    let mut ranked: Vec<(u64, String)> = candidates.to_vec();
    ranked.sort_by_key(|(id, name)| {
        let name_lower = name.to_ascii_lowercase();
        let is_substr =
            name_lower.contains(&query_lower) || query_lower.contains(&name_lower);
        let dist = edit_distance(&query_lower, &name_lower);
        (!is_substr, dist, *id)
    });
    ranked.truncate(5);
    Err(format!(
        "unknown {kind} '{query}' (nearby: {})",
        format_candidate_list(&ranked)
    ))
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

    /// Override the host command socket path.
    #[arg(long, global = true, value_hint = ValueHint::FilePath)]
    pub socket: Option<std::path::PathBuf>,

    /// Output as JSON (accepted on every command; no-op where output is already JSON).
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Open a workspace directory (plexi <path>)
    pub workspace_path: Option<String>,
}

#[derive(Subcommand)]
pub enum LeaseCmd {
    /// Acquire one slot. The calling shell owns it by default; no work is queued.
    Acquire {
        name: String,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        max: u32,
        /// Holder PID; must be this CLI or one of its ancestors.
        #[arg(long)]
        pid: Option<u32>,
    },
    /// Release this holder's leases (or just the named pool).
    Release {
        name: Option<String>,
        #[arg(long)]
        pid: Option<u32>,
    },
}

#[derive(Subcommand)]
pub enum CliSetupCmd {
    /// Install the `~/.local/bin` launcher, drop-in completions, and shell PATH registration.
    Setup {
        /// Append PATH and completion blocks to ~/.zshrc, ~/.bashrc, ~/.profile, and config.fish
        #[arg(long, overrides_with = "no_modify_path")]
        modify_path: bool,
        /// Do not edit shell startup files (.zshrc, .bashrc, .profile, config.fish)
        #[arg(long, overrides_with = "modify_path")]
        no_modify_path: bool,
    },
}

#[derive(Subcommand)]
pub enum Commands {
    /// Acquire process-lifetime capacity before starting work. Pauses exit with code 2.
    Lease {
        #[command(subcommand)]
        cmd: LeaseCmd,
    },
    /// Submit a text turn to the running host Assistant.
    Assistant {
        #[command(subcommand)]
        cmd: AssistantCmd,
    },
    /// Pair a phone through the Plexi relay and deliver its messages to this desktop.
    Relay {
        #[command(subcommand)]
        cmd: RelayCmd,
    },
    /// Show every registered agent and its runs. Cancel finishes an active run.
    ///
    /// The rows come from the agents API agent records and runs. `open` shows
    /// the same projection in a pane. `cancel` stops that run. `resolve` and
    /// `allow` never grant, and an agent pane is refused.
    ///
    /// Example: plexi command-view --json
    /// Example: plexi command-view --follow
    /// Example: plexi command-view open
    /// Example: plexi command-view cancel run_example
    #[command(name = "command-view")]
    CommandView {
        #[command(subcommand)]
        cmd: Option<CommandViewCmd>,
        /// Stream `command.view` events from `plexi.host.command` until interrupted.
        #[arg(long)]
        follow: bool,
    },
    // ── Workspace ─────────────────────────────────────────────────────────────
    /// Run a named command from `<repo>/.plexi-<channel>/commands.toml` or `~/.plexi-<channel>/commands.toml` (or global scripts).
    ///
    /// Resolution and precedence rules:
    ///
    /// (1) Global tier: `config_dir()/commands.toml` (`~/.plexi-alpha/commands.toml` on alpha,
    /// `~/.plexi-beta/commands.toml` on beta, `~/.plexi/commands.toml` on stable) plus executable
    /// files in `config_dir()/scripts/` (`~/.plexi-<channel>/scripts/`, or `~/.plexi/scripts/` on stable).
    ///
    /// (2) Workspace tier: `<workspace>/.plexi-<channel>/commands.toml` (`<workspace>/.plexi/commands.toml`
    /// on stable), where `<workspace>` is the nearest ancestor of the starting `cwd` containing a
    /// `.plexi-<channel>/` directory (`.plexi/` on stable) via `resolve_workspace_root_stopping_at`.
    /// The upward walk stops at `$HOME` and the passwd login home directory so `~` is never treated
    /// as a workspace. Fallback (`resolve_commands_workspace_root`): `cwd` itself when
    /// `cwd/.plexi-<channel>/commands.toml` (or `cwd/.plexi/commands.toml` on stable) exists and is
    /// distinct from the global `commands.toml`.
    ///
    /// (3) Starting directory by surface: `plexi run` in a terminal uses the shell's `cwd`.
    /// The command palette (`Cmd+P`) uses the focused pane's `cwd`, then falls back to the active
    /// context root (`effective_commands_workspace_root`); `Edit commands` opens that workspace's
    /// `commands.toml` (creating it from template if absent), or the global `commands.toml` when
    /// outside a workspace. `[keys]` `"<chord>" = "run:<name>"` bindings in
    /// `~/.plexi-<channel>/config.toml` (or `<repo>/.plexi-<channel>/config.toml`; `~/.plexi/config.toml`
    /// on stable) use the adopted/host workspace root, falling back to the active context root
    /// (`run_user_command_by_name`, validated by `plexi config check` against reserved shortcuts).
    ///
    /// (4) Precedence: workspace `commands.toml` > global `commands.toml` > global `scripts/`,
    /// merged by command name. `plexi run` with no arguments lists both scopes (`[workspace]` and
    /// `[global]`); `--json` prints `[{name, scope, description, run, steps, secrets}]` (secret
    /// names only, never values). Any secrets listed in a command definition are injected as
    /// environment variables automatically when the command runs.
    ///
    /// (5) Live reload: `commands.toml` and `scripts/` are re-read from disk on every command
    /// palette open and every `plexi run` or keybinding invocation — no host restart is needed.
    ///
    /// Multi-step commands (`steps = [...]`) execute each Plexi CLI invocation or shell snippet
    /// sequentially, substituting `$1`, `$2`, … and `$@` from extra arguments, and stop at the
    /// first failing step with `step <i>/<n> failed: <cmd> (exit <code>)`.
    ///
    /// Example: plexi run dev
    /// Example: plexi run play
    /// Example: plexi run --json
    #[command(next_help_heading = "Workspace")]
    Run {
        /// Command name to run (omit to list available commands)
        command: Option<String>,
        /// Extra arguments forwarded to the command as $1, $2, … and $@ positional params
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
    /// Preview and accept an agent's file edit before it is written.
    ///
    /// `propose` stores a change set and does not modify the file. `preview`
    /// prints the diff. `accept` writes it after the permission gate admits
    /// the same edit. If the file changed since propose, accept reports
    /// `stale` until `refresh`. `revert` restores a committed set.
    Changes {
        #[command(subcommand)]
        cmd: ChangesCmd,
    },
    /// Store and retrieve secrets (API keys, passwords, tokens).
    ///
    /// Secrets use macOS Keychain or, on Linux, Secret Service with a labeled
    /// encrypted-file fallback. Global, workspace, and folder scopes share that
    /// Linux backend. See https://plexiapp.com/docs/secrets/#linux-storage for
    /// backend selection and legacy plaintext migration.
    /// Folder secrets are an experiment, off by default in V1. Enable with
    /// `plexi experiments enable folder_secrets`, then `plexi host restart`
    /// (add `--yes` from a pane on that host). `secret set NAME --folder DIR`
    /// binds a value to a directory. New human terminal panes opened there or
    /// below it can receive the value as an environment variable. Folder-secret
    /// grant/read commands are experimental and are not supported in V1.
    ///
    /// Same-user native processes are not isolated from each other. A process
    /// running as this user can still read a value that a pane already holds.
    /// Folder secrets stop accidental injection into the wrong directory.
    #[command(alias = "secrets")]
    Secret {
        #[command(subcommand)]
        cmd: SecretCmd,
    },
    /// Manage workspace routines — scheduled shell commands.
    ///
    /// Routines are declared in `routines.toml` inside the workspace channel directory —
    /// `{workspace_channel_dir}/routines.toml`, e.g. `.plexi-alpha/routines.toml` on the
    /// alpha channel — and run automatically on schedule.
    /// **Requires Plexi to be running** — there is no background daemon. Routines only fire
    /// while the host process is open.
    ///
    /// Use `plexi routine list` to see configured routines, or `plexi routine run <name>` to fire one manually.
    Routine {
        #[command(subcommand)]
        cmd: RoutineCmd,
    },
    /// Create, inspect, and spawn agent definitions (`AGENT.md` + `settings.toml`).
    ///
    /// An agent is a directory containing `AGENT.md` (persona / system prompt) and
    /// `settings.toml` (`[agent]` metadata + `[harness]` CLI worker config).
    ///
    /// Two tiers exist, and workspace definitions shadow global ones with the same name:
    ///   - Workspace: `<workspace>/.plexi/agents/<name>/`
    ///   - Global:    `~/.plexi-<channel>/agents/<name>/` (pass `--global` to `create`)
    ///
    /// End-to-end worker workflow:
    ///   plexi agent create researcher --cli pi --model openrouter/~z-ai/glm-flash-latest \
    ///     --persona "You are a researcher." \
    ///     --result-schema '{"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}}}'
    ///   PANE=$(plexi agent spawn researcher --task "Summarize AgentRegistry")
    ///   plexi agent wait "$PANE" --timeout 300
    ///
    /// `agent start` without a first message and task-seeded `agent spawn` are both interactive adapter launches;
    /// only an explicitly configured Script job is one-shot.
    /// Spawn returns immediately with a pane id, before the worker is done.
    /// Wait in bounded slices with `plexi agent wait` without capture/send polling.
    /// Exit 0 means done; exit 2 means report progress and re-run the same wait;
    /// exit 1 is an error to inspect. Never use one long wait or respawn on timeout.
    #[command(verbatim_doc_comment)]
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
    /// Optional account that links this desktop to relay and cloud features.
    ///
    /// Free apps install without an account. Login requires the accounts backend
    /// enabled (`[marketplace].account_backend = "plexi"`); otherwise it fails
    /// closed with a clear message.
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
    ///
    /// Driving an agent in another pane (two commands, no sleeps, no capture polling):
    ///   1. Boot: `ID=$(plexi pane new --agent <tier-alias> -n "<name>" --no-focus)`
    ///      Blocks until the agent TUI is idle; a printed id means ready-to-brief.
    ///   2. Brief: `plexi pane send "$ID" "<brief>" --submit`
    ///      Settles the pane, presses Enter, and exits 0 only once the host
    ///      confirms the prompt left the input line.
    ///   3. Wait: `plexi pane wait "$ID" --until idle --timeout 300`
    ///      (or `plexi pane slot wait <slot> --pane-id "$ID" --until '<regex>'`)
    ///
    /// For an ordinary shell command in a terminal pane, use
    /// `plexi pane command <id> "<cmd>" --enter`.
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
    /// List everything waiting on you.
    ///
    /// One host record covers click approvals, agent questions, and blocked
    /// runs. The desktop badge and the phone page read that record.
    /// `resolve --approve` on a click approval is refused. `--deny` still denies.
    #[command(name = "needs-you")]
    NeedsYou {
        #[command(subcommand)]
        cmd: NeedsYouCmd,
    },
    /// List and change decisions stored by the permission monitor.
    ///
    /// `list` prints the live rows. `reset` clears a stored denial so the next
    /// call asks again. `revoke` removes an allow. `allow` turns a denial into
    /// an allow. Reset and allow from a pane, a call credential, or an agent
    /// file a Needs you item and leave the decision unchanged.
    Permissions {
        #[command(subcommand)]
        cmd: PermissionsCmd,
    },
    /// Send a notification to the Plexi UI.
    Notify {
        #[command(subcommand)]
        cmd: Option<NotifyCmd>,
        #[command(flatten)]
        post: NotifyPostArgs,
    },

    // ── AI ────────────────────────────────────────────────────────────────────
    /// AI configuration and diagnostics — scan hardware, check integrations, recommend models.
    #[command(next_help_heading = "AI")]
    Ai {
        #[command(subcommand)]
        cmd: AiCmd,
    },
    /// Summarize recorded AI usage from this channel's ledger.
    ///
    /// With no subcommand, prints per-client totals for tokens, cost, runs,
    /// and wall time. Reads `ai-ledger.jsonl` in the channel profile. Does
    /// not require a running host. A token count is a positive number or the
    /// word `unknown`. Rows written before run tags existed are migrated in
    /// place to explicit null `client` and `kind`.
    Ledger {
        #[command(subcommand)]
        cmd: Option<LedgerCmd>,
    },

    // ── System ────────────────────────────────────────────────────────────────
    /// Inspect what the host recorded: per-session flight traces.
    ///
    /// Reads files in the channel profile and does not require a running host.
    ///
    /// Example: plexi debug session list
    /// Example: plexi debug session show latest --pane 3 --join-agents
    Debug {
        #[command(subcommand)]
        cmd: DebugCmd,
    },
    /// List, enable, or disable experiments (features outside the stable v1 surface).
    ///
    /// Every experiment is off by default on every channel. Switches live in the
    /// `[experiments]` table of the global config.toml.
    ///
    /// Example: plexi experiments list
    /// Example: plexi experiments enable assistant
    #[command(next_help_heading = "System")]
    Experiments {
        #[command(subcommand)]
        cmd: ExperimentsCmd,
    },
    /// Install the agent skill compiled into this binary.
    ///
    /// Writes `SKILL.md` where Claude Code (`~/.claude/skills`) and Codex
    /// (`~/.codex/skills`) load user skills. The bytes are the copy embedded
    /// at build time, so the installed `plexi_version` matches this binary.
    ///
    /// Example: plexi skill install --agent claude
    #[command(next_help_heading = "System")]
    Skill {
        #[command(subcommand)]
        cmd: SkillCmd,
    },
    /// Print a shell completion script to stdout.
    ///
    /// Example: plexi completions zsh >> ~/.zshrc
    Completions {
        /// Shell name: zsh, bash, or fish
        shell: Option<String>,
    },
    /// Install the Plexi CLI launcher, drop-in completions, and shell PATH registration.
    ///
    /// Inside Plexi panes, `plexi` and tab completions are injected automatically
    /// without editing your shell dotfiles. Run `plexi cli setup` to install the
    /// `~/.local/bin` launcher and XDG/Homebrew completion drop-in files and append
    /// PATH and completion blocks to your shell startup files (`.zshrc`, `.bashrc`,
    /// `.profile`, `config.fish`). Pass `--no-modify-path` to skip dotfile edits.
    ///
    /// Example: plexi cli setup
    /// Example: plexi cli setup --no-modify-path
    #[command(next_help_heading = "System")]
    Cli {
        #[command(subcommand)]
        cmd: Option<CliSetupCmd>,
        /// Append PATH and completion blocks to ~/.zshrc, ~/.bashrc, ~/.profile, and config.fish
        #[arg(long, overrides_with = "no_modify_path")]
        modify_path: bool,
        /// Do not edit shell startup files (.zshrc, .bashrc, .profile, config.fish)
        #[arg(long, overrides_with = "modify_path")]
        no_modify_path: bool,
    },
    /// Check your Plexi config file for errors.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Capture, list, read, open, and delete notes.
    ///
    /// `plexi note "a thought"` captures a note. `plexi note list` prints note
    /// paths, `plexi note show <path>` prints one note, `plexi note open` picks
    /// one with fzf in the focused pane, and `plexi note rm <path>` deletes one.
    /// Notes live in the tier of the current directory (its nearest ancestor
    /// holding `.plexi/`, else the global tier); see `src/host/AGENTS.md` for the
    /// tier contract. `list` also rolls up nested tiers and the global tier.
    ///
    /// Text that is exactly `list`, `open`, `add`, `show`, or `rm` is read as
    /// that subcommand; shell quoting cannot tell them apart. Capture it with
    /// `plexi note add list`.
    ///
    /// Example: plexi note "remember to update the docs"
    #[command(args_conflicts_with_subcommands = true)]
    Note {
        #[command(subcommand)]
        cmd: Option<NoteCmd>,
        /// Note text to capture (shorthand for `note add`)
        text: Option<String>,
    },
    /// Compatibility spelling of `plexi note list` / `plexi note open`.
    #[command(hide = true)]
    Notes {
        #[command(subcommand)]
        cmd: Option<NoteCmd>,
    },
    /// Audit all installed apps for capability and config gaps.
    ///
    /// Checks every installed app's declared capabilities against your current config.toml
    /// and reports what's working and what needs to be configured. Use --json for scripting.
    Doctor,
    /// Interactive keybinding tutorial — learn split and navigate in real time.
    ///
    /// Walk through macOS pane controls inside a live pane.
    /// Requires macOS and a Plexi pane (PLEXI_PANE_ID must be set).
    Demo,
    /// Update this channel's managed package, or restore its previous generation.
    Update {
        /// Restore the retained previous package through the installation transaction
        #[arg(long)]
        rollback: bool,
    },
    /// Remove this channel's recorded package and launchers.
    ///
    /// Files outside the installation receipt, including development binaries
    /// and other channels, are preserved.
    Uninstall {
        /// Retain user data and its credentials (the default)
        #[arg(long = "keep-data", conflicts_with = "delete_data")]
        keep_data: bool,
        /// Permanently delete this channel's profile and integrity credentials
        #[arg(long, conflicts_with = "keep_data")]
        delete_data: bool,
        /// Remove this channel without a confirmation prompt
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
pub enum CommandViewCmd {
    /// Open the command view pane in the active context.
    Open,
    /// Stop a live run before its next tool call.
    ///
    /// Example: plexi command-view cancel run_example
    Cancel {
        /// Run id from `command-view --json`
        run: String,
    },
    /// Resolve a pending approval. An agent pane is refused, and this command does not grant.
    ///
    /// Example: plexi command-view resolve pending_example
    Resolve {
        /// Pending request id
        id: String,
    },
    /// Record an allow. An agent pane is refused, and this command does not grant.
    ///
    /// Example: plexi command-view allow --tool assistant.turn
    Allow {
        /// Tool name. Ignored. The command does not write a grant.
        #[arg(long)]
        tool: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum AssistantCmd {
    /// Submit through the same composer, model, and permission path as the desktop Assistant.
    Send {
        /// Prompt text. Omitted when `--status-for` only reads a pending turn.
        #[arg(long, required_unless_present = "status_for")]
        text: Option<String>,
        #[arg(long)]
        request_id: Option<String>,
        /// Target pane id or name.
        #[arg(long)]
        pane_id: Option<PaneTarget>,
        /// Target context id or name.
        #[arg(long)]
        context_id: Option<ContextTarget>,
        /// Caller-owned conversation. A phone session passes one stable id.
        /// Omit to start a new conversation that is not the desktop transcript.
        #[arg(long, conflicts_with = "desktop")]
        conversation: Option<String>,
        /// Append this turn to the desktop Assistant conversation.
        #[arg(long, conflicts_with = "conversation")]
        desktop: bool,
        /// Read the outcome of a turn that already returned waiting_for_permission.
        /// Does not submit a new prompt.
        #[arg(long, conflicts_with_all = ["text", "desktop", "conversation"])]
        status_for: Option<String>,
        /// Ledger client tag for this run. Omitted uses `[ai] client`.
        #[arg(long)]
        client: Option<String>,
        /// Ledger run kind: `system` or `output`. Omitted means `output`.
        #[arg(long)]
        kind: Option<String>,
    },
    /// Run one Assistant host tool through the host permission gate.
    ///
    /// Example: plexi assistant tool host.files.edit --input '{"path":"/tmp/note.md","old_string":"a","new_string":"b"}'
    Tool {
        /// Tool name, for example `host.editors.list` or `host.files.edit`.
        name: String,
        /// JSON object passed as the tool input.
        #[arg(long)]
        input: String,
    },
    /// Observe and resolve pending permission requests. This is not `plexi agent request`.
    Permission {
        #[command(subcommand)]
        cmd: AssistantPermissionCmd,
    },
}

#[derive(Subcommand)]
pub enum RelayCmd {
    /// Connect outbound to a phone relay and forward paired messages to the Assistant.
    Connect {
        /// Relay websocket URL (`ws://` locally, `wss://` in production).
        #[arg(long)]
        url: Option<String>,
    },
    /// Confirm a phone that redeemed the pairing code shown by `relay connect`.
    Confirm {
        /// Pairing id. Omit to confirm the phone currently waiting.
        pairing_id: Option<String>,
    },
    /// Revoke a paired phone. It must pair again and be confirmed.
    Revoke {
        /// Device id printed when the phone was confirmed.
        device_id: String,
    },
    /// Start a pairing code for another phone. Phones already paired stay paired.
    Pair,
    /// Remember the relay URL and connect it from the host on startup.
    Enable {
        /// Relay websocket URL (`ws://` locally, `wss://` in production).
        #[arg(long)]
        url: Option<String>,
    },
    /// Stop connecting to the relay when the host starts.
    Disable,
    /// Print the desktop's relay status.
    Status,
}

#[derive(Subcommand)]
pub enum DebugCmd {
    /// Inspect recorded host sessions.
    Session {
        #[command(subcommand)]
        cmd: DebugSessionCmd,
    },
}

#[derive(Subcommand)]
pub enum DebugSessionCmd {
    /// List recorded host sessions with start/last time, contexts, panes, and
    /// attached external agent session ids.
    List,
    /// Show one session as a single chronological timeline.
    ///
    /// With `--join-agents`, recorded agent session ids are resolved to their
    /// transcripts under `~/.pi/agent/sessions` and `~/.claude/projects`, and
    /// agent prompts and tool calls are interleaved with host permission and
    /// app events.
    Show {
        /// Session id (`sess-…`) or `latest`. Omitted means `latest`.
        session: Option<String>,
        /// Only records for this pane (and the calls it made).
        #[arg(long)]
        pane: Option<u64>,
        /// Show only CLI argv calls and their exit codes. Secret values, known
        /// secret options, and KEY=value assignments are redacted at capture.
        #[arg(long, conflicts_with = "join_agents")]
        cli: bool,
        /// Render timestamps in UTC instead of local time (text output only).
        #[arg(long)]
        utc: bool,
        /// Interleave external agent transcripts.
        #[arg(long)]
        join_agents: bool,
    },
}

#[derive(Subcommand)]
pub enum LedgerCmd {
    /// Totals for tokens, cost, run count, and wall time.
    Summary {
        /// Group rows by `client` or `kind`. Omitted means `client`.
        #[arg(long)]
        by: Option<String>,
        /// Include rows at or after this YYYY-MM-DD or RFC3339 timestamp.
        #[arg(long)]
        since: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ExperimentsCmd {
    /// Show every experiment: name, on/off, whether a host restart is needed.
    List,
    /// Turn an experiment on in the global config.toml.
    Enable {
        /// Experiment name; see `plexi experiments list`.
        name: String,
    },
    /// Turn an experiment off by removing its switch from the global config.toml.
    Disable {
        /// Experiment name; see `plexi experiments list`.
        name: String,
    },
}

#[derive(Subcommand)]
pub enum SkillCmd {
    /// Draft a reusable skill or commands.toml snippet from captured CLI calls.
    ///
    /// Prints for review; installs nothing (never edits `<repo>/.plexi-<channel>/commands.toml`
    /// or `~/.plexi-<channel>/commands.toml`, or `<repo>/.plexi/commands.toml` / `~/.plexi/commands.toml` on stable).
    /// Without --out, writes no draft files. Captured secret values and KEY=value assignments
    /// stay redacted. Review failed calls and suggested --subcontext forms before accepting the draft.
    Draft {
        /// Recorded host session id, or latest.
        #[arg(long)]
        from_session: String,
        /// Target this pane instead of PLEXI_PANE_ID (without either, includes all panes).
        #[arg(long, conflicts_with = "all_panes")]
        pane: Option<u64>,
        /// Include calls from every pane instead of the calling pane.
        #[arg(long)]
        all_panes: bool,
        /// Include agent bookkeeping and pane polling calls.
        #[arg(long)]
        include_internal: bool,
        /// Include failed or incomplete calls as review-only notes.
        #[arg(long)]
        include_failed: bool,
        /// Output format: SKILL.md or commands.toml snippet.
        #[arg(long = "as", default_value = "skill", value_parser = ["skill", "command"])]
        format: String,
        /// Write the draft to a new file instead of stdout; never installs it.
        #[arg(long, value_hint = ValueHint::FilePath)]
        out: Option<std::path::PathBuf>,
    },
    /// Write this binary's skill for one agent CLI, or all supported agents.
    Install {
        /// Which agent CLI loads the skill: claude, codex, pi, or all.
        #[arg(long, value_parser = ["claude", "codex", "pi", "all"])]
        agent: String,
    },
}

#[derive(Subcommand)]
pub enum NeedsYouCmd {
    /// List open items waiting on you as JSON.
    List,
    /// Show one needs-you item and its continuation record as JSON.
    Show {
        /// Needs-you item id or continuation id.
        id: String,
    },
    /// File a creative or blocking question for the human and return its id + continuation record.
    Ask {
        /// Question summary shown in Needs you and agent UI.
        #[arg(long)]
        summary: String,
        /// Target resource identifier (e.g. clip id or file path).
        #[arg(long, default_value = "workspace")]
        resource: String,
        /// Allowed answer choices (repeat `--choice` for multiple options).
        #[arg(long = "choice")]
        choices: Vec<String>,
        /// Allow free-text answers in addition to `--choice` options.
        #[arg(long)]
        allow_free_text: bool,
        /// Mark one `--choice` as recommended (shown starred in the decision overlay).
        #[arg(long)]
        recommend: Option<String>,
        /// Preview references (e.g. clip/frame/path references).
        #[arg(long = "preview-ref")]
        preview_refs: Vec<String>,
        /// Correlation id joining the question to an operation or continuation.
        #[arg(long)]
        correlation_id: Option<String>,
        /// Operation id for idempotent resume.
        #[arg(long)]
        operation_id: Option<String>,
        /// Agent session id for reattachment across restarts.
        #[arg(long)]
        session_id: Option<String>,
        /// Expected resource revision when resuming after the human answers.
        #[arg(long)]
        expected_revision: Option<String>,
        /// Target context id or name (defaults to caller pane's context).
        #[arg(long)]
        context_id: Option<ContextTarget>,
        /// Caller pane id or name (defaults to `$PLEXI_PANE_ID`).
        #[arg(long)]
        pane_id: Option<PaneTarget>,
    },
    /// Answer an open creative question (`Question` or `BlockedRun`). Never mints a permission grant.
    Answer {
        /// Needs-you question id.
        id: String,
        /// Selected choice or free-text answer.
        #[arg(long)]
        answer: String,
        /// Interactive UI prompt token returned when the question was filed.
        #[arg(long)]
        ui_token: Option<String>,
    },
    /// Reconcile open and resolved needs-you items and continuation records for a session, pane, or context.
    Reconcile {
        /// Filter by agent session id.
        #[arg(long)]
        session_id: Option<String>,
        /// Filter by pane id or name.
        #[arg(long)]
        pane_id: Option<PaneTarget>,
        /// Filter by context id or name.
        #[arg(long)]
        context_id: Option<ContextTarget>,
        /// Filter by specific item/run/operation/correlation id.
        #[arg(long)]
        id: Option<String>,
    },
    /// Mark a continuation record completed (or failed/canceled) with an execution receipt.
    Complete {
        /// Continuation run id, operation id, correlation id, or needs-you id.
        id: String,
        /// Terminal state (`completed`, `failed`, `canceled`, `outcome_unknown`).
        #[arg(long, default_value = "completed")]
        state: String,
        /// Execution receipt JSON or summary string.
        #[arg(long)]
        receipt: Option<String>,
    },
    /// Resolve one item. Approving a click approval is refused; deny still denies it.
    Resolve {
        id: String,
        /// Approve the item. A click approval stays open.
        #[arg(long, conflicts_with = "deny", required_unless_present = "deny")]
        approve: bool,
        /// Deny the item.
        #[arg(long, conflicts_with = "approve", required_unless_present = "approve")]
        deny: bool,
        /// Accepted so a paired phone can call resolve. The host still refuses
        /// an approval. Only a desktop Allow once click grants.
        #[arg(long)]
        from_phone: bool,
    },
}

#[derive(Subcommand)]
pub enum PermissionsCmd {
    /// List live permission decisions.
    List,
    /// Clear a stored denial so the next call asks again.
    Reset {
        /// Decision id from `plexi permissions list`.
        id: String,
    },
    /// Remove an allow, or refuse a pending ask.
    Revoke {
        /// Decision id from `plexi permissions list`.
        id: String,
    },
    /// Turn a denial or a pending ask into an allow.
    Allow {
        /// Decision id from `plexi permissions list`.
        id: String,
    },
}

#[derive(Subcommand)]
pub enum AssistantPermissionCmd {
    /// List pending permission requests as JSON.
    List,
    /// Show one pending permission request as JSON.
    Show { id: String },
    /// Resolve one pending request: once, session, always, or deny.
    /// The host refuses every choice from the terminal.
    Resolve {
        id: String,
        #[arg(long, value_parser = ["once", "session", "always", "deny", "deny_always", "revoke"])]
        choice: String,
    },
}

#[derive(Subcommand)]
pub enum ChangesCmd {
    /// Record a permission-gate allow for one exact edit.
    Allow {
        /// Agent id the grant and the later audit row name.
        #[arg(long)]
        agent: String,
        /// File the edit applies to.
        #[arg(long)]
        file: std::path::PathBuf,
        /// Exact text to replace. It must occur once.
        #[arg(long)]
        old: String,
        /// Replacement text.
        #[arg(long)]
        new: String,
    },
    /// Prepare an edit. The file is not modified.
    Propose {
        /// Agent id recorded on the change set.
        #[arg(long)]
        agent: String,
        /// File the edit applies to.
        #[arg(long)]
        file: std::path::PathBuf,
        /// Exact text to replace. It must occur once.
        #[arg(long)]
        old: String,
        /// Replacement text.
        #[arg(long)]
        new: String,
    },
    /// Print the prepared diff and whether the set is stale.
    Preview {
        /// Change set id from `propose`.
        id: String,
    },
    /// Write a pending change set after the permission gate admits it.
    Accept {
        /// Change set id from `propose`.
        id: String,
    },
    /// Rebase a stale change set onto the file's current text.
    Refresh {
        /// Change set id from `propose`.
        id: String,
    },
    /// Restore the file to the text from before accept.
    Revert {
        /// Change set id from `propose`.
        id: String,
    },
    /// Print this binary's profile directory.
    ///
    /// Channel-suffixed binaries use their own directory (`~/.plexi-pr-N`).
    /// Scripts should ask here instead of assuming `~/.plexi`.
    Profile,
}

#[derive(Subcommand)]
pub enum NotifyCmd {
    /// Remove a notification previously posted by this pane.
    Dismiss {
        /// Notification id printed by `plexi notify`.
        notify_id: String,
    },
}

#[derive(Args)]
pub struct NotifyPostArgs {
    /// Notification title (required when posting)
    #[arg(long)]
    pub title: Option<String>,
    /// Notification body text
    #[arg(long, default_value = "")]
    pub body: String,
    /// Add a clickable button to the notification. Format: `key:Label` (returns key when clicked) or
    /// `Label:pane_focus:<pane_id>` (switches focus to that pane when clicked).
    /// Repeatable.
    #[arg(long = "choice")]
    pub choices: Vec<String>,
    /// Action to perform on the host when a button is clicked. Format: `key:action_type:action_arg`.
    /// Repeatable. The host runs this even after the process that sent the notification has exited.
    #[arg(long = "host-action")]
    pub host_actions: Vec<String>,
    /// Queue choice buttons without waiting for a selected value
    #[arg(long)]
    pub no_wait: bool,
    /// How many seconds before the notification disappears (0 = stays until dismissed)
    #[arg(long, default_value = "0")]
    pub timeout: u64,
    /// How many seconds to wait for a choice response (0 = wait indefinitely)
    #[arg(long, default_value = "0")]
    pub wait_timeout: u64,
    /// Which panes see this notification: window, context, or global
    /// (default: context — the notification stays in the context that sent it)
    #[arg(long, value_name = "SCOPE", value_parser = ["window", "context", "global"])]
    pub scope: Option<String>,
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
    /// Declare an event stream so it can be emitted on and subscribed to.
    ///
    /// Registers a named stream under an app-id namespace with a JSON-Schema
    /// object describing its payload. Declaring is a prerequisite for `emit`.
    /// Re-declaring a stream replaces its previous schema. The first declare
    /// under a namespace you do not own prompts for host consent.
    ///
    /// Example: plexi events declare my-agent task.done --schema '{"type":"object"}'
    Declare {
        /// App-id namespace to declare the stream under (e.g. `my-agent`).
        app_id: String,
        /// Stream name to declare (e.g. `task.done`).
        stream: String,
        /// JSON-Schema object describing the event payload.
        #[arg(long, default_value = "{\"type\":\"object\"}")]
        schema: String,
        /// Human-readable description of when this event fires.
        #[arg(long)]
        description: Option<String>,
    },
    /// Emit an event onto a declared stream.
    ///
    /// Records a semantic event in the host timeline and fans it out to every
    /// broker-approved subscriber. The stream must already be declared. The
    /// emitter identity (actor_id) is host-stamped from your pane and cannot be
    /// set from the CLI; `--actor` sets only the advisory semantic category.
    ///
    /// Example: plexi events emit my-agent task.done --summary "Build finished" --resource build-42 --revision-after done
    Emit {
        /// App-id namespace the stream lives under (e.g. `my-agent`).
        app_id: String,
        /// Declared stream name to emit on (e.g. `task.done`).
        event: String,
        /// One-line human-readable description of what happened.
        #[arg(long)]
        summary: String,
        /// Id of the document/game/pane/resource the event is about.
        #[arg(long)]
        resource: String,
        /// Revision identifier after the change.
        #[arg(long = "revision-after")]
        revision_after: String,
        /// Semantic actor category that caused the change.
        #[arg(long, default_value = "agent", value_parser = ["user", "agent", "app", "system"])]
        actor: String,
        /// Scope class of the resource id (document, game, pane…). Defaults to pane.
        #[arg(long = "resource-scope")]
        resource_scope: Option<String>,
        /// Structured JSON payload; the declared stream schema is advisory and
        /// not enforced on the emit path.
        #[arg(long)]
        payload: Option<String>,
        /// Stable reference subscribers can fetch full state from.
        #[arg(long = "state-ref")]
        state_ref: Option<String>,
        /// Revision identifier before the change.
        #[arg(long = "revision-before")]
        revision_before: Option<String>,
        /// Opaque token that makes this event reversible (creates an undo checkpoint).
        #[arg(long = "rollback-token")]
        rollback_token: Option<String>,
        /// Other resource id touched by this change (repeatable).
        #[arg(long = "changed-resource")]
        changed_resources: Vec<String>,
    },
    /// List event streams currently declared by running apps.
    List,
    /// Print the host MCP server config for an MCP-aware agent.
    ///
    /// Emits a `mcpServers` JSON block pointing at this instance's host MCP
    /// server (read from `PLEXI_HOST_MCP_PORT` / `PLEXI_HOST_MCP_TOKEN`), so a
    /// Claude Code or Codex agent in this pane can call workspace app tools and
    /// subscribe to app events natively over MCP. The emitted credential is
    /// valid only while the originating pane remains alive.
    McpConfig,
}

#[derive(Subcommand)]
pub enum WorkspaceCmd {
    /// Set up a .plexi/ workspace in the current directory.
    ///
    /// Run this once inside your project folder. It creates `<repo>/.plexi-<channel>/`
    /// (or `<repo>/.plexi/` on stable) with `workspace.toml`, `secrets.toml`, `apps.toml`,
    /// and `commands.toml` so that secrets, apps, and commands are scoped to this project.
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
    /// Save a secret to the platform secret store.
    ///
    /// Plexi will prompt you to type the value (hidden). On macOS the secret is stored
    /// in the system keychain; on Linux it uses Secret Service or a labeled
    /// encrypted-file fallback. See https://plexiapp.com/docs/secrets/#linux-storage
    /// for storage and migration details. Terminal injection is opt-in in Secrets.
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
        /// Use a different platform-store entry name than the canonical env var name.
        /// On macOS this can reuse an existing Keychain entry. Example: plexi secret set OPENAI_API_KEY --alias openai_personal
        #[arg(long)]
        alias: Option<String>,
        /// Bind the secret to this directory. The value is stored in the OS keychain
        /// (or the labeled encrypted-file fallback) and injected into new human
        /// terminal panes opened in this directory or a subdirectory.
        /// Requires experiment folder_secrets, off by default in V1.
        /// Enable: plexi experiments enable folder_secrets, then plexi host restart
        /// (add --yes from a pane on that host).
        /// Example: plexi secret set OPENAI_API_KEY --folder ~/code/client-a
        #[arg(long, value_name = "DIR", conflicts_with_all = ["global", "alias"])]
        folder: Option<std::path::PathBuf>,
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
        /// List names in an explicit owned context (id, name or root). Agents need approval.
        #[arg(long, conflicts_with = "global", value_name = "ID|NAME|ROOT")]
        context: Option<ContextTarget>,
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
    /// Remove a folder-scoped secret.
    Rm {
        /// Environment variable name
        name: String,
        /// Directory the secret was bound to
        #[arg(long)]
        folder: std::path::PathBuf,
    },
    /// Ask for an allow on a folder secret. This command does not record one.
    ///
    /// Returns `permission_denied` and writes an audit row with the secret name
    /// and folder, never the value. This process does not record an allow. An
    /// agent pane cannot grant itself a secret, including a secret bound to
    /// another folder.
    Grant {
        /// Environment variable name
        name: String,
        /// Agent id. Stored as `agent:<id>`.
        #[arg(long, conflicts_with = "app")]
        agent: Option<String>,
        /// App id. Stored as `app:<id>`.
        #[arg(long, conflicts_with = "agent")]
        app: Option<String>,
        /// Directory the secret was bound to. Required when the name exists in more than one folder.
        #[arg(long)]
        folder: Option<std::path::PathBuf>,
    },
    /// Read a folder secret as an agent or app. Without a grant this prints
    /// `permission_required` and writes an audit row that names the secret
    /// but not its value.
    Read {
        /// Environment variable name
        name: String,
        /// Agent id. Stored as `agent:<id>`.
        #[arg(long, conflicts_with = "app")]
        agent: Option<String>,
        /// App id. Stored as `app:<id>`.
        #[arg(long, conflicts_with = "agent")]
        app: Option<String>,
        /// Directory the secret was bound to. Required when the name exists in more than one folder.
        #[arg(long)]
        folder: Option<std::path::PathBuf>,
    },
    /// Run a command with the environment a new terminal pane in `--cwd` receives,
    /// including folder secrets for that directory.
    ///
    /// Refused with `permission_denied` when the caller is a pane agent, including
    /// a child of that pane that cleared `PLEXI_PANE_ID`. The command is not
    /// started and no secret value is printed. Same-user native processes are
    /// not isolated from each other.
    Exec {
        /// Working directory of the spawned command, and the pane cwd used for injection.
        #[arg(long)]
        cwd: std::path::PathBuf,
        /// Command and arguments.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum AppCmd {
    /// Open an app or tool in a new pane.
    ///
    /// Pass an app id (e.g. `plexi app open calc`) or a path to an app directory
    /// containing a manifest.toml. Use `--mcp` to wrap an MCP server, or `--cli`
    /// to open any CLI tool with a Plexi UI.
    ///
    /// Default placement is a sibling split to the right — the calling pane is
    /// never taken over. Pass a direction flag (--down/--left/--up/--right),
    /// --tab, or --window to override; the app's manifest `[launch] placement`
    /// applies when no flag is given.
    Open {
        /// App id or path to open (mutually exclusive with --mcp and --cli)
        #[arg(conflicts_with_all = ["mcp", "cli"])]
        type_id: Option<String>,
        /// Explore a configured stdio MCP server in a Plexi pane.
        ///
        /// Example: plexi app open --mcp filesystem
        #[arg(long, num_args = 1, value_name = "SERVER_ID", conflicts_with = "cli")]
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
        /// Split direction or placement (down, left, up, right, tab, window)
        #[arg(long, value_name = "DIR", value_parser = ["down", "left", "up", "right", "tab", "window"], conflicts_with_all = ["down", "left", "up", "right", "tab", "window"])]
        split: Option<String>,
        /// Open the new pane relative to this pane id or name. Defaults to the calling
        /// pane (PLEXI_PANE_ID env), falling back to the focused pane.
        #[arg(long, value_name = "PANE_ID")]
        from: Option<PaneTarget>,
        /// Extra arguments passed through to the app (only valid with an app id)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, conflicts_with_all = ["mcp", "cli"])]
        extra_args: Vec<String>,
    },
    /// Pre-approve a raw `.wasm` component's host imports without a prompt.
    ///
    /// The non-interactive form of the review `plexi app open <file.wasm>`
    /// runs at a TTY: it persists Green grants for every host interface the
    /// component imports, scoped to the wasm's parent directory — the same
    /// store and scope an installed host checks at open time. Meant for
    /// automation (release gates, scene runners) pre-approving a vetted,
    /// repo-committed fixture so an installed host opens it without a human at
    /// a terminal. It never opens a pane; production interactive review is
    /// unchanged.
    Trust {
        /// Path to the `.wasm` component to trust.
        #[arg(value_hint = ValueHint::FilePath)]
        path: String,
    },
    /// Install an app from a local path, a remote source, or a pack file.
    ///
    /// Local path: `plexi app install ./my-app` — copies the app dir into Plexi's store.
    /// Remote source: `plexi app install github:owner/repo` — fetches and installs from GitHub.
    /// Pack file: `plexi app install --pack core` — installs from a pack file or the built-in core pack.
    /// Workspace pack: `plexi app install` (no args) — installs from `<repo>/.plexi-<channel>/apps.toml` (or `<repo>/.plexi/apps.toml` on stable).
    Install {
        /// Source to install: a local path, GitHub spec (github:owner/repo), or bare app id.
        /// Omit to install from the workspace pack (`<repo>/.plexi-<channel>/apps.toml`, or `<repo>/.plexi/apps.toml` on stable).
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
    /// Report obsolete first-party pre-v3 apps that launch-time reseeding quarantines.
    Prune {
        /// Show candidates without removing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Render an app headlessly (JSON frame tree by default, or PNG with --png).
    Render {
        /// App id or local path to render (e.g. "calc" or "./my-app")
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
    /// Run an app's AppHarness tests with `uv run --with pytest pytest tests/`.
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
    /// Show details about an installed app: manifest fields and tool status.
    ///
    /// Populates `tools` from the live tool registry when a pane of this app is
    /// running in the current context; otherwise labels the section as
    /// manifest-only (or static source scan) so an unopened app is never
    /// misreported as declaring no tools. Run `plexi app tools` to snapshot all
    /// live tools in the current context.
    Info { id: String },
    /// Read or replace a file-backed app's state document.
    ///
    /// Only apps that declare a `[state]` section are addressable. The state
    /// path is resolved from the manifest and the calling context — callers
    /// never pass a path, and there is no flag to address another context.
    State {
        #[command(subcommand)]
        cmd: AppStateCmd,
    },
    /// Create a new app from a template, or adopt an existing app directory.
    ///
    /// Scaffolds the folder structure and files you need to build a Plexi app:
    /// manifest.toml, main.py, tests/test_app.py, AGENTS.md, .gitignore, and
    /// plexi.scaffold.toml drift metadata.
    ///
    /// Pass `--adopt <dir>` (or `--adopt` inside an app directory) to write or
    /// refresh `plexi.scaffold.toml` for a pre-existing app without creating or
    /// modifying any source files.
    ///
    /// By default, a newly scaffolded app is placed in your workspace's app
    /// directory. If no workspace is detected, pass --global to scaffold into
    /// the global registry.
    ///
    /// Use --open to launch it in a split-right pane after scaffolding.
    /// See `sdk/python/AUTHORING.md` for the canonical authoring guide.
    Init {
        #[arg(required_unless_present_any = ["wasm", "adopt"])]
        name: Option<String>,
        /// Write or refresh `plexi.scaffold.toml` for an existing app directory without altering source files.
        #[arg(long, conflicts_with_all = ["wasm", "global", "open", "no_open", "from"])]
        adopt: bool,
        /// Create a Rust WASM component app with the Plexi WASM SDK.
        #[arg(long, value_name = "NAME", conflicts_with_all = ["name", "lang"])]
        wasm: Option<String>,
        /// App language/template: `python` (declarative UI). Use `--wasm <name>` for a Rust WASM component.
        #[arg(long, default_value = "python", value_parser = ["python"])]
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
        /// Open the new pane relative to this pane id or name. Defaults to the calling
        /// pane (PLEXI_PANE_ID env), falling back to the focused pane.
        #[arg(long, value_name = "PANE_ID")]
        from: Option<PaneTarget>,
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
        /// Pane id or name of the target app pane (from `plexi pane list`)
        pane_id: PaneTarget,
        /// Action name to invoke (e.g. "refresh", "navigate-to", "add-item")
        action: String,
        /// Optional arguments forwarded to the action handler
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Call a tool an app exposes and print its JSON result.
    ///
    /// Runs through the same tool dispatcher the Assistant uses, scoped to the
    /// calling pane's context (the active context when run outside a pane).
    /// The host stamps the caller identity from the pane credential or peer
    /// ancestry (`pane:<id>`). A missing pane is never the human `user`.
    /// The app sees that identity, never one taken from the input.
    /// Exits 1 with `error: <message>` when the tool or the app rejects the call.
    ///
    /// Example: plexi app call chess chess.state
    /// Example: plexi app call chess chess.play --input '{"game_id":"game-1","expected_revision":0,"operation_id":"op-1","move":"e2e4"}'
    #[command(name = "call")]
    Call {
        /// App id that exposes the tool (from `plexi app info`)
        app_id: String,
        /// Tool name as the app declares it (e.g. `chess.state`)
        tool: String,
        /// Tool input as a JSON object
        #[arg(long, default_value = "{}")]
        input: String,
        /// Pane id or name that owns the tool when more than one instance of the app is open.
        #[arg(long)]
        pane: Option<PaneTarget>,
        /// Wait up to two minutes for desktop approval, then execute once.
        #[arg(long)]
        wait_approved: bool,
    },
    /// Snapshot the live tool registry exposed by running app panes in the current context.
    ///
    /// Reads the same context-scoped `ToolDispatcher` registry that agents and
    /// `plexi app call` see: exposed tool name (including `<name>@<pane_id>`
    /// disambiguation when multiple panes expose the same tool), owning app id
    /// and pane id, description, and full JSON input/output schemas.
    ///
    /// Example: plexi app tools
    /// Example: plexi app tools --json
    /// Example: plexi app tools --app calc --json
    #[command(name = "tools")]
    Tools {
        /// Filter to tools exposed by this app id
        #[arg(long, value_name = "APP_ID")]
        app: Option<String>,
        /// Inspect a specific context id or name instead of the caller's context
        #[arg(long, value_name = "CONTEXT_ID")]
        context: Option<ContextTarget>,
    },
}

/// `plexi host <cmd>` — CLI-driven host launch with declarative boot state.
#[derive(Subcommand)]
pub enum HostCmd {
    /// Restart this channel, restoring the saved workspace.
    Restart {
        /// Confirm restarting the host containing this pane; this pane will close.
        #[arg(long)]
        yes: bool,
    },
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
        /// Boot a hermetic session: skip workspace restore on start and skip
        /// workspace save on shutdown. For automated runs (scene runners,
        /// release gates) that must never see or clobber the channel's saved
        /// session.
        #[arg(long)]
        ephemeral: bool,
        /// Launch without activating Plexi or taking focus. On macOS this uses
        /// Accessory activation policy, so the host has no normal Dock or
        /// menu-bar presence and should be driven through the CLI.
        #[arg(long)]
        background: bool,
    },
    /// Stop the running host and every pane agent for this channel.
    ///
    /// Non-interactive: sends pane process groups SIGHUP then SIGTERM, then
    /// SIGKILL after a grace period (PLEXI_QUIT_GRACE_MS, default 2000 ms;
    /// closing the window uses 500 ms).
    /// Sends a clean shutdown request first, falling back to SIGTERM if the
    /// host doesn't confirm exit in time.
    Stop,
    /// Write one info-level marker line into the running host's channel log.
    ///
    /// For automated drivers (release gates, scene runners, CI) that must
    /// leave start/finish summaries in `~/.plexi-<channel>/plexi.log` itself.
    ///
    /// Example: plexi host log --source editor_gate "gate finished passed=9 failed=0"
    Log {
        /// The marker text (flattened to one line in the log)
        #[arg(allow_hyphen_values = true)]
        message: String,
        /// Short tool identity prefixed to the line
        #[arg(long, default_value = "cli")]
        source: String,
    },
    /// Report whether this channel's host is running, its pid, socket path,
    /// and pane count.
    Status,
    /// Capture the running host window as a PNG through the real render
    /// pipeline — the pixels the user actually sees, no OS screen capture.
    ///
    /// Example: plexi host screenshot --pane 3 --output /tmp/pane3.png
    Screenshot {
        /// Crop the capture to this pane id or name's current screen rect
        #[arg(long)]
        pane: Option<PaneTarget>,
        /// Where to write the PNG (default: <profile>/screenshots/<timestamp>.png)
        #[arg(long, short = 'o')]
        output: Option<String>,
    },
    /// Inject a key or chord into the host-level keyboard path.
    ///
    /// This is a validation seam for host shortcuts: it does not send bytes
    /// to a focused pane. Example: `plexi-pr-2906 host key cmd+1`.
    Key {
        /// Single key, named key, or chord such as `cmd+1` or `cmd+shift+2`.
        key: String,
    },
}

/// `plexi account <cmd>` — marketplace account management.
#[derive(Subcommand)]
pub enum AccountCmd {
    /// Show whether you are logged in.
    Status,
    /// Log in to your marketplace account via emailed sign-in link.
    ///
    /// Runs the device-code flow: plexiapp.com emails a link, you click it in
    /// any browser, and the CLI stores the session. Magic-link login creates the
    /// account on first use — there is no separate signup.
    Login {
        /// Account email (falls back to [marketplace].account_email in config)
        #[arg(long)]
        email: Option<String>,
    },
    /// Log out and clear the local session.
    Logout,
}

#[derive(Subcommand)]
pub enum PaneCmd {
    /// Hide a pane (defaults to the calling pane).
    Hide {
        /// Target pane id or name (defaults to the calling pane).
        pane_id: Option<PaneTarget>,
    },
    /// Reveal a hidden pane (defaults to the calling pane).
    #[command(visible_alias = "unhide")]
    Reveal {
        /// Target pane id or name (defaults to the calling pane).
        pane_id: Option<PaneTarget>,
    },
    /// Focus a pane and toggle fullscreen zoom, or enter its sub-context.
    Zoom {
        /// Target pane id or name (defaults to the calling pane).
        pane_id: Option<PaneTarget>,
    },
    /// Focus a pane and swap it with a directional neighbor.
    Swap {
        #[arg(value_parser = ["left", "right", "up", "down"])]
        direction: String,
        /// Target pane id or name (defaults to the calling pane).
        #[arg(long)]
        pane_id: Option<PaneTarget>,
    },
    /// Wait for current idle, blocked, or exited state; print its lifecycle record.
    /// Exit 0 on match, 2 on timeout, 1 on invalid predicate or plumbing failure.
    Wait {
        /// Target pane id or name.
        pane_id: PaneTarget,
        /// Predicate: idle, blocked, or exited (any exit, including unknown status).
        #[arg(long)]
        until: String,
        /// Host deadline in seconds; must be finite and positive.
        #[arg(long, default_value_t = 300.0)]
        timeout: f64,
    },
    /// Follow brokered pane lifecycle records as NDJSON until interrupted.
    Events {
        #[arg(long, required = true)]
        follow: bool,
        /// Limit deliveries to one pane id or name in the caller's allowed context.
        #[arg(long)]
        pane: Option<PaneTarget>,
    },
    /// Open a terminal pane, the Agents pane (`agents`), or launch an agent with `--agent`.
    ///
    /// `--agent <tier-alias>` is the supported way to launch an agent into a fresh
    /// pane: the host runs the command in the new pane and prints the pane id only
    /// once the agent reports idle, so a printed id on stdout means ready-to-brief
    /// with `plexi pane send <id> "<brief>" --submit` (two commands, no sleeps, no
    /// capture polling).
    ///
    /// Examples:
    ///   plexi pane new --agent c-large -n "worker" --no-focus   # agent pane, id printed once ready-to-brief
    ///   plexi pane new --worktree feature/0668-test             # terminal in isolated wtp worktree
    ///   plexi pane new                                          # empty terminal, split right
    ///   plexi pane new "npm run dev" -n "dev"                   # terminal with command, named
    ///   plexi pane new -d                                       # split below
    ///
    /// For apps use `plexi app open`. For MCP servers use `plexi app open --mcp`.
    New {
        /// Shell command to run, or `agents` to open the built-in Agents pane
        #[arg(conflicts_with = "agent")]
        cmd: Option<String>,
        /// Launch an agent in the new pane and block until it reports ready-to-brief.
        ///
        /// The value is a shell command — normally a size-tier alias such as
        /// `c-large` or `codex-small` — run in the new pane verbatim. The pane
        /// id is printed only once the agent has booted, so a successful id on
        /// stdout means the pane is ready to receive a brief via
        /// `plexi pane send <id> "<brief>" --submit`. On boot timeout the reason
        /// and the created pane id go to stderr, stdout stays empty, and the
        /// exit code is 2 so the caller can close the pane.
        ///
        /// Requires a running Plexi host (PLEXI_SOCKET).
        #[arg(long, value_name = "TIER_ALIAS")]
        agent: Option<String>,
        /// How long to wait for the agent to report ready, in seconds.
        /// Defaults to 60 when omitted (the host owns the default).
        #[arg(long, value_name = "SECS", requires = "agent")]
        boot_timeout: Option<f64>,
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
        /// Pane id or name to split relative to. Defaults to the calling pane
        /// (PLEXI_PANE_ID env), falling back to the focused pane.
        #[arg(long, value_name = "PANE_ID")]
        from: Option<PaneTarget>,
        /// Close the pane when the command finishes
        #[arg(long, short = 'e', conflicts_with = "agent")]
        ephemeral: bool,
        /// Keep focus on the current pane
        #[arg(long)]
        no_focus: bool,
        /// Working directory
        #[arg(long, value_hint = ValueHint::DirPath)]
        cwd: Option<String>,
        /// Create or check out an isolated git worktree for <BRANCH> via `wtp`
        /// and start the pane inside it. When the pane closes, the host
        /// automatically reclaims the worktree if clean and unmoved, or retains
        /// it and reports its path if dirty or ahead.
        #[arg(long, value_name = "BRANCH")]
        worktree: Option<String>,
        /// Declare a JSON Schema (inline JSON object or path to a `.json` file)
        /// that the pane's structured return (`plexi pane result write`) must
        /// satisfy before the host accepts it.
        #[arg(long, value_name = "JSON_OR_PATH")]
        result_schema: Option<String>,
        /// Chained steps to run sequentially after the pane opens (reuses the command step runner)
        #[arg(long = "on-load", value_name = "STEP")]
        on_load: Vec<String>,
        /// Extra arguments forwarded to the command (for example after `--`)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, conflicts_with = "agent")]
        extra_args: Vec<String>,
    },
    /// Rename a pane.
    ///
    /// With one argument, renames the current pane: plexi pane name "My Project"
    /// With two arguments, renames any pane by id or name: plexi pane name 42 "My Project"
    Name {
        /// Pane id or name (from `plexi pane list`) or the new name if renaming the current pane
        first: String,
        /// New name when a pane id or name is given as the first argument
        second: Option<String>,
    },
    /// List all open panes as a JSON array.
    ///
    /// Filter by context: `--context` (no value) returns panes in the caller's context
    /// (reads PLEXI_CONTEXT_ID from env). `--context <id-or-name>` filters to a specific context.
    List {
        /// Filter by context id or name. With no argument, reads PLEXI_CONTEXT_ID from env (caller's context).
        /// With an argument, returns panes in that specific context.
        #[arg(long, value_name = "ID", num_args = 0..=1, default_missing_value = "current")]
        context: Option<String>,
    },
    /// Show the parent/child session provenance forest across open panes (`pstree` / `ps --forest` style).
    ///
    /// Displays each root pane and its spawned child panes with `spawned_by`, agent role, state, and title.
    /// Pass `--json` to output the nested forest as JSON.
    Tree {
        /// Filter by context id or name. With no argument, reads PLEXI_CONTEXT_ID from env (caller's context).
        #[arg(long, value_name = "ID", num_args = 0..=1, default_missing_value = "current")]
        context: Option<String>,
    },
    /// Move the visible focus to a specific pane.
    ///
    /// This moves what the user sees on screen — it does not change which pane an agent is running in.
    /// An agent calling this from pane A remains in pane A; the user just sees pane B highlighted.
    Focus {
        /// Pane id or name to focus (from `plexi pane list`).
        #[arg(required_unless_present_any = ["back", "forward"], conflicts_with_all = ["back", "forward"])]
        pane_id: Option<PaneTarget>,
        /// Step back through pane focus history.
        #[arg(long, conflicts_with = "forward")]
        back: bool,
        /// Step forward through pane focus history.
        #[arg(long)]
        forward: bool,
    },
    /// Configure a host-owned recurring prompt for a terminal pane.
    ///
    /// Example: plexi pane heartbeat 42 --every 5m --text "cycle"
    Heartbeat {
        /// Pane id or name to prompt (from `plexi pane list`)
        pane_id: PaneTarget,
        /// Interval such as 30s, 5m, or 1h.
        #[arg(long, conflicts_with = "off")]
        every: Option<String>,
        /// Prompt text submitted on each beat.
        #[arg(long, conflicts_with = "off")]
        text: Option<String>,
        /// Only submit when the shared agent detector reports idle (default).
        #[arg(long, action = clap::ArgAction::SetTrue)]
        while_idle_only: bool,
        /// Disable the pane heartbeat.
        #[arg(long, conflicts_with_all = ["every", "text", "while_idle_only"])]
        off: bool,
    },
    /// Close a pane. Omit the pane id to close the pane you are currently in.
    ///
    /// If the pane was created with `plexi pane new --worktree <BRANCH>`, the
    /// host automatically reclaims the worktree via `wtp` when clean and
    /// unmoved, or retains it and prints the retained worktree path when dirty
    /// or ahead.
    Close {
        /// Pane id or name to close (from `plexi pane list`). Defaults to the current pane if not given.
        pane_id: Option<PaneTarget>,
    },
    /// Type text into another pane as if it came from the keyboard, or pass `--submit`
    /// for host-confirmed prompt submission.
    ///
    /// Pass `--submit` (`-s`) when briefing an agent or submitting a prompt in a
    /// terminal pane: the host settles the pane, presses Enter (re-sending once if
    /// the text was parked as a collapsed paste), and exits 0 only once the prompt
    /// has left the input line — no `sleep` or `pane key <id> enter` needed.
    ///
    /// Without `--submit`, terminal panes receive raw PTY bytes and app panes receive
    /// one egui text input event after the host focuses the target pane. For an
    /// ordinary shell command, see also `plexi pane command <id> "<cmd>" --enter`.
    ///
    /// Examples:
    ///   plexi pane send 42 "Read the brief and implement the fix" --submit
    ///   plexi pane send 42 "partial text"
    Send {
        /// Pane id or name to send text to (from `plexi pane list`)
        pane_id: PaneTarget,
        /// Text to type into the pane (prefer `--submit` to submit with confirmation)
        #[arg(allow_hyphen_values = true)]
        text: String,
        /// Press Enter for you and wait until the host confirms the prompt
        /// left the pane's input line.
        ///
        /// The host settles the pane, submits, and re-sends Enter once if the
        /// text is still parked as a collapsed paste. Exits 0 only when the
        /// submission is confirmed; on an unconfirmed submit it exits non-zero
        /// and prints the observed input line to stderr. Terminal panes only.
        #[arg(long, short = 's')]
        submit: bool,
        /// Paste literal text, including newlines, as one edit. App panes only;
        /// does not read or change the system clipboard.
        #[arg(long, conflicts_with = "submit")]
        paste: bool,
    },
    /// Print the id of the pane you are currently in.
    ///
    /// Useful in scripts: MY_PANE=$(plexi pane self)
    #[command(name = "self")]
    Self_,
    /// Print details about a pane (default: the current pane, or the previously focused pane) as JSON.
    ///
    /// Example: plexi pane info 42
    Info {
        /// Pane id or name to describe. Defaults to the current pane (PLEXI_PANE_ID).
        #[arg(conflicts_with = "previous")]
        pane_id: Option<PaneTarget>,
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
        /// Pane id or name to capture output from. Defaults to the current pane.
        pane_id: Option<PaneTarget>,
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
        /// Print only captured lines on stdout. The next cursor is printed to stderr.
        #[arg(long)]
        plain: bool,
    },
    /// Report one composite working, idle, blocked, or unknown verdict as JSON.
    Status {
        /// Pane id or name to inspect.
        pane_id: PaneTarget,
    },
    /// Send a key press to a pane.
    ///
    /// For terminal panes, injects the keystroke into the terminal.
    /// For app panes, delivers a structured key event.
    ///
    /// To submit a prompt or command with host confirmation, prefer
    /// `plexi pane send <id> "<text>" --submit` (or `plexi pane command <id> "<cmd>" --enter`)
    /// instead of chaining `pane send` + `pane key <id> enter`: `--submit` settles
    /// the pane, handles collapsed-paste Enter, and exits 0 only once the host
    /// confirms the input line cleared.
    ///
    /// Key formats: single character ("h"), named key ("enter", "escape", "space",
    /// "up", "down", "left", "right", "backspace", "plus", "minus", "equals"),
    /// or chord ("ctrl+c").
    ///
    /// Example: plexi pane key 42 escape
    Key {
        /// Pane id or name to send the key to (from `plexi pane list`)
        pane_id: PaneTarget,
        /// Key to press
        key: String,
    },
    /// Drop a local file or image URL onto a pane.
    Drop {
        /// Target pane id or name.
        pane_id: PaneTarget,
        path_or_url: String,
    },
    /// Inject a synthetic pointer click into an app pane, for driving canvas
    /// interaction without OS-level automation.
    ///
    /// Two mutually exclusive targeting modes:
    ///
    /// PANE-PIXEL coordinates (origin at the pane's top-left) — the honest
    /// primitive, since it exercises the same fit=contain/fit=fill
    /// `canvas_transform` inversion a real click goes through. The host
    /// injects a real pointer move + press + release into the live egui pass,
    /// never a parallel resolver.
    ///
    /// `--node <node_id>` — targets a specific Button/TextInput/ListView node
    /// by the id `plexi pane state` reports, so a caller never has to compute
    /// pixel geometry. The host resolves the node's on-screen rect during the
    /// next render pass and fails loudly if the id is missing or not an
    /// interactive role.
    ///
    /// Only app panes accept clicks today.
    ///
    /// Examples:
    ///   plexi pane click 42 120 80
    ///   plexi pane click 42 --node 5
    Click {
        /// Pane id or name to click (from `plexi pane list`)
        pane_id: PaneTarget,
        /// X offset in pane pixels from the pane's top-left corner
        #[arg(requires = "y", conflicts_with = "node")]
        x: Option<f32>,
        /// Y offset in pane pixels from the pane's top-left corner
        #[arg(requires = "x", conflicts_with = "node")]
        y: Option<f32>,
        /// Node id to click (from `plexi pane state`), instead of pixel coordinates
        #[arg(long, conflicts_with_all = ["x", "y"])]
        node: Option<String>,
        /// Pointer button: "left", "right", or "middle"
        #[arg(long, default_value = "left")]
        button: String,
    },
    /// Drag the pointer across an app pane through the production input path:
    /// press, N intermediate moves, release — delivered one frame at a time.
    ///
    /// Endpoints are pane-pixel coordinates ("x,y" from the pane's top-left)
    /// or semantic node ids from `plexi pane state` (the drag targets the
    /// node's rendered bounds center).
    ///
    /// Example: plexi pane drag 42 --from 40,120 --to 260,120 --steps 12
    Drag {
        /// Pane id or name to drag in (from `plexi pane list`)
        pane_id: PaneTarget,
        /// Start position "x,y" in pane pixels
        #[arg(long, conflicts_with = "from_node")]
        from: Option<String>,
        /// Start node id (from `plexi pane state`)
        #[arg(long)]
        from_node: Option<String>,
        /// End position "x,y" in pane pixels
        #[arg(long, conflicts_with = "to_node")]
        to: Option<String>,
        /// End node id (from `plexi pane state`)
        #[arg(long)]
        to_node: Option<String>,
        /// Intermediate pointer moves between press and release (max 256)
        #[arg(long, default_value_t = 8)]
        steps: u32,
        /// Pointer button: "left", "right", or "middle"
        #[arg(long, default_value = "left")]
        button: String,
    },
    /// Send an ordinary shell command to a terminal pane as if typed from the
    /// keyboard.
    ///
    /// `--enter` submits it through the same host-confirmed settle -> Enter ->
    /// confirm sequence as `pane send --submit` (see that command for the
    /// full contract), rather than racing a raw newline against the shell's
    /// line editor. Exits 0 only once the host confirms the command was
    /// submitted; on an unconfirmed submit it exits non-zero and prints the
    /// observed input line to stderr. Terminal panes only.
    ///
    /// This verb is scoped to ordinary shell commands. Driving an
    /// interactive TUI belongs to `plexi pane send --submit`; booting another
    /// agent belongs to `plexi pane new --agent`.
    ///
    /// Example: plexi pane command 42 "git status" --enter
    #[command(name = "command")]
    Command {
        /// Pane id or name to send the command to (from `plexi pane list`)
        pane_id: PaneTarget,
        /// Text to send to the pane
        text: String,
        /// Submit the command through the host-confirmed settle -> Enter ->
        /// confirm sequence, instead of just typing it
        #[arg(long, short = 'e')]
        enter: bool,
    },
    /// Return the current UI state of a pane as JSON.
    ///
    /// For app panes: returns a versioned, runtime-neutral `semantic` tree. Process
    /// apps also retain the compatible `frame` RenderCommand array. Agents can use
    /// the semantic nodes to inspect what any app runtime is currently displaying.
    ///
    /// For terminal panes: returns a simple status object (type, title, pane_id).
    ///
    /// Every response also carries `claimed_state` (the pane's own `status`
    /// slot — an agent-authored claim, with its step token embedded in the
    /// value by convention, or `null` if never written) and `observed_state`
    /// (host-derived: process alive, seconds since last output, child pids,
    /// current foreground command) — always together, never one without the
    /// other. Pass `--stale-after <secs>` to additionally raise a typed
    /// `stale_claim` when a claim exists but the host has observed no output
    /// within that many seconds; omit it and `stale_claim` stays `null` with
    /// no evaluation performed.
    ///
    /// Example: plexi pane state 42 --stale-after 300
    State {
        /// Pane id or name to query (from `plexi pane list`)
        pane_id: PaneTarget,
        /// Raise `stale_claim` when the claimed state looks fresh but no
        /// output has been observed within this many seconds. Omitted by
        /// default — no stale evaluation runs unless this is set.
        #[arg(long)]
        stale_after: Option<u64>,
    },
    /// Manage host-managed named file slots for a pane.
    Slot {
        #[command(subcommand)]
        cmd: PaneSlotCmd,
    },
    /// Send and receive structured messages via a host-managed per-pane FIFO mailbox.
    Message {
        #[command(subcommand)]
        cmd: PaneMessageCmd,
    },
    /// Read a pane's validated structured result.
    ///
    /// Spawners declare the JSON Schema at spawn (`plexi pane new --result-schema <schema>`).
    /// The worker agent returns its structured JSON once via
    /// `plexi agent done --result <json>`: the host validates it against the
    /// declared schema synchronously and rejects malformed or schema-violating
    /// payloads back to the agent (exit 1 with the validation error) so the
    /// agent can self-correct before any consumer sees it.
    ///
    /// Reading via `plexi pane result <id>` (or `plexi pane result read <id>`)
    /// prints the validated JSON object to stdout and exits 0, or exits 2 with
    /// `error_code: "no_result_yet"` when no result has been written yet (or on
    /// `--wait` timeout), or exits 1 with `error_code: "invalid_result"` when
    /// the last attempted write was rejected and no valid result has been
    /// committed yet.
    #[command(args_conflicts_with_subcommands = true)]
    Result {
        #[command(subcommand)]
        cmd: Option<PaneResultCmd>,
        /// Pane id or name to read the result from (defaults to PLEXI_PANE_ID).
        #[arg(conflicts_with = "pane_id_flag")]
        pane_id: Option<PaneTarget>,
        /// Pane id or name (flag form; defaults to PLEXI_PANE_ID).
        #[arg(long = "pane-id", id = "pane_id_flag")]
        pane_id_flag: Option<PaneTarget>,
        /// Block host-side until a valid result is committed or `--timeout` expires.
        #[arg(long)]
        wait: bool,
        /// Seconds to wait when `--wait` is set before exiting 2.
        #[arg(long, default_value_t = 300.0)]
        timeout: f64,
    },
}

#[derive(Subcommand)]
pub enum PaneResultCmd {
    /// Read a pane's validated structured result as JSON.
    ///
    /// Exits 0 and prints the validated JSON object when committed, exits 2
    /// (`no_result_yet` or `--wait` `timeout`) when no result has been written
    /// yet, or exits 1 (`invalid_result`) when the last write was rejected by
    /// schema validation and not yet corrected.
    Read {
        /// Pane id or name (positional form). Defaults to PLEXI_PANE_ID.
        #[arg(conflicts_with = "pane_id_flag")]
        pane_id: Option<PaneTarget>,
        /// Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID.
        #[arg(long = "pane-id", id = "pane_id_flag")]
        pane_id_flag: Option<PaneTarget>,
        /// Block host-side until a valid result is committed or `--timeout` expires.
        #[arg(long)]
        wait: bool,
        /// Seconds to wait when `--wait` is set before exiting 2.
        #[arg(long, default_value_t = 300.0)]
        timeout: f64,
    },
    /// Block host-side until a pane commits a valid structured result, then print it.
    ///
    /// Equivalent to `plexi pane result <id> --wait`. Exits 0 with the validated
    /// JSON object, 2 on timeout, 1 on error.
    ///
    /// Agent spawn returns immediately. Wait in bounded slices with
    /// `plexi pane result wait <pane_id> --timeout 300`, well under your tool
    /// harness call limit. On exit 2, report progress and re-run the same wait
    /// until done (exit 0). Inspect errors on exit 1. Never use one long wait
    /// or spawn a replacement worker just because a slice expired.
    Wait {
        /// Pane id or name (positional form). Defaults to PLEXI_PANE_ID.
        #[arg(conflicts_with = "pane_id_flag")]
        pane_id: Option<PaneTarget>,
        /// Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID.
        #[arg(long = "pane-id", id = "pane_id_flag")]
        pane_id_flag: Option<PaneTarget>,
        /// Seconds to wait before exiting 2.
        #[arg(long, default_value_t = 300.0)]
        timeout: f64,
    },
}

#[derive(Subcommand)]
pub enum AppStateCmd {
    /// Print an app's current state document to stdout.
    Get {
        /// App id (from `plexi app list`)
        app: String,
        /// Which declared scope to read. Defaults to the app's first declared
        /// scope; a scope the app did not declare is an error.
        #[arg(long, value_parser = ["global", "context"])]
        scope: Option<String>,
    },
    /// Replace an app's state document, reading from a file or stdin.
    Set {
        /// App id (from `plexi app list`)
        app: String,
        /// File to read the new document from. Reads stdin when omitted.
        file: Option<std::path::PathBuf>,
        /// Which declared scope to write. Defaults to the app's first declared
        /// scope; a scope the app did not declare is an error.
        #[arg(long, value_parser = ["global", "context"])]
        scope: Option<String>,
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
        /// Pane id or name. Defaults to PLEXI_PANE_ID.
        #[arg(long)]
        pane_id: Option<PaneTarget>,
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
        /// Pane id or name (positional form). Defaults to PLEXI_PANE_ID.
        #[arg(conflicts_with = "pane_id_flag")]
        pane_id: Option<PaneTarget>,
        /// Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID.
        #[arg(long = "pane-id", id = "pane_id_flag")]
        pane_id_flag: Option<PaneTarget>,
    },
    /// Block until a slot's value matches a pattern, then print it.
    ///
    /// Exits 0 on a match, 2 on timeout (nothing on stdout), 1 on error.
    /// A slot whose current value already matches returns immediately.
    Wait {
        /// Slot name
        name: String,
        /// Pane id or name (positional form). Defaults to PLEXI_PANE_ID.
        #[arg(conflicts_with = "pane_id_flag")]
        pane_id: Option<PaneTarget>,
        /// Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID.
        #[arg(long = "pane-id", id = "pane_id_flag")]
        pane_id_flag: Option<PaneTarget>,
        /// Regex the slot value must match.
        #[arg(long)]
        until: String,
        /// Seconds to wait before giving up.
        #[arg(long, default_value_t = 300.0)]
        timeout: f64,
    },
    /// List slots for a pane as JSON.
    List {
        /// Pane id or name (positional form). Defaults to PLEXI_PANE_ID.
        #[arg(conflicts_with = "pane_id_flag")]
        pane_id: Option<PaneTarget>,
        /// Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID.
        #[arg(long = "pane-id", id = "pane_id_flag")]
        pane_id_flag: Option<PaneTarget>,
    },
    /// Delete a named pane slot.
    Delete {
        /// Slot name
        name: String,
        /// Pane id or name (positional form). Defaults to PLEXI_PANE_ID.
        #[arg(conflicts_with = "pane_id_flag")]
        pane_id: Option<PaneTarget>,
        /// Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID.
        #[arg(long = "pane-id", id = "pane_id_flag")]
        pane_id_flag: Option<PaneTarget>,
    },
}

#[derive(Subcommand)]
pub enum PaneMessageCmd {
    /// Enqueue a message in a target pane's FIFO mailbox.
    ///
    /// Sender identity (`from_pane_id`) is stamped host-side from the socket
    /// peer process ancestry and is never self-reported. Prints a JSON ack
    /// with `status` (`"queued"` or `"delivered"`), `message_id`, `queue_len`,
    /// and `woke`.
    ///
    /// Pass `--submit` to wake an idle target agent with a short notice to run
    /// `plexi pane message recv` without interrupting a busy agent.
    Send {
        /// Target pane id or name (from `plexi pane list`)
        pane_id: PaneTarget,
        /// Message payload to enqueue
        #[arg(allow_hyphen_values = true)]
        payload: String,
        /// Wake the target agent if idle (never interrupts a busy agent)
        #[arg(long)]
        submit: bool,
    },
    /// Drain all queued messages from a pane's mailbox as a JSON array.
    ///
    /// Defaults to `PLEXI_PANE_ID`. With `--timeout N` or `--wait`, blocks host-side with zero
    /// polling until at least one message is in the mailbox or `--timeout`
    /// expires (exits 0 with the drained JSON array, 2 on timeout, 1 on error).
    Recv {
        /// Pane id or name (positional form). Defaults to PLEXI_PANE_ID.
        #[arg(conflicts_with = "pane_id_flag")]
        pane_id: Option<PaneTarget>,
        /// Pane id or name. Same as the positional form; defaults to PLEXI_PANE_ID.
        #[arg(long = "pane-id", id = "pane_id_flag")]
        pane_id_flag: Option<PaneTarget>,
        /// Block host-side until at least one message arrives or `--timeout` expires.
        #[arg(
            long,
            default_value_if("timeout", clap::builder::ArgPredicate::IsPresent, "true")
        )]
        wait: bool,
        /// Block for up to N seconds before exiting 2. --wait alone uses 300 seconds.
        #[arg(long)]
        timeout: Option<f64>,
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
    /// Check installed CLI tools for changes to their help output and update Plexi's knowledge of them.
    Watch {
        /// Only check this one CLI tool instead of all of them
        cli: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum ContextCmd {
    /// Rename a context (defaults to the calling pane's context).
    Rename {
        name: String,
        /// Target context id or name (defaults to the calling pane's context).
        #[arg(long)]
        context_id: Option<ContextTarget>,
    },
    /// Close a context, requesting desktop confirmation when configured.
    Close {
        /// Target context id or name (defaults to the calling pane's context).
        context_id: Option<ContextTarget>,
    },
    /// Park a context and move focus to an unparked neighbor.
    Park {
        /// Target context id or name (defaults to the calling pane's context).
        context_id: Option<ContextTarget>,
    },
    /// Unpark a context and focus it.
    Unpark {
        /// Target context id or name (defaults to the calling pane's context).
        context_id: Option<ContextTarget>,
    },
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
        /// Pane id or name to anchor the portal split at (requires --parent).
        /// Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the
        /// parent context's focused pane.
        #[arg(long, value_name = "PANE_ID")]
        from: Option<PaneTarget>,
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
    /// Create a sub-context under the current one, pre-populated with panes.
    ///
    /// One command spins up a scoped squad: N panes in a single tiled window
    /// inside the new sub-context, each running the same command (or its own).
    /// Unlike `context new --parent --window`, this creates exactly N panes —
    /// no spare terminal — and roots them at the caller's cwd.
    ///
    /// Examples:
    ///   plexi context sub agentsquad --agents 3 --command cm
    ///   plexi context sub review --agents 2 --command "cm review" --command "cm test"
    ///   plexi context sub build --agents 4 --layout columns --focus
    Sub {
        /// Name for the new sub-context.
        name: String,
        /// Root path for the sub-context. Defaults to the caller's cwd.
        #[arg(long)]
        path: Option<String>,
        /// Number of panes to open inside the new sub-context.
        #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=32))]
        agents: u32,
        /// Command to launch in each pane. Give it once to apply to every pane,
        /// or exactly --agents times to give each pane its own command.
        #[arg(long, action = clap::ArgAction::Append, value_name = "CMD")]
        command: Vec<String>,
        /// How the panes are arranged inside the sub-context's window.
        /// Defaults to `tiled` when the pane count fills a square grid (4, 9, …)
        /// and `columns` otherwise so no grid cell is left empty.
        #[arg(long, value_enum)]
        layout: Option<crate::protocol::SubContextLayout>,
        /// Zoom into the new sub-context after creation. Default: stay put.
        #[arg(long)]
        focus: bool,
        /// Pane id or name to anchor the portal split at. Defaults to the calling pane
        /// (PLEXI_PANE_ID env), falling back to the parent's focused pane.
        #[arg(long, value_name = "PANE_ID")]
        from: Option<PaneTarget>,
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
    /// Zoom into a sub-context by its context id or name.
    Zoom {
        /// Target context id or name.
        context_id: ContextTarget,
    },
    /// Zoom out of the current sub-context to the parent.
    ZoomOut,
    /// Push a pane into a new sub-context.
    ///
    /// Defaults to the calling pane (PLEXI_PANE_ID env), falling back to the
    /// focused pane.
    Push {
        /// Name for the new sub-context. Defaults to the pane name.
        name: Option<String>,
        /// Pane id or name to push. Defaults to the calling pane (PLEXI_PANE_ID env),
        /// falling back to the focused pane.
        #[arg(long, value_name = "PANE_ID")]
        pane_id: Option<PaneTarget>,
    },
    /// List all open contexts as a JSON array.
    List,
}

#[derive(Subcommand)]
pub enum ConfigCmd {
    /// Reload configuration in the running host.
    Reload,
    /// Validate the selected config.toml and report any errors.
    ///
    /// Reads the workspace config inside a workspace, otherwise the global config.
    Check {
        #[command(flatten)]
        scope: ConfigScopeArgs,
    },
    /// Open the selected config.toml in your $EDITOR.
    ///
    /// Opens the workspace config inside a workspace, otherwise the global config.
    Edit {
        #[command(flatten)]
        scope: ConfigScopeArgs,
    },
    /// Print a config key from the selected config.toml to stdout.
    ///
    /// Supports dotted keys: agents.low, agents.medium, agents.high.
    /// Reads the workspace config inside a workspace, otherwise the global config.
    Get {
        #[command(flatten)]
        scope: ConfigScopeArgs,
        /// Dotted key to retrieve (e.g. agents.medium).
        key: String,
    },
    /// Overwrite the selected config.toml with the built-in default template.
    ///
    /// Writes the workspace config inside a workspace, otherwise the global config.
    /// Creates a backup at config.toml.bak before overwriting.
    Reset {
        #[command(flatten)]
        scope: ConfigScopeArgs,
    },
    /// Print all known config keys with type, current value, and description.
    ///
    /// Columns: key\ttype\tvalue\tdescription. Use --json for machine-readable output.
    /// Reads the workspace config inside a workspace, otherwise the global config.
    List {
        #[command(flatten)]
        scope: ConfigScopeArgs,
    },
    /// Set one or more keys in the selected config.toml.
    ///
    /// Each argument must be in KEY=VALUE form (e.g. theme.preset=dracula font_size=14).
    /// Writes the workspace config inside a workspace, otherwise the global config.
    /// Pass --global to read or write only the global config.
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
    /// Use the global channel config.toml only (`~/.plexi-<channel>/config.toml`, or `~/.plexi/config.toml` on stable).
    #[arg(short = 'g', long = "global", conflicts_with = "workspace")]
    pub global: bool,
    /// Use the active workspace's channel-scoped config.toml only (`<repo>/.plexi-<channel>/config.toml`, or `<repo>/.plexi/config.toml` on stable).
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
    /// List routines defined in the workspace's routines.toml with their schedule and next fire time.
    List,
    /// Manually trigger a named routine from the workspace's routines.toml.
    ///
    /// The routine fires exactly like a scheduled run: into its configured context
    /// when one is named (erroring if that context does not exist), otherwise into
    /// the caller's context.
    Run {
        /// Name of the routine to run
        name: String,
        /// Fire the routine even when it is disabled
        #[arg(long)]
        force: bool,
    },
    /// Add a routine to the workspace's routines.toml.
    ///
    /// The schedule is validated against the same parser the scheduler uses, so an
    /// accepted routine is guaranteed to fire. Hand-written comments elsewhere in
    /// the file are preserved.
    Add {
        /// Unique name for the routine
        name: String,
        /// Shell command the routine runs
        #[arg(long)]
        command: String,
        /// When to run — e.g. "every 30m", "daily at 09:00", "0 9 * * 1-5"
        #[arg(long)]
        schedule: String,
        /// Context to fire into (default: the active context at fire time)
        #[arg(long)]
        context: Option<String>,
        /// Close the spawned pane when the command exits
        #[arg(long)]
        ephemeral: bool,
    },
    /// Remove a routine from the workspace's routines.toml.
    Remove {
        /// Name of the routine to remove
        name: String,
    },
    /// Re-enable a disabled routine (removes its `enabled = false` key).
    Enable {
        /// Name of the routine to enable
        name: String,
    },
    /// Disable a routine without deleting it (sets `enabled = false`; it never fires until re-enabled).
    Disable {
        /// Name of the routine to disable
        name: String,
    },
}

#[derive(Subcommand)]
pub enum NoteCmd {
    /// Capture a note explicitly, even when the text is `list`, `open`, or `add`.
    ///
    /// Writes a timestamped note into the tier of the current directory.
    ///
    /// Example: plexi note add "remember to update the docs"
    Add {
        /// Note text to capture
        text: String,
    },
    /// Print paths of every note in every visible tier, newest first.
    ///
    /// Tiers are the current directory's context tier (its nearest ancestor
    /// holding a `.plexi/` directory), any tier nested below it, and the global
    /// tier. Resolved from the working directory, never from the environment.
    List,
    /// Open a note picker with fzf in the focused terminal pane.
    ///
    /// Candidates come from `plexi note list`. Requires fzf and a running host;
    /// both are reported as errors rather than silently degrading.
    Open,
    /// Print one note's contents by the path `plexi note list` printed.
    ///
    /// Refuses any path outside a notes tier visible from the current directory,
    /// so a hand-typed path cannot read an arbitrary file.
    ///
    /// Example: plexi note show ~/.plexi/notes/20260101T000000.000Z.md
    Show {
        /// Note path from `plexi note list`
        path: std::path::PathBuf,
    },
    /// Delete one note by the path `plexi note list` printed.
    ///
    /// Irreversible: the note file is removed from disk. Refuses any path
    /// outside a notes tier visible from the current directory.
    ///
    /// Example: plexi note rm ~/.plexi/notes/20260101T000000.000Z.md
    Rm {
        /// Note path from `plexi note list`
        path: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
pub enum AgentCmd {
    /// Inspect or run a stored team of independent ranking agents.
    ///
    /// Team v0 TOML is resolved from workspace channel teams/<name>.toml,
    /// then profile teams/<name>.toml, then built-in consult-experts.
    /// Fields: version = 0; options = ["Proceed", "Experiment", "Defer"].
    /// Each [[members]] has role, harness (pi/claude/codex/agy/script),
    /// grants = ["agents.ping=allow", "agents.review=deny"], optional model.
    /// Script members require command: a shell command whose stdout is exactly
    /// {"ranking":["Proceed","Experiment","Defer"],"rationale":"why"}.
    /// Scripts receive PLEXI_AGENT_TASK, PLEXI_TEAM_OPTIONS and PLEXI_TEAM_RUN_ID.
    /// Real harnesses report the same JSON through agent done.
    /// At least three members independently rank every option once. Borda awards
    /// N-1 down to 0 points. Optional tie_break = "Defer" selects a tied leader;
    /// default: last option. If that option is not tied, use the last tied option.
    /// Optional quorum = 2 requires that many valid ballots; default: all members.
    /// The host always writes RESULT.md with COMPLETE, PARTIAL, or FAILED status,
    /// member outcomes and valid ballots. Below quorum there is no winner.
    /// Failed/timed-out member panes stay open for inspection.
    /// Grants use existing delegation admission; a team is not authority.
    /// Format v0 may change. consult-experts uses pi, claude and codex by default.
    #[command(verbatim_doc_comment)]
    Team {
        #[command(subcommand)]
        cmd: AgentTeamCmd,
    },
    /// Focus the next live agent needing attention (needs-you, blocked, done-unseen).
    Next,
    /// Show tasks and their workers, ordered by attention, with blocking reasons.
    Fleet {
        #[command(subcommand)]
        cmd: Option<AgentFleetCmd>,
        /// Select all, needs-attention, working, or done workers.
        #[arg(long, value_enum, default_value = "all")]
        filter: crate::protocol::FleetFilter,
    },
    /// Create a new agent definition (`AGENT.md` + `settings.toml`) in the workspace or global tier.
    ///
    /// Writes `<workspace>/.plexi/agents/<name>/` (or `~/.plexi-<channel>/agents/<name>/` with `--global`)
    /// and validates the directory through `AgentDefinition::load_dir`. Refuses to overwrite an
    /// existing agent directory.
    ///
    /// Complete `settings.toml` example:
    /// ```toml
    /// [agent]
    /// id = "researcher"
    /// display_name = "Researcher"
    /// description = "Fast codebase researcher"
    ///
    /// [harness]
    /// cli = "pi"
    /// model = "openrouter/~z-ai/glm-flash-latest"
    /// fallback = "claude"
    /// ephemeral = true
    /// result_schema = '{"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}}}'
    /// ```
    #[command(verbatim_doc_comment)]
    Create {
        /// Agent slug (`a-z`, `0-9`, `-`)
        name: String,
        /// CLI harness to launch (`pi`, `claude`, `codex`, `agy`, or `script`). Interactive harnesses (`pi`, `claude`, `codex`, `agy`) remain interactive in start/spawn; only explicit script jobs are one-shot.
        #[arg(long, value_parser = ["pi", "claude", "codex", "agy", "script"])]
        cli: String,
        /// Model identifier passed verbatim via `--model`
        #[arg(long)]
        model: Option<String>,
        /// Fallback CLI harness when the primary binary is not on PATH (`pi`, `claude`, `codex`, `agy`, `script`)
        #[arg(long, value_parser = ["pi", "claude", "codex", "agy", "script"])]
        fallback: Option<String>,
        /// Short human-readable description of the agent's role
        #[arg(long)]
        description: Option<String>,
        /// Inline persona text written to `AGENT.md` (mutually exclusive with `--persona-file`)
        #[arg(
            long,
            required_unless_present = "persona_file",
            conflicts_with = "persona_file"
        )]
        persona: Option<String>,
        /// Path to a markdown file whose contents are written to `AGENT.md` (mutually exclusive with `--persona`)
        #[arg(
            long = "persona-file",
            required_unless_present = "persona",
            conflicts_with = "persona"
        )]
        persona_file: Option<String>,
        /// Optional JSON Schema string or path to a schema file enforced on `plexi agent done`
        #[arg(long = "result-schema")]
        result_schema: Option<String>,
        /// Auto-close the worker pane on clean exit (`0`)
        #[arg(long)]
        ephemeral: bool,
        /// Human-facing display name in `[agent]` (defaults to `<name>`)
        #[arg(long = "display-name")]
        display_name: Option<String>,
        /// Write to the global profile registry (`~/.plexi-<channel>/agents/<name>/`) instead of the workspace
        #[arg(long)]
        global: bool,
        /// Directory used to resolve the workspace root (defaults to current working directory)
        #[arg(long)]
        cwd: Option<String>,
        /// Optional custom harness flag(s) passed to the adapter (can be repeated)
        #[arg(long = "flag", action = clap::ArgAction::Append)]
        flags: Vec<String>,
    },
    /// Show supported custom harness flags and safety rules for an agent or adapter.
    ///
    /// When `<target>` is an agent slug, displays its resolved safe custom flags.
    /// When `<target>` is a harness adapter (`pi`, `claude`, `codex`, `agy`, `script`)
    /// or omitted, displays the adapter options inventory, reserved options, and security rules.
    ///
    /// Example: plexi agent flags
    /// Example: plexi agent flags claude
    /// Example: plexi agent flags researcher
    Flags {
        /// Agent slug or harness adapter name (`pi`, `claude`, `codex`, `agy`, `script`)
        #[arg(value_name = "TARGET")]
        target: Option<String>,
        /// Directory used to resolve the workspace root (defaults to current working directory)
        #[arg(long)]
        cwd: Option<String>,
    },
    /// List active agents from both workspace and global tiers.
    ///
    /// Prints each agent's name, tier (`workspace` or `global`), CLI + model (or `assistant`
    /// when no `[harness]` table is present), description, and whether it shadows a global agent.
    List {
        /// Directory used to resolve the workspace root (defaults to current working directory)
        #[arg(long)]
        cwd: Option<String>,
    },
    /// Show the resolved definition for `<name>`, its directory path, and its exact `spawn` command.
    Show {
        /// Agent name to inspect
        name: String,
        /// Directory used to resolve the workspace root (defaults to current working directory)
        #[arg(long)]
        cwd: Option<String>,
    },
    /// Call an agent role and return its validated completion or blocking question as JSON.
    Call {
        role: String,
        #[arg(long)]
        task: String,
        /// Create a managed worktree for the child.
        #[arg(long)]
        worktree: Option<String>,
        /// Return the child pane ID immediately.
        #[arg(long = "async")]
        asynchronous: bool,
    },
    /// Reply through a child agent's structured mailbox.
    Message {
        /// Target pane id or name.
        pane_id: PaneTarget,
        text: String,
        /// Wait for the child's next completion or question.
        #[arg(long)]
        wait: bool,
    },
    /// Transfer the current task to a permitted successor role.
    Handoff {
        role: String,
        #[arg(long)]
        task: String,
    },
    /// Start an interactive agent session in a new pane from the agent registry (`<workspace>/.plexi/agents/<name>/` or global).
    ///
    /// Resolves `<name>` by walking up from `--cwd` (or the current directory) to the project root
    /// (stopping at `$HOME`), then `~/.plexi-<channel>/agents/<name>/`, then built-in. Launches the
    /// agent's configured `[harness]` (`pi`, `claude`, `codex`, `agy`) interactively in the project
    /// directory with `AGENT.md` persona applied, the `# Plexi worker contract`, and `PLEXI_*` env set.
    ///
    /// `agent start` without a first message and task-seeded `agent spawn` are both interactive adapter launches;
    /// only an explicitly configured Script job is one-shot. Child results report via `plexi agent done` and are
    /// collected with `plexi agent wait` without capture/send polling.
    ///
    /// Example: plexi agent start researcher
    Start {
        /// Agent name in `<workspace>/.plexi/agents/<name>/` or `~/.plexi-<channel>/agents/<name>/`
        name: String,
        /// Optional initial message to send in the interactive session
        #[arg(value_name = "MESSAGE", conflicts_with = "message")]
        prompt: Option<String>,
        /// Optional initial message to send in the interactive session
        #[arg(long, value_name = "MESSAGE")]
        message: Option<String>,
        /// Working directory for workspace lookup and the interactive agent pane
        #[arg(long)]
        cwd: Option<String>,
    },
    /// Spawn a CLI worker agent from the agent registry (`<workspace>/.plexi/agents/<name>/` or global).
    ///
    /// Loads `<name>` through `AgentRegistry` (workspace shadowing global), refuses agents without
    /// a `[harness]` table, composes the CLI command with persona and `# Plexi worker contract`,
    /// and launches the worker pane. Codex workers (including calls and team members) use
    /// `codex --sandbox workspace-write -c sandbox_workspace_write.network_access=true`.
    /// They can write in their workspace and use the network; Plexi adds no profile writable root.
    ///
    /// Task-seeded `agent spawn` and `agent start` without a first message are both interactive adapter launches;
    /// only an explicitly configured Script job is one-shot. Results report via `plexi agent done` and are
    /// collected with `plexi agent wait` or parent idle-batch delivery without capture/send polling.
    ///
    /// Example: plexi agent spawn researcher --task "Audit auth module" --subcontext auth-audit --ephemeral
    Spawn {
        /// Agent name in `<workspace>/.plexi/agents/<name>/` or `~/.plexi-<channel>/agents/<name>/`
        name: String,
        /// Task brief passed to the spawned agent harness
        #[arg(long)]
        task: String,
        /// Sub-context name to spawn into
        #[arg(long)]
        subcontext: Option<String>,
        /// Auto-close the pane on clean exit (`0`) so the sub-context collapses when all workers finish
        #[arg(long)]
        ephemeral: bool,
        /// Parent/caller pane id or name (defaults to `$PLEXI_PANE_ID`)
        #[arg(long, value_name = "PANE_ID")]
        from: Option<PaneTarget>,
        /// Working directory for workspace lookup and the spawned worker pane
        #[arg(long)]
        cwd: Option<String>,
    },
    /// Validate a worker's structured result against its schema, write completion slots, and exit cleanly.
    ///
    /// Validates `--result` against the pane's `result_schema` (when present), writes
    /// the `completed` and `summary` pane slots, and transitions the pane to `Idle` so
    /// an ephemeral worker can exit `0` and trigger sub-context auto-collapse.
    ///
    /// Example: plexi agent done --result '{"status":"pass"}' --summary "Auth audit complete"
    Done {
        /// Structured JSON result payload
        #[arg(long)]
        result: String,
        /// Human- and master-readable completion summary written to the `summary` slot
        #[arg(long)]
        summary: Option<String>,
        /// Target pane id or name (defaults to `$PLEXI_PANE_ID`)
        #[arg(long = "pane-id")]
        pane_id: Option<PaneTarget>,
    },
    /// Read a completed agent's persisted result (`RESULT.md` or `result.json` with `--json`).
    ///
    /// Reads `<workspace>/.plexi/results/<pane-or-run-id>/RESULT.md` (and `result.json`),
    /// defaulting to the most recently completed agent result when `<pane>` is omitted.
    ///
    /// Example: plexi agent result
    /// Example: plexi agent result 42 --json
    Result {
        /// Pane ID, agent role name, or run/session ID (defaults to latest completed run)
        #[arg(value_name = "PANE")]
        pane: Option<String>,
        /// Pane ID, agent role name, or run/session ID (alias for positional `<PANE>`)
        #[arg(long = "pane", visible_alias = "pane-id", value_name = "PANE", conflicts_with = "pane")]
        pane_flag: Option<String>,
    },
    /// Wait for one or more child agent panes to finish via `plexi agent done` and pull their results.
    ///
    /// Blocks until every specified child pane has reported completion (or exited/failed) or `--timeout`
    /// expires. Results pulled via `agent wait` are consumed so the host does not re-deliver them as a
    /// duplicate idle-batch turn to the parent pane.
    ///
    /// Example: plexi agent wait 42 43 --timeout 300
    Wait {
        /// Child pane IDs or names to wait for
        #[arg(required = true)]
        pane_ids: Vec<PaneTarget>,
        /// Timeout in seconds
        #[arg(long, default_value_t = 300.0)]
        timeout: f64,
    },
    /// Escalate a blocking question to the parent master agent (Tier 1 `Blocked` state).
    ///
    /// Transitions this worker pane to `Blocked`, writes the `question` slot, and wakes
    /// the `PLEXI_PARENT_PANE` master agent waiting on `plexi pane wait` without filing
    /// a human-facing `NeedsYou` alert.
    ///
    /// Example: plexi agent ask "Should we preserve the legacy v1 token endpoint?"
    Ask {
        /// Question text for the parent master agent
        question: String,
        /// Target pane id or name (defaults to `$PLEXI_PANE_ID`)
        #[arg(long = "pane-id")]
        pane_id: Option<PaneTarget>,
    },
    /// Report agent state for this pane to the host.
    ///
    /// Called internally by hook scripts. Requires PLEXI_SOCKET and PLEXI_PANE_ID
    /// to be set in the environment.
    ///
    /// Example: plexi agent report --state working --agent claude-code
    Report {
        /// State to report: working, blocked, needs_you, idle, interrupted, failed, or completed
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
        /// Raw provider lifecycle event, retained separately from the UI state.
        #[arg(long)]
        event: Option<String>,
        /// Explicit blocking reason; state-only callers remain supported.
        #[arg(long, value_enum, requires = "state")]
        blocked_reason: Option<crate::protocol::AgentBlockedReason>,
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
    /// Show closed agent runs and sessions from `~/.plexi-<channel>/agent_history.jsonl`.
    ///
    /// Example: plexi agent history
    /// Example: plexi agent history --limit 20 --json
    History {
        /// Maximum number of recent closed runs to display (0 = all)
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Reopen a closed agent run or session from `agent_history.jsonl` in its saved working directory.
    ///
    /// Resolves the matching entry by `run_id` or harness `session_id` and spawns a new
    /// pane running the harness's resume command (`claude --resume <id>`, `codex resume <id>`,
    /// `pi --session <id>`, `agy --resume <id>`).
    ///
    /// Example: plexi agent reopen sess-claude-42
    /// Example: plexi agent reopen run-7-1729100000000
    Reopen {
        /// Run ID (`run-...`) or harness session ID to reopen
        target: String,
        /// Optional sub-context name to reopen the session into
        #[arg(long)]
        subcontext: Option<String>,
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
    /// Read one harness hook payload (JSON) from stdin and report the agent state.
    /// Registered automatically for agents Plexi spawns; no `jq` or shell required.
    /// Example: plexi agent hook-event --agent claude-code < payload.json
    #[command(name = "hook-event", hide = true)]
    HookEvent {
        /// Agent name to report as (default: $PLEXI_AGENT_NAME, else claude-code)
        #[arg(long)]
        agent: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum AgentFleetCmd {
    /// Dismiss a failed, stalled, or completed agent run from the fleet view while keeping it in `agent history`.
    Dismiss {
        /// Pane ID (`#215` or `215`) to dismiss (omit when `--all-failed` is set)
        #[arg(required_unless_present = "all_failed", conflicts_with = "all_failed")]
        target: Option<String>,
        /// Dismiss all failed, stalled, and exited-without-report runs
        #[arg(long = "all-failed")]
        all_failed: bool,
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
pub enum AgentTeamCmd {
    /// Show the resolved v0 team, including roles, harnesses, options and grants.
    Show { name: String },
    /// Run independent team members in one task and wait for RESULT.md.
    Run {
        name: String,
        #[arg(long)]
        question: String,
        /// Output directory (default: team-results/<run-id> in a workspace, or the channel profile's sessions/teams/ outside one).
        #[arg(long)]
        out: Option<std::path::PathBuf>,
        /// Replace the stored options; repeat for each option (at least two).
        #[arg(long = "option")]
        options: Vec<String>,
        /// Host-side deadline in seconds; unfinished members are reported as timed out.
        #[arg(long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(1..=86400))]
        timeout: u64,
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
    Doctor,

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
        normalize_config_scope_aliases, AppCmd, Cli, Commands, ConfigCmd, ConfigScope, NotifyCmd,
        SecretCmd,
    };
    use clap::Parser;

    #[test]
    fn session_history_and_draft_flags_parse() {
        let cli = Cli::try_parse_from(["plexi", "skill", "draft", "--from-session", "latest",
            "--all-panes", "--include-internal", "--include-failed"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Skill { cmd: super::SkillCmd::Draft {
            all_panes: true, include_internal: true, include_failed: true, .. } })));
        assert!(Cli::try_parse_from(["plexi", "skill", "draft", "--from-session", "latest", "--pane", "19", "--all-panes"]).is_err());
        let cli = Cli::try_parse_from(["plexi", "debug", "session", "show", "--utc", "--cli"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Debug { cmd: super::DebugCmd::Session {
            cmd: super::DebugSessionCmd::Show { utc: true, cli: true, .. } } })));
    }

    #[test]
    fn message_recv_timeout_implies_wait() {
        for (extra, expected_wait, expected_timeout) in [
            (vec![], false, None),
            (vec!["--wait"], true, None),
            (vec!["--timeout", "1"], true, Some(1.0)),
            (vec!["--timeout", "0"], true, Some(0.0)),
            (vec!["--wait", "--timeout", "1"], true, Some(1.0)),
        ] {
            let mut argv = vec!["plexi", "pane", "message", "recv", "7"];
            argv.extend(extra);
            let cli = Cli::try_parse_from(argv).unwrap();
            match cli.command.unwrap() {
                super::Commands::Pane {
                    cmd:
                        super::PaneCmd::Message {
                            cmd: super::PaneMessageCmd::Recv { wait, timeout, .. },
                        },
                } => {
                    assert_eq!(wait, expected_wait);
                    assert_eq!(timeout, expected_timeout);
                }
                _ => panic!("expected recv"),
            }
        }
    }

    #[test]
    fn note_parses_shorthand_capture_subcommands_and_bare_help() {
        use super::{Commands, NoteCmd};
        let parse = |args: &[&str]| {
            let mut v = vec!["plexi"];
            v.extend_from_slice(args);
            Cli::try_parse_from(v)
        };
        let note = |c: Cli| match c.command {
            Some(Commands::Note { cmd, text }) => (cmd, text),
            _ => panic!("not a note command"),
        };
        let (cmd, text) = note(parse(&["note", "a thought"]).unwrap());
        assert!(cmd.is_none());
        assert_eq!(text.as_deref(), Some("a thought"));
        let (cmd, text) = note(parse(&["note"]).unwrap());
        assert!(cmd.is_none() && text.is_none());
        for (word, want_list) in [("list", true), ("open", false)] {
            let (cmd, text) = note(parse(&["note", word]).unwrap());
            assert!(text.is_none());
            assert!(matches!(
                (cmd, want_list),
                (Some(NoteCmd::List), true) | (Some(NoteCmd::Open), false)
            ));
        }
        for word in ["list", "open", "add", "show", "rm"] {
            let (cmd, _) = note(parse(&["note", "add", word]).unwrap());
            assert!(matches!(cmd, Some(NoteCmd::Add { text }) if text == word));
        }
        // `show` and `rm` take the path `plexi note list` printed.
        let (cmd, _) = note(parse(&["note", "show", "/tmp/a.md"]).unwrap());
        assert!(matches!(cmd, Some(NoteCmd::Show { path }) if path.ends_with("/tmp/a.md")));
        let (cmd, _) = note(parse(&["note", "rm", "/tmp/a.md"]).unwrap());
        assert!(matches!(cmd, Some(NoteCmd::Rm { path }) if path.ends_with("/tmp/a.md")));
        // Stray arguments are errors, never accidental captures.
        assert!(parse(&["note", "list", "extra"]).is_err());
        assert!(parse(&["note", "add"]).is_err());
        assert!(parse(&["note", "show"]).is_err());
        assert!(parse(&["note", "rm"]).is_err());
        assert!(parse(&["note", "one", "two"]).is_err());
    }

    #[test]
    fn notes_compat_dispatches_and_is_hidden() {
        use super::{Commands, NoteCmd};
        for (args, ok) in [
            (vec!["plexi", "notes"], true),
            (vec!["plexi", "notes", "list"], true),
            (vec!["plexi", "notes", "open"], true),
        ] {
            let c = Cli::try_parse_from(args).unwrap();
            assert!(ok && matches!(c.command, Some(Commands::Notes { .. })));
        }
        let c = Cli::try_parse_from(["plexi", "notes", "list"]).unwrap();
        assert!(matches!(
            c.command,
            Some(Commands::Notes {
                cmd: Some(NoteCmd::List)
            })
        ));
        let root = <Cli as clap::CommandFactory>::command();
        assert!(root
            .find_subcommand("notes")
            .is_some_and(|c| c.is_hide_set()));
        assert!(root
            .find_subcommand("note")
            .is_some_and(|c| !c.is_hide_set()));
    }

    #[test]
    fn notify_preserves_post_flags_and_exposes_dismiss_subcommand() {
        let post = Cli::try_parse_from(["plexi", "notify", "--title", "Done", "--timeout", "30"])
            .expect("notify post parses");
        let Some(Commands::Notify { cmd: None, post }) = post.command else {
            panic!("expected notify post");
        };
        assert_eq!(post.title.as_deref(), Some("Done"));
        assert_eq!(post.timeout, 30);

        let dismiss = Cli::try_parse_from(["plexi", "notify", "dismiss", "cli:42:abc"])
            .expect("notify dismiss parses");
        let Some(Commands::Notify {
            cmd: Some(NotifyCmd::Dismiss { notify_id }),
            ..
        }) = dismiss.command
        else {
            panic!("expected notify dismiss");
        };
        assert_eq!(notify_id, "cli:42:abc");
    }

    #[test]
    fn notify_rejects_removed_level_flag() {
        assert!(
            Cli::try_parse_from(["plexi", "notify", "--title", "Done", "--level", "info"]).is_err(),
            "removed notification level flag must fail loudly"
        );
    }

    #[test]
    fn host_start_accepts_ephemeral_flag() {
        let cli = Cli::try_parse_from([
            "plexi",
            "host",
            "start",
            "--ephemeral",
            "--pane",
            "cwd=/tmp",
        ])
        .unwrap();
        let Some(Commands::Host { cmd }) = cli.command else {
            panic!("expected host command");
        };
        let super::HostCmd::Start {
            ephemeral, panes, ..
        } = cmd
        else {
            panic!("expected host start command");
        };
        assert!(ephemeral);
        assert_eq!(panes, ["cwd=/tmp"]);
    }

    #[test]
    fn host_start_accepts_background_flag() {
        let cli = Cli::try_parse_from(["plexi", "host", "start", "--background"]).unwrap();
        let Some(Commands::Host { cmd }) = cli.command else {
            panic!("expected host command");
        };
        let super::HostCmd::Start { background, .. } = cmd else {
            panic!("expected host start command");
        };
        assert!(background);
    }

    #[test]
    fn pane_send_paste_is_explicit_and_conflicts_with_submit() {
        for args in [
            ["plexi", "pane", "send", "42", "hello\nworld", "--paste"],
            ["plexi", "pane", "send", "42", "--paste", "hello\nworld"],
        ] {
            let cli = Cli::try_parse_from(args).unwrap();
            assert!(matches!(cli.command,
                Some(Commands::Pane { cmd: super::PaneCmd::Send { paste: true, submit: false, text, .. } })
                    if text == "hello\nworld"));
        }
        assert!(Cli::try_parse_from([
            "plexi", "pane", "send", "42", "hello", "--paste", "--submit",
        ])
        .is_err());
    }

    #[test]
    fn pane_send_accepts_leading_hyphen_text() {
        // Markdown list markers ("- ") are routine `pane send` payloads;
        // without allow_hyphen_values clap rejects them as unknown flags.
        let cli = Cli::try_parse_from(["plexi", "pane", "send", "42", "- "]).unwrap();
        let Some(Commands::Pane { cmd }) = cli.command else {
            panic!("expected pane command");
        };
        let super::PaneCmd::Send {
            pane_id,
            text,
            submit,
            paste,
        } = cmd
        else {
            panic!("expected pane send command");
        };
        assert_eq!(pane_id, super::PaneTarget::from(42));
        assert_eq!(text, "- ");
        assert!(!submit, "plain `pane send` must stay type-only");
        assert!(!paste, "plain `pane send` must stay text input");
    }

    /// `--submit` must parse on either side of the hyphen-tolerant text
    /// positional: `allow_hyphen_values` makes flag/positional ordering the
    /// kind of thing that silently swallows a flag as text.
    #[test]
    fn pane_send_submit_parses_before_and_after_the_text() {
        for argv in [
            ["plexi", "pane", "send", "42", "hello", "--submit"],
            ["plexi", "pane", "send", "42", "--submit", "hello"],
            ["plexi", "pane", "send", "42", "hello", "-s"],
        ] {
            let cli = Cli::try_parse_from(argv).unwrap();
            let Some(Commands::Pane { cmd }) = cli.command else {
                panic!("expected pane command for {argv:?}");
            };
            let super::PaneCmd::Send {
                pane_id,
                text,
                submit,
                ..
            } = cmd
            else {
                panic!("expected pane send command for {argv:?}");
            };
            assert_eq!(pane_id, super::PaneTarget::from(42));
            assert_eq!(text, "hello", "text mis-parsed for {argv:?}");
            assert!(submit, "--submit not seen for {argv:?}");
        }
    }

    #[test]
    fn global_socket_override_parses_after_subcommand() {
        let cli = Cli::try_parse_from(["plexi", "pane", "list", "--socket", "/tmp/explicit.sock"])
            .unwrap();

        assert_eq!(
            cli.socket,
            Some(std::path::PathBuf::from("/tmp/explicit.sock"))
        );
    }

    #[test]
    fn secret_list_accepts_global_flag() {
        let cli = Cli::try_parse_from(["plexi", "secret", "list", "--global"]).unwrap();

        let Some(Commands::Secret { cmd }) = cli.command else {
            panic!("expected secret command");
        };
        let SecretCmd::List { global, .. } = cmd else {
            panic!("expected secret list command");
        };
        assert!(global);
        assert!(!cli.json);
    }

    #[test]
    fn secret_list_accepts_json_flag() {
        let cli = Cli::try_parse_from(["plexi", "secret", "list", "--json"]).unwrap();
        assert!(cli.json);
        assert!(matches!(
            cli.command,
            Some(Commands::Secret {
                cmd: SecretCmd::List { global: false, .. }
            })
        ));
    }

    #[test]
    fn secrets_alias_routes_to_secret_command() {
        let cli = Cli::try_parse_from(["plexi", "secrets", "list"]).unwrap();

        let Some(Commands::Secret { cmd }) = cli.command else {
            panic!("expected secret command");
        };
        assert!(!cli.json);
        assert!(matches!(cmd, SecretCmd::List { global: false, .. }));
    }

    #[test]
    fn removed_update_apps_alias_is_rejected() {
        assert!(Cli::try_parse_from(["plexi", "update", "apps"]).is_err());
    }

    #[test]
    fn removed_pane_set_title_alias_is_rejected() {
        assert!(Cli::try_parse_from(["plexi", "pane", "set-title", "Example"]).is_err());
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
    fn app_open_mcp_accepts_only_a_server_id() {
        let cli = Cli::try_parse_from(["plexi", "app", "open", "--mcp", "filesystem"]).unwrap();
        let Some(Commands::App { cmd: AppCmd::Open { mcp, .. } }) = cli.command else {
            panic!("expected MCP app open");
        };
        assert_eq!(mcp, ["filesystem"]);
        assert!(Cli::try_parse_from(["plexi", "app", "open", "--mcp", "npx", "server", "/tmp"]).is_err());
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
    fn pane_list_accepts_json_flag() {
        let cli = Cli::try_parse_from(["plexi", "pane", "list", "--json"]).unwrap();
        let Some(Commands::Pane {
            cmd: super::PaneCmd::List { context },
        }) = cli.command
        else {
            panic!("expected pane list command");
        };
        assert_eq!(context, None);
        assert!(cli.json);
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

    #[test]
    fn app_init_wasm_accepts_name_as_flag_value() {
        let cli = Cli::try_parse_from(["plexi", "app", "init", "--wasm", "counter"]).unwrap();

        let Some(Commands::App { cmd }) = cli.command else {
            panic!("expected app command");
        };
        let AppCmd::Init {
            name, wasm, lang, ..
        } = cmd
        else {
            panic!("expected app init command");
        };
        assert_eq!(name, None);
        assert_eq!(wasm.as_deref(), Some("counter"));
        assert_eq!(lang, "python");
    }

    #[test]
    fn app_init_adopt_accepts_dir_in_any_position_or_defaults_to_cwd() {
        for (argv, expected_dir) in [
            (
                vec!["plexi", "app", "init", "--adopt", "apps/calc"],
                Some("apps/calc"),
            ),
            (
                vec!["plexi", "app", "init", "apps/calc", "--adopt"],
                Some("apps/calc"),
            ),
            (vec!["plexi", "app", "init", "--adopt"], None),
        ] {
            let cli = Cli::try_parse_from(argv).expect("app init --adopt parses");
            let Some(Commands::App {
                cmd: AppCmd::Init { name, adopt, .. },
            }) = cli.command
            else {
                panic!("expected app init command");
            };
            assert!(adopt);
            assert_eq!(name.as_deref(), expected_dir);
        }
    }

    #[test]
    fn bare_ledger_is_the_per_client_summary() {
        let cli = Cli::try_parse_from(["plexi", "ledger"]).expect("bare ledger parses");
        assert!(matches!(cli.command, Some(Commands::Ledger { cmd: None })));

        let summary = Cli::try_parse_from([
            "plexi",
            "ledger",
            "summary",
            "--by",
            "kind",
            "--since",
            "2026-01-01",
            "--json",
        ])
        .expect("ledger summary parses");
        let Some(Commands::Ledger {
            cmd: Some(super::LedgerCmd::Summary { by, since }),
        }) = summary.command
        else {
            panic!("expected ledger summary");
        };
        assert_eq!(by.as_deref(), Some("kind"));
        assert_eq!(since.as_deref(), Some("2026-01-01"));
        assert!(summary.json);
    }

    fn slot_pane_id(args: &[&str]) -> Option<Option<super::PaneTarget>> {
        let cli = Cli::try_parse_from(args).ok()?;
        let Some(Commands::Pane {
            cmd: super::PaneCmd::Slot { cmd },
        }) = cli.command
        else {
            panic!("expected pane slot");
        };
        Some(match cmd {
            super::PaneSlotCmd::Read {
                pane_id,
                pane_id_flag,
                ..
            }
            | super::PaneSlotCmd::Wait {
                pane_id,
                pane_id_flag,
                ..
            }
            | super::PaneSlotCmd::Delete {
                pane_id,
                pane_id_flag,
                ..
            }
            | super::PaneSlotCmd::List {
                pane_id,
                pane_id_flag,
            } => pane_id.or(pane_id_flag),
            super::PaneSlotCmd::Write { pane_id, .. } => pane_id,
        })
    }

    #[test]
    fn pane_slot_verbs_accept_flag_and_positional_pane_id() {
        let verbs: [(&'static [&'static str], &'static [&'static str]); 4] = [
            (&["read", "status"], &[]),
            (&["wait", "status"], &["--until", "x"]),
            (&["list"], &[]),
            (&["delete", "status"], &[]),
        ];
        for (verb, extra) in verbs {
            let base: [&'static str; 3] = ["plexi", "pane", "slot"];
            let build = |tail: &'static [&'static str]| -> Vec<&'static str> {
                base.iter()
                    .chain(verb.iter())
                    .chain(tail.iter())
                    .chain(extra.iter())
                    .copied()
                    .collect()
            };
            assert_eq!(
                slot_pane_id(&build(&["--pane-id", "42"])),
                Some(Some(super::PaneTarget::from(42)))
            );
            assert_eq!(
                slot_pane_id(&build(&["42"])),
                Some(Some(super::PaneTarget::from(42)))
            );
            assert_eq!(
                slot_pane_id(&build(&["worker-1"])),
                Some(Some(super::PaneTarget::from("worker-1")))
            );
            assert_eq!(slot_pane_id(&build(&[])), Some(None));
            assert_eq!(
                slot_pane_id(&build(&["42", "--pane-id", "43"])),
                None,
                "both spellings must be rejected for {verb:?}"
            );
        }
    }

    #[test]
    fn pane_help_signposts_agent_boot_and_confirmed_submit_recipe() {
        let mut root = <Cli as clap::CommandFactory>::command();
        let render_long_help = |path: &[&str], root: &mut clap::Command| -> String {
            let mut cmd = root
                .find_subcommand_mut(path[0])
                .unwrap_or_else(|| panic!("missing subcommand {}", path[0]));
            for seg in &path[1..] {
                cmd = cmd
                    .find_subcommand_mut(seg)
                    .unwrap_or_else(|| panic!("missing subcommand {seg}"));
            }
            cmd.render_long_help().to_string()
        };

        let pane_help = render_long_help(&["pane"], &mut root);
        assert!(
            pane_help.contains("Driving an agent in another pane"),
            "plexi pane --help must include the agent orientation block:\n{pane_help}"
        );
        assert!(
            pane_help.contains("pane new --agent") && pane_help.contains("pane send") && pane_help.contains("--submit"),
            "plexi pane --help must name the two-step recipe (pane new --agent + pane send --submit):\n{pane_help}"
        );
        assert!(
            pane_help.contains("no sleeps, no capture polling"),
            "plexi pane --help must state no sleeps, no capture polling:\n{pane_help}"
        );

        let pane_new_help = render_long_help(&["pane", "new"], &mut root);
        assert!(
            pane_new_help.contains("supported way to launch an agent")
                && pane_new_help.contains("ready-to-brief"),
            "plexi pane new --help must promote --agent up front and state that a printed id means ready-to-brief:\n{pane_new_help}"
        );
        let agent_opt_pos = pane_new_help
            .find("--agent <TIER_ALIAS>")
            .expect("--agent option must appear in pane new --help");
        let down_opt_pos = pane_new_help
            .find("--down")
            .expect("--down option must appear in pane new --help");
        assert!(
            agent_opt_pos < down_opt_pos,
            "--agent must be promoted ahead of split placement flags in pane new --help:\n{pane_new_help}"
        );

        let pane_send_help = render_long_help(&["pane", "send"], &mut root);
        assert!(
            pane_send_help.contains("--submit") && pane_send_help.contains("host-confirmed"),
            "plexi pane send --help must cross-reference --submit for confirmed submission:\n{pane_send_help}"
        );

        let pane_key_help = render_long_help(&["pane", "key"], &mut root);
        assert!(
            pane_key_help.contains("pane send") && pane_key_help.contains("--submit"),
            "plexi pane key --help must cross-reference pane send --submit for confirmed submission:\n{pane_key_help}"
        );
    }

    #[test]
    fn agent_help_distinguishes_interactive_start_spawn_from_script_jobs_without_capture_send_polling() {
        let mut root = <Cli as clap::CommandFactory>::command();
        let render_long_help = |path: &[&str], root: &mut clap::Command| -> String {
            let mut cmd = root
                .find_subcommand_mut(path[0])
                .unwrap_or_else(|| panic!("missing subcommand {}", path[0]));
            for seg in &path[1..] {
                cmd = cmd
                    .find_subcommand_mut(seg)
                    .unwrap_or_else(|| panic!("missing subcommand {seg}"));
            }
            cmd.render_long_help().to_string()
        };

        let agent_help = render_long_help(&["agent"], &mut root);
        assert!(
            agent_help.contains("interactive adapter launches")
                && agent_help.contains("Script job is one-shot")
                && agent_help.contains("without capture/send polling"),
            "plexi agent --help must clarify interactive start/spawn vs one-shot script jobs without capture/send polling:\n{agent_help}"
        );

        let agent_start_help = render_long_help(&["agent", "start"], &mut root);
        assert!(
            agent_start_help.contains("interactive adapter launches")
                && agent_start_help.contains("Script job is one-shot")
                && agent_start_help.contains("without capture/send polling"),
            "plexi agent start --help must clarify interactive start/spawn vs one-shot script jobs without capture/send polling:\n{agent_start_help}"
        );

        let agent_spawn_help = render_long_help(&["agent", "spawn"], &mut root);
        assert!(
            agent_spawn_help.contains("interactive adapter launches")
                && agent_spawn_help.contains("Script job is one-shot")
                && agent_spawn_help.contains("without capture/send polling"),
            "plexi agent spawn --help must clarify interactive start/spawn vs one-shot script jobs without capture/send polling:\n{agent_spawn_help}"
        );
    }

    #[test]
    fn pane_info_accepts_a_positional_pane_id() {
        use super::{Commands, PaneCmd};
        let parse = |args: &[&str]| Cli::try_parse_from(args);
        let cli = parse(&["plexi", "pane", "info", "42"]).expect("pane info 42");
        assert!(matches!(
            cli.command,
            Some(Commands::Pane { cmd: PaneCmd::Info { pane_id: Some(ref t), previous: None } })
                if t == &super::PaneTarget::from(42)
        ));
        let cli = parse(&["plexi", "pane", "info"]).expect("pane info");
        assert!(matches!(
            cli.command,
            Some(Commands::Pane { cmd: PaneCmd::Info { pane_id: None, previous: None } })
        ));
        assert!(parse(&["plexi", "pane", "info", "42", "--previous"]).is_err());
    }

    #[test]
    fn pane_result_parses_shorthand_read_and_subcommands() {
        use super::{Commands, PaneCmd};

        let parse = |args: &[&str]| Cli::try_parse_from(args);

        // `plexi pane result 42`
        let cli = parse(&["plexi", "pane", "result", "42"]).expect("pane result 42");
        assert!(matches!(
            cli.command,
            Some(Commands::Pane {
                cmd: PaneCmd::Result {
                    cmd: None,
                    pane_id: Some(ref t),
                    wait: false,
                    ..
                }
            }) if t == &super::PaneTarget::from(42)
        ));

        // `plexi pane result 42 --wait --timeout 15`
        let cli = parse(&["plexi", "pane", "result", "42", "--wait", "--timeout", "15"])
            .expect("pane result 42 --wait");
        assert!(matches!(
            cli.command,
            Some(Commands::Pane {
                cmd: PaneCmd::Result {
                    cmd: None,
                    pane_id: Some(ref t),
                    wait: true,
                    timeout: tm,
                    ..
                }
            }) if t == &super::PaneTarget::from(42) && (tm - 15.0).abs() < f64::EPSILON
        ));

        // The write/schema verbs were folded into `plexi agent done`.
        assert!(parse(&["plexi", "pane", "result", "write", r#"{"status":"pass"}"#]).is_err()
            || matches!(
                parse(&["plexi", "pane", "result", "write", r#"{"status":"pass"}"#]).unwrap().command,
                Some(Commands::Pane { cmd: PaneCmd::Result { cmd: None, .. } })
            ));
        assert!(parse(&["plexi", "pane", "result", "42", "--write", "{}"]).is_err());
        assert!(parse(&["plexi", "pane", "result", "--schema", "{}"]).is_err());

        // `plexi pane new --agent c-large --result-schema schema.json`
        let cli = parse(&[
            "plexi",
            "pane",
            "new",
            "--agent",
            "c-large",
            "--result-schema",
            "schema.json",
        ])
        .expect("pane new --result-schema");
        assert!(matches!(
            cli.command,
            Some(Commands::Pane {
                cmd: PaneCmd::New {
                    result_schema: Some(ref s),
                    ..
                }
            }) if s == "schema.json"
        ));
    }

    #[test]
    fn resolve_target_context_and_pane_exact_prefix_ambiguous_unknown_and_numeric() {
        use super::{resolve_target_from_candidates, ContextTarget, PaneTarget};

        let contexts = vec![
            (1, "main".to_string()),
            (2, "sprint-alpha".to_string()),
            (3, "sprint-beta".to_string()),
            (4, "sprint".to_string()),
        ];

        // Numeric id resolves without consulting candidates.
        assert_eq!(ContextTarget::from("42").numeric_id(), Some(42));
        assert_eq!(
            resolve_target_from_candidates("context", "42", &[]),
            Ok(42)
        );

        // Exact name match wins even when it is a prefix of other candidates.
        assert_eq!(
            resolve_target_from_candidates("context", "sprint", &contexts),
            Ok(4)
        );
        assert_eq!(
            resolve_target_from_candidates("context", "main", &contexts),
            Ok(1)
        );

        // Unique prefix match resolves.
        assert_eq!(
            resolve_target_from_candidates("context", "sprint-a", &contexts),
            Ok(2)
        );
        assert_eq!(
            resolve_target_from_candidates("context", "ma", &contexts),
            Ok(1)
        );

        // Ambiguous prefix errors and lists matching candidates as `name (id)`.
        let err = resolve_target_from_candidates("context", "sprint-", &contexts)
            .expect_err("sprint- is ambiguous");
        assert!(err.contains("ambiguous context 'sprint-'"), "got: {err}");
        assert!(err.contains("sprint-alpha (2)"), "got: {err}");
        assert!(err.contains("sprint-beta (3)"), "got: {err}");

        // Unknown name errors and lists nearby candidates as `name (id)`.
        let err = resolve_target_from_candidates("context", "spriX", &contexts)
            .expect_err("spriX is unknown");
        assert!(err.contains("unknown context 'spriX'"), "got: {err}");
        assert!(err.contains("sprint (4)"), "got: {err}");

        // Pane targets follow the exact same contract.
        let panes = vec![
            (10, "worker-0890".to_string()),
            (11, "worker-0891".to_string()),
            (12, "tester".to_string()),
            (13, "worker".to_string()),
        ];
        assert_eq!(PaneTarget::from("99").numeric_id(), Some(99));
        assert_eq!(resolve_target_from_candidates("pane", "99", &[]), Ok(99));
        assert_eq!(
            resolve_target_from_candidates("pane", "worker", &panes),
            Ok(13)
        );
        assert_eq!(
            resolve_target_from_candidates("pane", "tes", &panes),
            Ok(12)
        );
        let err = resolve_target_from_candidates("pane", "worker-0", &panes)
            .expect_err("worker-0 is ambiguous");
        assert!(err.contains("ambiguous pane 'worker-0'"), "got: {err}");
        assert!(err.contains("worker-0890 (10)"), "got: {err}");
        assert!(err.contains("worker-0891 (11)"), "got: {err}");
        let err = resolve_target_from_candidates("pane", "unknown-pane", &panes)
            .expect_err("unknown-pane is unknown");
        assert!(err.contains("unknown pane 'unknown-pane'"), "got: {err}");
        assert!(err.contains("tester (12)"), "got: {err}");
    }

    #[test]
    fn every_leaf_subcommand_accepts_json_flag() {
        use clap::{ArgAction, CommandFactory};

        fn collect_leaf_paths(cmd: &clap::Command, prefix: Vec<String>, out: &mut Vec<(Vec<String>, clap::Command)>) {
            let subs: Vec<_> = cmd
                .get_subcommands()
                .filter(|s| s.get_name() != "help")
                .collect();
            if subs.is_empty() {
                out.push((prefix, cmd.clone()));
                return;
            }
            for sub in subs {
                let mut next = prefix.clone();
                next.push(sub.get_name().to_string());
                collect_leaf_paths(sub, next, out);
            }
        }

        let mut root = Cli::command();
        root.build();
        let mut leaves = Vec::new();
        collect_leaf_paths(&root, vec!["plexi".to_string()], &mut leaves);
        assert!(!leaves.is_empty(), "expected leaf subcommands");

        for (path, leaf_cmd) in leaves {
            let mut argv = path.clone();
            // Synthesize required arguments for this leaf command so `Cli::try_parse_from`
            // reaches full argument validation with `--json`.
            for arg in leaf_cmd.get_arguments() {
                let id = arg.get_id().as_str();
                if id == "help" || id == "version" || id == "json" || id == "socket" {
                    continue;
                }
                let needs_value = arg.is_required_set()
                    || id == "text" && path.ends_with(&["assistant".to_string(), "send".to_string()])
                    || id == "approve" && path.ends_with(&["needs-you".to_string(), "resolve".to_string()])
                    || id == "name" && path.ends_with(&["app".to_string(), "init".to_string()])
                    || id == "pane_id" && path.ends_with(&["pane".to_string(), "focus".to_string()])
                    || id == "persona" && path.ends_with(&["agent".to_string(), "create".to_string()])
                    || id == "target"
                        && (path.ends_with(&[
                            "agent".to_string(),
                            "fleet".to_string(),
                            "dismiss".to_string(),
                        ]) || path.ends_with(&[
                            "agent".to_string(),
                            "fleet".to_string(),
                            "interrupt".to_string(),
                        ]));
                if !needs_value {
                    continue;
                }
                let sample_val = arg
                    .get_value_parser()
                    .possible_values()
                    .and_then(|mut pv| pv.next().map(|v| v.get_name().to_string()))
                    .unwrap_or_else(|| "1".to_string());
                if arg.is_positional() {
                    argv.push(sample_val);
                } else if let Some(long) = arg.get_long() {
                    argv.push(format!("--{long}"));
                    if !matches!(
                        arg.get_action(),
                        ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count
                    ) {
                        argv.push(sample_val);
                    }
                }
            }
            argv.push("--json".to_string());
            let parsed = Cli::try_parse_from(&argv).unwrap_or_else(|e| {
                panic!("leaf command {:?} failed to parse with --json (argv={argv:?}): {e}", path)
            });
            assert!(
                parsed.json,
                "leaf command {:?} parsed --json but cli.json was false",
                path
            );
        }
    }

    #[test]
    fn pane_unhide_alias_parses_to_reveal() {
        use super::PaneCmd;
        use clap::CommandFactory;

        let cli = Cli::try_parse_from(["plexi", "pane", "unhide"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Pane {
                cmd: PaneCmd::Reveal { pane_id: None }
            })
        ));

        let cli_with_target = Cli::try_parse_from(["plexi", "pane", "unhide", "42"]).unwrap();
        assert!(matches!(
            cli_with_target.command,
            Some(Commands::Pane {
                cmd: PaneCmd::Reveal {
                    pane_id: Some(ref t)
                }
            }) if *t == 42
        ));

        let mut root = Cli::command();
        let pane_sub = root.find_subcommand_mut("pane").unwrap();
        let help = pane_sub.render_long_help().to_string();
        assert!(
            help.contains("unhide"),
            "expected `pane --help` to show visible alias `unhide`, got:\n{help}"
        );
    }
}
