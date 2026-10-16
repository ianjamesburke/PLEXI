//! Git worktree isolation for panes (`plexi pane new --worktree <branch>`).
//!
//! Creates a worktree through `wtp` (repo policy forbids raw `git worktree`)
//! and reclaims it on pane close when nothing changed (clean working tree and
//! HEAD still at the base commit). When the worktree is dirty or has moved
//! ahead/away from its base commit, close retains the worktree and reports the
//! retained path.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Host-tracked metadata for a worktree created via `plexi pane new --worktree`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedWorktree {
    /// Branch name passed to `--worktree <branch>`.
    pub branch: String,
    /// Worktree directory created by `wtp add`.
    pub path: PathBuf,
    /// Primary repository checkout path (`wtp cd`), used as cwd when running
    /// `wtp remove` so removal succeeds even after the pane's own directory is
    /// being torn down.
    pub repo_root: PathBuf,
    /// Commit SHA (`git rev-parse HEAD`) of the worktree immediately after creation.
    pub base_head: String,
    /// True when `wtp add -b <branch>` created a fresh local branch (so clean
    /// reclamation should also pass `--with-branch`); false when the branch
    /// already existed before the pane was spawned.
    pub created_branch: bool,
}

/// Outcome of closing a pane that owned a [`ManagedWorktree`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeCloseReport {
    /// `"reclaimed"` when the worktree was clean and unmoved and `wtp remove`
    /// succeeded; `"retained"` when the worktree was kept on disk.
    pub action: String,
    pub branch: String,
    pub path: PathBuf,
    /// Human-readable reason when retained (`"dirty"`, `"ahead"`, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Provision a git worktree for `branch` via `wtp`, anchored at `anchor_dir`.
///
/// Validates `branch`, resolves the main worktree root via `wtp cd`, checks
/// whether `refs/heads/<branch>` already exists, invokes `wtp add` (`-b` when
/// new), and records the initial `HEAD` commit so close-time reclamation can
/// tell whether any commits were made.
pub fn create_worktree(anchor_dir: &Path, branch: &str) -> Result<ManagedWorktree, String> {
    let branch = branch.trim();
    if branch.is_empty() {
        return Err("worktree branch name cannot be empty".to_string());
    }
    if branch.starts_with('-') {
        return Err(format!(
            "invalid worktree branch name '{branch}': must not start with '-'"
        ));
    }

    if !anchor_dir.exists() {
        return Err(format!(
            "cannot create worktree: base directory '{}' does not exist",
            anchor_dir.display()
        ));
    }

    // Resolve the main worktree root through `wtp cd` (with no args, `wtp cd`
    // prints the main worktree path). This also verifies `wtp` is installed and
    // `anchor_dir` is inside a git repository.
    let wtp_cd_out = Command::new("wtp")
        .arg("cd")
        .current_dir(anchor_dir)
        .output()
        .map_err(|e| format!("failed to run `wtp cd` in '{}': {e}", anchor_dir.display()))?;
    if !wtp_cd_out.status.success() {
        let stderr = String::from_utf8_lossy(&wtp_cd_out.stderr);
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            format!(
                "`wtp cd` failed in '{}' (not a git repository?)",
                anchor_dir.display()
            )
        } else {
            format!("`wtp cd` failed in '{}': {detail}", anchor_dir.display())
        });
    }
    let repo_root_str = String::from_utf8_lossy(&wtp_cd_out.stdout)
        .lines()
        .last()
        .unwrap_or("")
        .trim()
        .to_string();
    let repo_root = if repo_root_str.is_empty() {
        anchor_dir.to_path_buf()
    } else {
        PathBuf::from(repo_root_str)
    };

    // Check whether the local branch already exists so we know whether to pass
    // `-b` to `wtp add` and whether clean reclamation should delete the branch.
    let branch_exists = Command::new("git")
        .arg("-C")
        .arg(&repo_root)
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    let mut add_cmd = Command::new("wtp");
    add_cmd.arg("add");
    if !branch_exists {
        add_cmd.arg("-b").arg(branch);
    } else {
        add_cmd.arg(branch);
    }
    add_cmd.arg("--quiet").current_dir(anchor_dir);

    log::info!(
        "pane_worktree: creating worktree branch={branch:?} created_branch={} anchor={} repo_root={}",
        !branch_exists,
        anchor_dir.display(),
        repo_root.display()
    );

    let add_out = add_cmd
        .output()
        .map_err(|e| format!("failed to run `wtp add` for branch '{branch}': {e}"))?;
    if !add_out.status.success() {
        let stderr = String::from_utf8_lossy(&add_out.stderr);
        let stdout = String::from_utf8_lossy(&add_out.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim()
        } else {
            stdout.trim()
        };
        log::warn!("pane_worktree: `wtp add` failed for branch={branch:?}: {detail}");
        return Err(format!(
            "`wtp add` failed for branch '{branch}': {detail}"
        ));
    }

    // Resolve the created path via `wtp cd <branch>`, falling back to the last
    // non-empty stdout line of `wtp add --quiet`.
    let worktree_path = resolve_worktree_path(&repo_root, branch).or_else(|| {
        String::from_utf8_lossy(&add_out.stdout)
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(PathBuf::from)
    });

    let Some(path) = worktree_path.filter(|p| p.is_dir()) else {
        return Err(format!(
            "`wtp add` succeeded for '{branch}', but worktree path could not be resolved"
        ));
    };

    let base_head = git_head_sha(&path).ok_or_else(|| {
        format!(
            "worktree created at '{}', but `git rev-parse HEAD` failed",
            path.display()
        )
    })?;

    log::info!(
        "pane_worktree: created branch={branch:?} path={} base_head={base_head} created_branch={}",
        path.display(),
        !branch_exists
    );

    Ok(ManagedWorktree {
        branch: branch.to_string(),
        path,
        repo_root,
        base_head,
        created_branch: !branch_exists,
    })
}

