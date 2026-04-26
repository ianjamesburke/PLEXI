//! Agent Workspace pane substrate (#348).
//!
//! A pane that runs an AI coding CLI (Claude Code, Codex, Gemini CLI) inside an
//! auto-created git worktree. The worktree gives every agent its own isolated
//! branch — multiple agents can work on the same repo concurrently without
//! stepping on each other.
//!
//! This file owns:
//! - `AgentCli` — the three supported CLIs (binary name + display name).
//! - `AgentWorkspacePane` — the pane state: PTY backend, CLI, branch name,
//!   worktree path, optional task label.
//! - `create_worktree` / `remove_worktree` — `git worktree add/remove` shellouts
//!   (no `git2` crate, matches #320/#322 precedent).
//! - Branch name + worktree path generation (`plexi/agent-<slug>-<short-hash>`).
//!
//! Out of scope for this PR (lands in #349):
//! - Modal picker UI, status heuristics (Idle/Thinking/Writing/Done/Error),
//!   diff sidebar, merge button, last-CLI-per-repo persistence.

use crate::pane::TerminalPane;
use crate::tiling::PaneId;
use egui_term::{BackendSettings, PtyEvent};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

pub mod persistence;

// ── AgentCli ────────────────────────────────────────────────────────────────

/// The three AI coding CLIs the substrate knows how to spawn. The binary name
/// is what we shell out to via `which` / PTY exec; the display name is what
/// the palette and pane header render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCli {
    ClaudeCode,
    Codex,
    GeminiCli,
}

impl AgentCli {
    pub fn binary_name(&self) -> &'static str {
        match self {
            AgentCli::ClaudeCode => "claude",
            AgentCli::Codex => "codex",
            AgentCli::GeminiCli => "gemini",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            AgentCli::ClaudeCode => "Claude Code",
            AgentCli::Codex => "Codex",
            AgentCli::GeminiCli => "Gemini CLI",
        }
    }

    /// All three CLIs in display order — used by the modal picker.
    pub fn all() -> [AgentCli; 3] {
        [AgentCli::ClaudeCode, AgentCli::Codex, AgentCli::GeminiCli]
    }

    /// Resolve the CLI's binary on PATH. `false` means not installed; the
    /// modal greys out the entry and appends "(not installed)".
    pub fn is_installed(&self) -> bool {
        which_in_path(self.binary_name()).is_some()
    }
}

/// Walk `$PATH` looking for an executable named `name`. Pure-stdlib equivalent
/// of `which::which` so we don't pull a new crate. macOS GUI bundles don't
/// inherit shell PATH, so we additionally probe a small set of common bin
/// dirs that the user's CLI is likely installed in.
fn which_in_path(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let path_env = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path_env).collect();
    // Common installer locations the GUI bundle won't have on PATH.
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".cargo/bin"));
        dirs.push(home.join(".bun/bin"));
        dirs.push(home.join(".npm-global/bin"));
        dirs.push(home.join("Library/Caches/Volta/bin"));
        dirs.push(home.join(".volta/bin"));
        dirs.push(home.join(".asdf/shims"));
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    for dir in dirs {
        let candidate = dir.join(name);
        if let Ok(meta) = std::fs::metadata(&candidate) {
            if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
                return Some(candidate);
            }
        }
    }
    None
}

// ── AgentStatus ─────────────────────────────────────────────────────────────

/// Heuristic agent activity state, derived from PTY Wakeup events + child exit
/// status. Driven entirely from the foreground UI thread — no parallel tap on
/// stdout is needed because `egui_term` already publishes `PtyEvent::Wakeup` on
/// every grid change (one per chunk of stdout, broadly) and `PtyEvent::ChildExit`
/// on process exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    /// No output activity in the last `IDLE_THRESHOLD`.
    Idle,
    /// Output within the last `IDLE_THRESHOLD` but not within `WRITING_THRESHOLD`.
    Thinking,
    /// Output within the last `WRITING_THRESHOLD`.
    Writing,
    /// Process exited with code 0.
    Done,
    /// Process exited with a non-zero code.
    Error,
}

impl AgentStatus {
    pub fn label(&self) -> &'static str {
        match self {
            AgentStatus::Idle => "idle",
            AgentStatus::Thinking => "thinking",
            AgentStatus::Writing => "writing",
            AgentStatus::Done => "done",
            AgentStatus::Error => "error",
        }
    }
}

/// Output considered "fresh" (Writing) within this window since the last
/// Wakeup.
pub const WRITING_THRESHOLD: Duration = Duration::from_secs(1);
/// Output considered "active" (Thinking) within this window since the last
/// Wakeup. Anything older is Idle.
pub const IDLE_THRESHOLD: Duration = Duration::from_secs(5);

