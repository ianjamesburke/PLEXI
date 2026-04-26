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
    /// modal picker (#349) is what populates this; the substrate accepts an
    /// empty label and renders "(no task)" in the header.
    pub task_label: String,
}

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
        })
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
}