/// Inspect `wt` on pane close: reclaim via `wtp remove` when clean and unmoved,
/// or retain on disk and report why when dirty or ahead/moved.
pub fn reclaim_or_retain_worktree(wt: &ManagedWorktree) -> WorktreeCloseReport {
    if !wt.path.exists() {
        log::info!(
            "pane_worktree: close branch={:?} path={} already absent; treating as reclaimed",
            wt.branch,
            wt.path.display()
        );
        return WorktreeCloseReport {
            action: "reclaimed".to_string(),
            branch: wt.branch.clone(),
            path: wt.path.clone(),
            reason: None,
        };
    }

    let mut retain_reasons: Vec<&'static str> = Vec::new();

    match is_worktree_dirty(&wt.path) {
        Ok(true) => retain_reasons.push("dirty"),
        Ok(false) => {}
        Err(err) => {
            log::warn!(
                "pane_worktree: could not check git status in {}: {err}; retaining",
                wt.path.display()
            );
            retain_reasons.push("status_unknown");
        }
    }

    match git_head_sha(&wt.path) {
        Some(current_head) => {
            if current_head != wt.base_head {
                retain_reasons.push("ahead");
            }
        }
        None => {
            log::warn!(
                "pane_worktree: could not read HEAD in {}; retaining",
                wt.path.display()
            );
            retain_reasons.push("head_unknown");
        }
    }

    if !retain_reasons.is_empty() {
        let reason = retain_reasons.join(",");
        log::info!(
            "pane_worktree: retaining worktree branch={:?} path={} reason={reason}",
            wt.branch,
            wt.path.display()
        );
        return WorktreeCloseReport {
            action: "retained".to_string(),
            branch: wt.branch.clone(),
            path: wt.path.clone(),
            reason: Some(reason),
        };
    }

    // Clean and unmoved: reclaim via `wtp remove`. Run from `repo_root` so
    // `wtp` never complains about running inside the worktree being removed.
    let mut rm_cmd = Command::new("wtp");
    rm_cmd.arg("remove");
    if wt.created_branch {
        rm_cmd.arg("--with-branch");
    }
    rm_cmd.arg(&wt.branch);
    if wt.repo_root.is_dir() {
        rm_cmd.current_dir(&wt.repo_root);
    }

    match rm_cmd.output() {
        Ok(out) if out.status.success() => {
            log::info!(
                "pane_worktree: reclaimed clean worktree branch={:?} path={} with_branch={}",
                wt.branch,
                wt.path.display(),
                wt.created_branch
            );
            WorktreeCloseReport {
                action: "reclaimed".to_string(),
                branch: wt.branch.clone(),
                path: wt.path.clone(),
                reason: None,
            }
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            log::warn!(
                "pane_worktree: `wtp remove` failed for branch={:?} path={}: {}; retaining",
                wt.branch,
                wt.path.display(),
                stderr.trim()
            );
            WorktreeCloseReport {
                action: "retained".to_string(),
                branch: wt.branch.clone(),
                path: wt.path.clone(),
                reason: Some("remove_failed".to_string()),
            }
        }
        Err(err) => {
            log::warn!(
                "pane_worktree: failed to execute `wtp remove` for branch={:?} path={}: {err}; retaining",
                wt.branch,
                wt.path.display()
            );
            WorktreeCloseReport {
                action: "retained".to_string(),
                branch: wt.branch.clone(),
                path: wt.path.clone(),
                reason: Some("remove_failed".to_string()),
            }
        }
    }
}