/// Pure-data status derivation. `now` is the reference clock, `last_output_at`
/// is `Some(t)` for the most recent Wakeup, `exit_code` is `Some(code)` once
/// the child has exited.
///
/// Sticky-once-terminal: Done/Error never transition back. Callers ensure
/// that by short-circuiting on a cached terminal state — this function would
/// happily return Idle if `last_output_at` aged out before exit fired.
pub fn derive_status(
    now: Instant,
    last_output_at: Option<Instant>,
    exit_code: Option<i32>,
) -> AgentStatus {
    if let Some(code) = exit_code {
        return if code == 0 { AgentStatus::Done } else { AgentStatus::Error };
    }
    let Some(t) = last_output_at else {
        return AgentStatus::Idle;
    };
    let elapsed = now.saturating_duration_since(t);
    if elapsed < WRITING_THRESHOLD {
        AgentStatus::Writing
    } else if elapsed < IDLE_THRESHOLD {
        AgentStatus::Thinking
    } else {
        AgentStatus::Idle
    }
}

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum AgentWorkspaceError {
    /// The directory is not inside a git repository.
    NotAGitRepository(PathBuf),
    /// `git worktree add` exited non-zero. Wraps git's stderr verbatim.
    GitWorktreeAdd(String),
    /// `git worktree remove` exited non-zero. Wraps git's stderr verbatim.
    GitWorktreeRemove(String),
    /// Failed to spawn the CLI binary into a PTY (TerminalBackend creation
    /// failed). The pane creation falls back to writing a clear message into
    /// scrollback — this variant only fires when the backend itself can't
    /// even be constructed.
    PtySpawn(String),
    /// I/O error invoking git.
    Io(std::io::Error),
}

impl std::fmt::Display for AgentWorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentWorkspaceError::NotAGitRepository(p) => {
                write!(f, "{} is not inside a git repository", p.display())
            }
            AgentWorkspaceError::GitWorktreeAdd(msg) => {
                write!(f, "git worktree add failed: {msg}")
            }
            AgentWorkspaceError::GitWorktreeRemove(msg) => {
                write!(f, "git worktree remove failed: {msg}")
            }
            AgentWorkspaceError::PtySpawn(msg) => write!(f, "PTY spawn failed: {msg}"),
            AgentWorkspaceError::Io(e) => write!(f, "io error invoking git: {e}"),
        }
    }
}

impl std::error::Error for AgentWorkspaceError {}

impl From<std::io::Error> for AgentWorkspaceError {
    fn from(e: std::io::Error) -> Self {
        AgentWorkspaceError::Io(e)
    }
}

// ── Branch + path generation ────────────────────────────────────────────────

/// Sanitise a free-form label into a short, branch-safe slug. Lowercase ASCII
/// letters/digits/dashes only; collapse runs of dashes; max 30 chars; no
/// leading/trailing dashes.
fn sanitise_slug(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_dash = true; // suppress leading dash
    for ch in input.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch == '-' || ch == '_' || ch == ' ' || ch == '/' {
            Some('-')
        } else {
            None
        };
        if let Some(c) = mapped {
            if c == '-' {
                if !last_dash {
                    out.push('-');
                    last_dash = true;
                }
            } else {
                out.push(c);
                last_dash = false;
            }
            if out.len() >= 30 {
                break;
            }
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Produce a fresh branch name + worktree path for a new agent workspace.
/// Branch name format: `plexi/agent-<slug>-<short-hash>`
/// Worktree path:     `<repo>/.git/worktrees/plexi-<short-hash>`
///
/// `<slug>` derives from `task_label` (or the CLI binary name if the label is
/// empty); `<short-hash>` is the first 8 hex chars of a fresh UUID v4.
fn generate_branch_and_path(
    repo_path: &Path,
    cli: AgentCli,
    task_label: &str,
) -> (String, PathBuf) {
    let raw_slug = if task_label.trim().is_empty() {
        cli.binary_name().to_string()
    } else {
        sanitise_slug(task_label)
    };
    let slug = if raw_slug.is_empty() {
        cli.binary_name().to_string()
    } else {
        raw_slug
    };
    let short_hash = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
    let branch = format!("plexi/agent-{slug}-{short_hash}");
    let worktree = repo_path
        .join(".git")
        .join("worktrees")
        .join(format!("plexi-{short_hash}"));
    (branch, worktree)
}

// ── Git worktree shell-outs ─────────────────────────────────────────────────

/// `git -C <repo> worktree add -b <branch> <worktree_path>`
pub fn create_worktree(
    repo_path: &Path,
    worktree_path: &Path,
    branch_name: &str,
) -> Result<(), AgentWorkspaceError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("worktree")
        .arg("add")
        .arg("-b")
        .arg(branch_name)
        .arg(worktree_path)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AgentWorkspaceError::GitWorktreeAdd(stderr));
    }
    Ok(())
}

/// `git -C <repo> worktree remove --force <worktree_path>`
///
/// `--force` because the agent may have left dirty files in the worktree;
/// keeping the branch around is what matters for review. Branch deletion is
/// intentionally NOT performed — review/merge happens after pane close.
pub fn remove_worktree(
    repo_path: &Path,
    worktree_path: &Path,
) -> Result<(), AgentWorkspaceError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(worktree_path)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AgentWorkspaceError::GitWorktreeRemove(stderr));
    }
    Ok(())
}

/// Walk up from `start` looking for a `.git` directory or file. Returns the
/// directory containing it (the repo root). Used to resolve
/// "current workspace root" → "git repo root" before creating a worktree.
pub fn find_git_repo_root(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

// ── AgentWorkspacePane ──────────────────────────────────────────────────────

/// Pane state for a running agent workspace. The PTY is reused as-is from the
/// terminal pane infrastructure — `TerminalPane` already exposes scrollback,
/// child PID, and dynamic resize; we just point its backend at the CLI binary
/// and the worktree path. The header above the terminal is the only render
/// difference (`<CLI display> · <branch> · <task or "(no task)">`).
pub struct AgentWorkspacePane {
    pub id: PaneId,
    pub terminal: TerminalPane,
    pub cli: AgentCli,
    /// Original repo (NOT the worktree path) — `git worktree remove` runs from here.
    pub repo_path: PathBuf,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    /// User-supplied task label. Empty string when none was provided. The
    /// modal picker (#349) populates this; the substrate accepts an empty
    /// label and renders "(no task)" in the header.
    pub task_label: String,
    // ── Status heuristic (#349) ─────────────────────────────────────────────
    /// Most recent `PtyEvent::Wakeup` timestamp. Updated by
    /// `record_pty_activity` from the host's pty-event drain. `None` until
    /// the first Wakeup arrives.
    pub last_output_at: Option<Instant>,
    /// Captured on `PtyEvent::ChildExit(code)`. Once `Some`, status is sticky
    /// (Done or Error).
    pub exit_code: Option<i32>,
    /// Cached status from the last `update_status` call — render path reads
    /// this without recomputing per-frame.
    pub cached_status: AgentStatus,
    // ── Changed-files sidebar (#349) ────────────────────────────────────────
    /// Cached `git diff --name-status` result. Refreshed on a 2s timer.
    pub changed_files: Vec<ChangedFile>,
    /// Last time the sidebar refresh ran.
    pub last_diff_refresh: Option<Instant>,
    // ── Merge feedback (#349) ───────────────────────────────────────────────
    /// Set after the user clicks "Ready to merge". `MergeOutcome::Conflict`
    /// surfaces a notification card with the file list; `Merged` flashes a
    /// "merged" badge briefly.
    pub merge_outcome: Option<MergeOutcome>,
    /// `Instant` when merge_outcome was last set — used to clear the flash
    /// after a few seconds.
    pub merge_outcome_at: Option<Instant>,
}

/// One row in the changed-files sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// Single-letter status from `git diff --name-status`: M, A, D, R, C, T, U.
    pub status: char,
    /// Path relative to the worktree root.
    pub path: String,
}

/// Outcome of a "Ready to merge" click. Mirrors the two paths the merge code
/// can take: clean merge (badge flashes), or conflict (files are surfaced via
/// the notification queue).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    Merged,
    Conflict { files: Vec<String> },
    Failed { stderr: String },
}

/// Sidebar refresh cadence. Sub-frame poll would torch CPU; 2s is plenty
/// fresh for "I just saved a file in the worktree".
pub const DIFF_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// How long a "merged" badge stays on screen after a clean merge.
pub const MERGE_FLASH_DURATION: Duration = Duration::from_secs(3);