fn resolve_worktree_path(repo_root: &Path, branch: &str) -> Option<PathBuf> {
    let out = Command::new("wtp")
        .args(["cd", branch])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .last()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

fn git_head_sha(worktree_path: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(worktree_path)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

fn is_worktree_dirty(worktree_path: &Path) -> Result<bool, String> {
    let _ = Command::new("git")
        .arg("-C")
        .arg(worktree_path)
        .args(["update-index", "-q", "--refresh"])
        .output();
    let out = Command::new("git")
        .arg("-C")
        .arg(worktree_path)
        .args(["status", "--porcelain"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(!String::from_utf8_lossy(&out.stdout).trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_temp_git_repo() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo dir");

        let run = |args: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .status()
                .expect("run git");
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["init", "-b", "main"]);
        run(&["config", "user.email", "test@plexiapp.com"]);
        run(&["config", "user.name", "Plexi Test"]);
        std::fs::write(repo.join("README.md"), "hello\n").expect("write readme");
        run(&["add", "README.md"]);
        run(&["commit", "-m", "initial"]);

        (tmp, repo)
    }

    #[test]
    fn clean_unmoved_worktree_is_reclaimed_on_close() {
        let (_tmp, repo) = init_temp_git_repo();
        let wt = create_worktree(&repo, "feature/clean-test").expect("create worktree");
        assert!(wt.path.is_dir(), "worktree directory must exist");
        assert!(wt.created_branch);

        let report = reclaim_or_retain_worktree(&wt);
        assert_eq!(report.action, "reclaimed");
        assert_eq!(report.branch, "feature/clean-test");
        assert_eq!(report.path, wt.path);
        assert!(report.reason.is_none());
        assert!(
            !wt.path.exists(),
            "reclaimed worktree directory must be removed"
        );

        let branch_check = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                "refs/heads/feature/clean-test",
            ])
            .status()
            .expect("check branch");
        assert!(
            !branch_check.success(),
            "freshly created branch must also be removed when worktree is reclaimed"
        );
    }

    #[test]
    fn dirty_worktree_is_retained_and_reports_path() {
        let (_tmp, repo) = init_temp_git_repo();
        let wt = create_worktree(&repo, "feature/dirty-test").expect("create worktree");
        std::fs::write(wt.path.join("untracked.txt"), "work in progress\n")
            .expect("write untracked file");

        let report = reclaim_or_retain_worktree(&wt);
        assert_eq!(report.action, "retained");
        assert_eq!(report.branch, "feature/dirty-test");
        assert_eq!(report.path, wt.path);
        assert_eq!(report.reason.as_deref(), Some("dirty"));
        assert!(
            wt.path.is_dir(),
            "dirty worktree must remain on disk at reported path"
        );
    }

    #[test]
    fn ahead_worktree_is_retained_and_reports_path() {
        let (_tmp, repo) = init_temp_git_repo();
        let wt = create_worktree(&repo, "feature/ahead-test").expect("create worktree");

        let status = Command::new("git")
            .arg("-C")
            .arg(&wt.path)
            .args(["commit", "--allow-empty", "-m", "new work"])
            .status()
            .expect("commit in worktree");
        assert!(status.success());

        let report = reclaim_or_retain_worktree(&wt);
        assert_eq!(report.action, "retained");
        assert_eq!(report.branch, "feature/ahead-test");
        assert_eq!(report.path, wt.path);
        assert_eq!(report.reason.as_deref(), Some("ahead"));
        assert!(
            wt.path.is_dir(),
            "worktree with new commits must remain on disk at reported path"
        );
    }

    #[test]
    fn rejects_empty_or_flag_like_branch_names() {
        let (_tmp, repo) = init_temp_git_repo();
        assert!(create_worktree(&repo, "").is_err());
        assert!(create_worktree(&repo, "   ").is_err());
        assert!(create_worktree(&repo, "--help").is_err());
    }
}