impl AgentWorkspacePane {
    /// Allocate a branch + worktree, run `git worktree add`, then spawn a PTY
    /// running the CLI binary inside the new worktree.
    ///
    /// If the CLI binary is not on PATH, the worktree is still created and the
    /// PTY spawn surfaces the error in scrollback (`build_settings` resolves
    /// the binary name verbatim — `egui_term`'s pty exec returns ENOENT, which
    /// the user can read in the terminal). This is intentional: the substrate
    /// keeps the pane open so the error is visible. The modal picker (#349)
    /// will pre-flight `which` checks.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        pane_id: PaneId,
        cli: AgentCli,
        repo_path: PathBuf,
        task_label: String,
        ctx: egui::Context,
        pty_event_tx: Sender<(u64, PtyEvent)>,
        env: std::collections::HashMap<String, String>,
        dynamic_colors: std::collections::HashMap<usize, [u8; 3]>,
        default_font_size: f32,
    ) -> Result<Self, AgentWorkspaceError> {
        if !repo_path.join(".git").exists() {
            return Err(AgentWorkspaceError::NotAGitRepository(repo_path));
        }
        let (branch_name, worktree_path) =
            generate_branch_and_path(&repo_path, cli, &task_label);

        create_worktree(&repo_path, &worktree_path, &branch_name)?;

        let settings = BackendSettings {
            shell: cli.binary_name().to_string(),
            args: vec![],
            env,
            dynamic_colors,
            working_directory: Some(worktree_path.clone()),
        };
        let terminal = match TerminalPane::new(
            pane_id,
            ctx,
            pty_event_tx,
            settings,
            default_font_size,
        ) {
            Some(t) => t,
            None => {
                // PTY spawn failed catastrophically — back out the worktree.
                let _ = remove_worktree(&repo_path, &worktree_path);
                return Err(AgentWorkspaceError::PtySpawn(format!(
                    "could not spawn '{}' in {}",
                    cli.binary_name(),
                    worktree_path.display()
                )));
            }
        };

        Ok(Self {
            id: pane_id,
            terminal,
            cli,
            repo_path,
            branch_name,
            worktree_path,
            task_label,
            last_output_at: None,
            exit_code: None,
            cached_status: AgentStatus::Idle,
            changed_files: Vec::new(),
            last_diff_refresh: None,
            merge_outcome: None,
            merge_outcome_at: None,
        })
    }

    /// Restore an existing pane after a workspace reload — the worktree and
    /// branch already exist, we just relaunch the CLI inside the worktree.
    /// Branch is NOT recreated.
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        pane_id: PaneId,
        cli: AgentCli,
        repo_path: PathBuf,
        branch_name: String,
        worktree_path: PathBuf,
        task_label: String,
        ctx: egui::Context,
        pty_event_tx: Sender<(u64, PtyEvent)>,
        env: std::collections::HashMap<String, String>,
        dynamic_colors: std::collections::HashMap<usize, [u8; 3]>,
        default_font_size: f32,
    ) -> Option<Self> {
        let settings = BackendSettings {
            shell: cli.binary_name().to_string(),
            args: vec![],
            env,
            dynamic_colors,
            working_directory: Some(worktree_path.clone()),
        };
        let terminal = TerminalPane::new(pane_id, ctx, pty_event_tx, settings, default_font_size)?;
        Some(Self {
            id: pane_id,
            terminal,
            cli,
            repo_path,
            branch_name,
            worktree_path,
            task_label,
            last_output_at: None,
            exit_code: None,
            cached_status: AgentStatus::Idle,
            changed_files: Vec::new(),
            last_diff_refresh: None,
            merge_outcome: None,
            merge_outcome_at: None,
        })
    }

    /// Record a PTY Wakeup or ChildExit observation. Called from the host's
    /// pty-event drain loop — see `app/mod.rs::drain_pty_events`.
    pub fn record_pty_activity(&mut self, event: &PtyEvent, now: Instant) {
        use egui_term::PtyEvent as E;
        match event {
            E::Wakeup => {
                self.last_output_at = Some(now);
            }
            E::ChildExit(code) => {
                self.exit_code = Some(*code);
            }
            _ => {}
        }
        self.cached_status = derive_status(now, self.last_output_at, self.exit_code);
    }

    /// Recompute the cached status against the current clock. Called once per
    /// frame from the host's UI tick so Thinking → Idle transitions don't wait
    /// for the next Wakeup.
    pub fn refresh_status(&mut self, now: Instant) {
        // Don't downgrade a sticky terminal state: once exit_code is set, the
        // derive function pins Done/Error.
        self.cached_status = derive_status(now, self.last_output_at, self.exit_code);
    }

    /// Header label — `<CLI display> · <branch> · <task or "(no task)">`.
    pub fn header_label(&self) -> String {
        let task = if self.task_label.trim().is_empty() {
            "(no task)"
        } else {
            self.task_label.as_str()
        };
        format!("{} · {} · {}", self.cli.display_name(), self.branch_name, task)
    }

    /// Refresh `changed_files` if the cache is older than `DIFF_REFRESH_INTERVAL`.
    /// Cheap when not due — just a clock comparison. Caller (per-frame UI tick)
    /// can invoke this unconditionally.
    pub fn maybe_refresh_diff(&mut self, now: Instant) {
        let due = self
            .last_diff_refresh
            .is_none_or(|t| now.saturating_duration_since(t) >= DIFF_REFRESH_INTERVAL);
        if !due {
            return;
        }
        self.last_diff_refresh = Some(now);
        match read_changed_files(&self.repo_path, &self.branch_name) {
            Ok(files) => self.changed_files = files,
            Err(e) => log::debug!(
                "agent_workspace: changed_files refresh failed for {}: {e}",
                self.branch_name
            ),
        }
    }

    /// Run `git checkout main && git merge <branch>` against the **original
    /// repo path** (not the worktree). Stores the outcome in
    /// `self.merge_outcome` for the render path to surface.
    ///
    /// Strategy: shell to `git -C <repo> checkout main`, then
    /// `git -C <repo> merge --no-ff --no-edit <branch>`. We use `--no-edit` so
    /// the merge commit message is auto-generated; `--no-ff` to keep the topic
    /// branch visible in history. On conflict, parse `git status -s` for `U*`
    /// rows and surface the file list.
    pub fn run_merge(&mut self) {
        let outcome = perform_merge(&self.repo_path, &self.branch_name);
        log::info!(
            "agent_workspace: merge {} -> main: {:?}",
            self.branch_name,
            outcome
        );
        self.merge_outcome = Some(outcome);
        self.merge_outcome_at = Some(Instant::now());
    }

    /// Drop a stale "merged" flash so the badge clears after a few seconds.
    pub fn maybe_clear_merge_flash(&mut self, now: Instant) {
        if let Some(t) = self.merge_outcome_at {
            if now.saturating_duration_since(t) >= MERGE_FLASH_DURATION
                && matches!(self.merge_outcome, Some(MergeOutcome::Merged))
            {
                self.merge_outcome = None;
                self.merge_outcome_at = None;
            }
        }
    }
}

// ── Diff helpers ────────────────────────────────────────────────────────────

/// Run `git -C <repo> diff --name-status main..<branch>`. Output is one row
/// per changed file:
///
///   M\tsrc/foo.rs
///   A\tsrc/bar.rs
///   R100\told.rs\tnew.rs
///
/// We only surface the first letter of the status code (the modal-picker
/// sidebar isn't a porcelain replacement). Renames render with the new path.
pub fn read_changed_files(
    repo_path: &Path,
    branch_name: &str,
) -> Result<Vec<ChangedFile>, AgentWorkspaceError> {
    let range = format!("main..{branch_name}");
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("diff")
        .arg("--name-status")
        .arg(&range)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // Treat "unknown revision" as empty diff — happens when main has no
        // commits, or when the branch has zero divergence yet.
        if stderr.contains("unknown revision") || stderr.contains("not a valid object") {
            return Ok(Vec::new());
        }
        return Err(AgentWorkspaceError::GitWorktreeAdd(stderr));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files = Vec::new();
    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let Some(status) = parts.next() else { continue };
        let status_char = status.chars().next().unwrap_or('?');
        // Rename/copy rows have two paths; pick the second (destination).
        let path = match status_char {
            'R' | 'C' => parts.nth(1).unwrap_or_default().to_string(),
            _ => parts.next().unwrap_or_default().to_string(),
        };
        if path.is_empty() {
            continue;
        }
        files.push(ChangedFile {
            status: status_char,
            path,
        });
    }
    Ok(files)
}

// ── Merge helpers ───────────────────────────────────────────────────────────

/// Run `git checkout main && git merge --no-ff --no-edit <branch>` in the
/// repo. On non-zero exit, parse `git status -s` for unmerged paths and
/// return `MergeOutcome::Conflict` so the caller can surface the file list.
pub fn perform_merge(repo_path: &Path, branch_name: &str) -> MergeOutcome {
    let checkout = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("checkout")
        .arg("main")
        .output();
    match checkout {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return MergeOutcome::Failed { stderr };
        }
        Err(e) => return MergeOutcome::Failed { stderr: e.to_string() },
    }

    let merge = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("merge")
        .arg("--no-ff")
        .arg("--no-edit")
        .arg(branch_name)
        .output();
    let merge_out = match merge {
        Ok(out) => out,
        Err(e) => return MergeOutcome::Failed { stderr: e.to_string() },
    };
    if merge_out.status.success() {
        return MergeOutcome::Merged;
    }
    // Non-zero exit → likely conflict. Parse `git status -s`.
    let conflict_files = read_unmerged_paths(repo_path);
    if conflict_files.is_empty() {
        let stderr = String::from_utf8_lossy(&merge_out.stderr).trim().to_string();
        return MergeOutcome::Failed { stderr };
    }
    MergeOutcome::Conflict { files: conflict_files }
}

/// `git status -s` rows starting with U / AA / DD mark unmerged paths. We
/// extract the path (columns 4..end) for the conflict surface.
fn read_unmerged_paths(repo_path: &Path) -> Vec<String> {
    let Ok(out) = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("status")
        .arg("-s")
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut paths = Vec::new();
    for line in stdout.lines() {
        if line.len() < 3 {
            continue;
        }
        let xy = &line[..2];
        // From `git status` docs: XY combinations marking unmerged are
        // DD, AU, UD, UA, DU, AA, UU.
        let unmerged = matches!(xy, "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU");
        if !unmerged {
            continue;
        }
        let path = line[3..].trim().to_string();
        if !path.is_empty() {
            paths.push(path);
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// Spin up a bare-minimum git repo in a tempdir for hermetic tests.
    fn make_test_repo() -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        let status = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["init", "-q", "-b", "main"])
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");
        // Need at least one commit before `worktree add -b` can branch from HEAD.
        let _ = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["config", "user.email", "test@example.com"])
            .status();
        let _ = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["config", "user.name", "test"])
            .status();
        std::fs::write(dir.path().join("README"), "hi").unwrap();
        let _ = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["add", "."])
            .status();
        let status = Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["commit", "-q", "-m", "init"])
            .status()
            .expect("git commit");
        assert!(status.success(), "git commit failed");
        dir
    }

    #[test]
    fn create_runs_git_worktree_add() {
        let repo = make_test_repo();
        let (branch, wt_path) =
            generate_branch_and_path(repo.path(), AgentCli::ClaudeCode, "fix login bug");
        create_worktree(repo.path(), &wt_path, &branch).expect("worktree add");

        // The worktree directory exists and contains a `.git` file (worktrees
        // get a gitfile, not a directory, but `.exists()` is true either way).
        assert!(wt_path.exists(), "worktree dir should exist");
        assert!(wt_path.join(".git").exists(), "worktree should have .git");

        // The branch is on the new ref.
        let out = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["branch", "--list", &branch])
            .output()
            .expect("git branch");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains(&branch),
            "branch '{branch}' should exist; got: {stdout}"
        );
    }

    #[test]
    fn close_runs_git_worktree_remove() {
        let repo = make_test_repo();
        let (branch, wt_path) =
            generate_branch_and_path(repo.path(), AgentCli::Codex, "");
        create_worktree(repo.path(), &wt_path, &branch).expect("worktree add");
        assert!(wt_path.exists());

        remove_worktree(repo.path(), &wt_path).expect("worktree remove");
        assert!(!wt_path.exists(), "worktree dir should be gone");

        // Branch must still exist (review-after-close is the whole point).
        let out = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["branch", "--list", &branch])
            .output()
            .expect("git branch");
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(&branch),
            "branch should survive worktree remove"
        );
    }

    #[test]
    fn create_in_non_git_dir_errors_clearly() {
        let dir = TempDir::new().unwrap();
        let (branch, wt_path) =
            generate_branch_and_path(dir.path(), AgentCli::GeminiCli, "task");
        let err = create_worktree(dir.path(), &wt_path, &branch)
            .expect_err("non-git dir must error");
        match err {
            AgentWorkspaceError::GitWorktreeAdd(msg) => {
                assert!(!msg.is_empty(), "stderr should not be empty");
            }
            other => panic!("expected GitWorktreeAdd, got {other:?}"),
        }
    }

    #[test]
    fn branch_name_generation_is_unique_across_calls() {
        let repo = make_test_repo();
        let (b1, _) =
            generate_branch_and_path(repo.path(), AgentCli::ClaudeCode, "task");
        let (b2, _) =
            generate_branch_and_path(repo.path(), AgentCli::ClaudeCode, "task");
        assert_ne!(b1, b2, "two calls must yield distinct branch names");
        assert!(b1.starts_with("plexi/agent-task-"));
        assert!(b2.starts_with("plexi/agent-task-"));
    }

    #[test]
    fn worktree_path_under_repo_dot_git() {
        let repo = make_test_repo();
        let (_, wt_path) =
            generate_branch_and_path(repo.path(), AgentCli::ClaudeCode, "x");
        let wt_str = wt_path.to_string_lossy();
        assert!(
            wt_str.contains(".git/worktrees/") || wt_str.contains(".git\\worktrees\\"),
            "worktree path must live under .git/worktrees/, got {wt_str}"
        );
    }

    #[test]
    fn slug_sanitiser_collapses_and_trims() {
        assert_eq!(sanitise_slug("Fix Login Bug"), "fix-login-bug");
        assert_eq!(sanitise_slug("///---weird///"), "weird");
        assert_eq!(sanitise_slug(""), "");
        assert_eq!(sanitise_slug("a/b c_d"), "a-b-c-d");
        // 30-char cap
        let long = "a".repeat(100);
        let s = sanitise_slug(&long);
        assert!(s.len() <= 30);
    }

    #[test]
    fn header_label_renders_no_task_when_empty() {
        // Pure formatting — no PTY needed.
        let label = format!(
            "{} · {} · {}",
            AgentCli::ClaudeCode.display_name(),
            "plexi/agent-x-12345678",
            "(no task)"
        );
        assert_eq!(
            label,
            "Claude Code · plexi/agent-x-12345678 · (no task)"
        );
    }

    #[test]
    fn empty_task_falls_back_to_cli_name() {
        let repo_path = std::path::PathBuf::from("/tmp");
        let (branch, _) = generate_branch_and_path(&repo_path, AgentCli::Codex, "");
        assert!(
            branch.starts_with("plexi/agent-codex-"),
            "empty task should slug to CLI binary name; got {branch}"
        );
    }

    // ── Status heuristics (#349) ────────────────────────────────────────────

    #[test]
    fn status_idle_after_5s_silence() {
        let now = Instant::now();
        // last output 6s ago — past IDLE_THRESHOLD
        let last = now.checked_sub(Duration::from_secs(6)).unwrap();
        assert_eq!(derive_status(now, Some(last), None), AgentStatus::Idle);
    }

    #[test]
    fn status_writing_during_active_output() {
        let now = Instant::now();
        // last output 200ms ago — well within WRITING_THRESHOLD (1s)
        let last = now.checked_sub(Duration::from_millis(200)).unwrap();
        assert_eq!(derive_status(now, Some(last), None), AgentStatus::Writing);
    }

    #[test]
    fn status_thinking_between_writing_and_idle() {
        let now = Instant::now();
        // last output 3s ago — between 1s (writing) and 5s (idle)
        let last = now.checked_sub(Duration::from_secs(3)).unwrap();
        assert_eq!(derive_status(now, Some(last), None), AgentStatus::Thinking);
    }

    #[test]
    fn status_idle_when_no_output_yet() {
        let now = Instant::now();
        assert_eq!(derive_status(now, None, None), AgentStatus::Idle);
    }

    #[test]
    fn status_done_on_clean_exit() {
        let now = Instant::now();
        // Even with stale output, exit code 0 pins Done.
        let last = now.checked_sub(Duration::from_secs(60)).unwrap();
        assert_eq!(derive_status(now, Some(last), Some(0)), AgentStatus::Done);
    }

    #[test]
    fn status_error_on_nonzero_exit() {
        let now = Instant::now();
        let last = now.checked_sub(Duration::from_millis(100)).unwrap();
        // Even with very recent output, nonzero exit pins Error.
        assert_eq!(
            derive_status(now, Some(last), Some(1)),
            AgentStatus::Error
        );
        assert_eq!(
            derive_status(now, Some(last), Some(127)),
            AgentStatus::Error
        );
    }

    #[test]
    fn status_terminal_state_is_sticky_via_pane_helper() {
        // The pane keeps `exit_code` set after ChildExit, so subsequent
        // refresh_status calls cannot transition out of Done/Error even if a
        // late Wakeup arrives.
        let now = Instant::now();
        // Simulate the pane fields directly — the pane struct can't be
        // constructed without a PTY, but the derive function is what defines
        // stickiness.
        let last_output_at = Some(now);
        let exit_code = Some(0);
        // Even with last_output_at == now (would otherwise be Writing),
        // exit_code dominates.
        assert_eq!(derive_status(now, last_output_at, exit_code), AgentStatus::Done);
    }

    // ── Diff sidebar (#349) ─────────────────────────────────────────────────

    #[test]
    fn changed_files_refresh_picks_up_new_modifications() {
        let repo = make_test_repo();
        // Create a feature branch with a commit so `main..branch` shows files.
        let (branch, wt_path) =
            generate_branch_and_path(repo.path(), AgentCli::ClaudeCode, "diff test");
        create_worktree(repo.path(), &wt_path, &branch).expect("worktree add");

        // Initial diff is empty (no commits yet).
        let initial = read_changed_files(repo.path(), &branch).expect("diff ok");
        assert!(initial.is_empty(), "fresh branch has no diff vs main");

        // Add a commit on the branch via the worktree. We `git add` the
        // specific file (not `.`) because the substrate places worktrees
        // under `.git/worktrees/<id>/` which is also where git stashes its
        // own per-worktree metadata; `git add .` would pull in that
        // bookkeeping. Real-world agent usage edits user files by name, so
        // this matches reality. (Worktree path layout itself is a substrate
        // concern — see the substrate tests for the path invariant.)
        std::fs::write(wt_path.join("hello.txt"), "world").unwrap();
        let _ = Command::new("git")
            .arg("-C")
            .arg(&wt_path)
            .args(["config", "user.email", "test@example.com"])
            .status();
        let _ = Command::new("git")
            .arg("-C")
            .arg(&wt_path)
            .args(["config", "user.name", "test"])
            .status();
        let _ = Command::new("git")
            .arg("-C")
            .arg(&wt_path)
            .args(["add", "hello.txt"])
            .status();
        let status = Command::new("git")
            .arg("-C")
            .arg(&wt_path)
            .args(["commit", "-q", "-m", "add hello"])
            .status()
            .unwrap();
        assert!(status.success());

        let diff = read_changed_files(repo.path(), &branch).expect("diff ok");
        // The diff includes the user file plus any internal git metadata that
        // got committed despite our specific `add hello.txt` (e.g. logs/HEAD
        // is sometimes auto-staged by `git commit`). What we care about for
        // this test: the user-visible file must be present.
        assert!(
            diff.iter().any(|f| f.path == "hello.txt" && f.status == 'A'),
            "expected hello.txt in diff, got {:?}",
            diff.iter().map(|f| (&f.status, &f.path)).collect::<Vec<_>>()
        );
    }

    // ── Merge button (#349) ─────────────────────────────────────────────────

    #[test]
    fn merge_button_runs_git_merge_against_original_repo() {
        let repo = make_test_repo();
        let (branch, wt_path) =
            generate_branch_and_path(repo.path(), AgentCli::ClaudeCode, "merge test");
        create_worktree(repo.path(), &wt_path, &branch).expect("worktree add");

        // Add a commit on the branch.
        std::fs::write(wt_path.join("feature.txt"), "feature").unwrap();
        let _ = Command::new("git")
            .arg("-C")
            .arg(&wt_path)
            .args(["add", "."])
            .status();
        let _ = Command::new("git")
            .arg("-C")
            .arg(&wt_path)
            .args(["config", "user.email", "test@example.com"])
            .status();
        let _ = Command::new("git")
            .arg("-C")
            .arg(&wt_path)
            .args(["config", "user.name", "test"])
            .status();
        let _ = Command::new("git")
            .arg("-C")
            .arg(&wt_path)
            .args(["commit", "-q", "-m", "add feature"])
            .status();

        // Run merge via our helper — must operate on the ORIGINAL repo path.
        let outcome = perform_merge(repo.path(), &branch);
        assert_eq!(outcome, MergeOutcome::Merged);

        // The original repo's main now has feature.txt.
        assert!(
            repo.path().join("feature.txt").exists(),
            "merge must land file on original repo's main, not the worktree"
        );
    }

    #[test]
    fn merge_surfaces_conflict_when_both_branches_diverge() {
        let repo = make_test_repo();
        // Branch off, edit README on the branch.
        let (branch, wt_path) =
            generate_branch_and_path(repo.path(), AgentCli::Codex, "conflict test");
        create_worktree(repo.path(), &wt_path, &branch).expect("worktree add");
        std::fs::write(wt_path.join("README"), "from branch").unwrap();
        for cmd in [
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "test"],
            vec!["add", "."],
            vec!["commit", "-q", "-m", "branch edit"],
        ] {
            let _ = Command::new("git")
                .arg("-C")
                .arg(&wt_path)
                .args(&cmd)
                .status();
        }

        // Edit README on main (in the original repo).
        std::fs::write(repo.path().join("README"), "from main").unwrap();
        for cmd in [
            vec!["add", "."],
            vec!["commit", "-q", "-m", "main edit"],
        ] {
            let _ = Command::new("git")
                .arg("-C")
                .arg(repo.path())
                .args(&cmd)
                .status();
        }

        let outcome = perform_merge(repo.path(), &branch);
        match outcome {
            MergeOutcome::Conflict { files } => {
                assert!(files.iter().any(|f| f == "README"), "expected README in conflict list, got {files:?}");
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }
}
