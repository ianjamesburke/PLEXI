//! Host-loop helpers run off the loop (stint 1632).
//!
//! `pgrep -P`, `wtp` and git are bounded by `bounded::Helper` (2 s, 60 s and
//! 15 s), but a bound only caps a stall: a host loop waiting on one still drops
//! input and starves PTY, IPC and screenshots. So the loop never waits. A
//! request that needs a helper starts a job on its own thread and returns
//! without answering; the job's result arrives through a `UiMailbox`, and
//! `App::logic` drains it and finishes the request against the state current
//! at that moment.

use std::path::PathBuf;
use std::sync::Arc;

use super::ui_mailbox::{MailboxReceiver, UiMailbox, UiWake};
use super::PlexiApp;
use crate::host::pane_liveness::Pgrep;
use crate::host::worktree::{self, ManagedWorktree, WorktreeCloseReport};
use crate::protocol::HostRequest;

/// The external programs jobs run, with their limits. Production uses
/// [`HelperTools::system`]; tests swap in hung fakes.
pub(crate) struct HelperTools {
    pub(crate) worktree: worktree::Tools,
    pub(crate) pgrep: Pgrep,
}

impl HelperTools {
    fn system() -> Self {
        Self {
            worktree: worktree::Tools::system(),
            pgrep: Pgrep::system(),
        }
    }
}

/// A finished job, drained by [`PlexiApp::service_host_helpers`].
pub(crate) enum Finished {
    /// `pgrep -P` answered for a `get_pane_state` request.
    PaneChildren {
        pane_id: u64,
        response_file: String,
        stale_after_secs: Option<u64>,
        child_pids: Vec<u32>,
    },
    /// A worktree a `spawn_pane` or `agent_call` request asked for.
    WorktreeCreated {
        request: Box<HostRequest>,
        worktree: Result<ManagedWorktree, String>,
    },
    /// A worktree was reclaimed or retained. `reply` is the `close_pane`
    /// response file still owed an answer.
    WorktreeClosed {
        pane_id: Option<u64>,
        report: WorktreeCloseReport,
        reply: Option<String>,
    },
}

/// The worktree a `spawn_pane` or `agent_call` request is finished with.
/// `None` until a job has answered; the request handler takes it when it
/// owns the result, and whatever it leaves behind is reclaimed.
pub(crate) type Provisioned = Option<Result<ManagedWorktree, String>>;

pub(crate) struct HostHelpers {
    tools: Arc<HelperTools>,
    done: UiMailbox<Finished>,
    finished: MailboxReceiver<Finished>,
}

impl HostHelpers {
    pub(crate) fn new(wake: Arc<dyn UiWake>) -> Self {
        let (done, finished) = UiMailbox::channel(wake, "host_helpers");
        Self {
            tools: Arc::new(HelperTools::system()),
            done,
            finished,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_tools(&mut self, tools: HelperTools) {
        self.tools = Arc::new(tools);
    }
}

impl PlexiApp {
    /// Run `job` on its own thread; its result is drained by
    /// [`Self::service_host_helpers`]. When no thread can be started,
    /// `unstarted` builds the result now and it is finished in place, so
    /// every request is still answered.
    fn start_host_helper(
        &mut self,
        what: &'static str,
        job: impl FnOnce(&HelperTools) -> Finished + Send + 'static,
        unstarted: impl FnOnce(std::io::Error) -> Finished,
    ) {
        log::info!("host_helper: dispatch job={what}");
        let tools = Arc::clone(&self.host_helpers.tools);
        let done = self.host_helpers.done.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("host-helper-{what}"))
            .spawn(move || {
                if done.send(job(&tools)).is_err() {
                    log::warn!("host_helper: job={what} finished after the host loop went away");
                }
            });
        if let Err(error) = spawned {
            log::error!("host_helper: could not start a thread for job={what}: {error}");
            self.finish_host_helper(unstarted(error));
        }
    }

    /// Drain finished jobs. Lives in `App::logic`, so an occluded host still
    /// answers. Returns true when anything was finished.
    pub(crate) fn service_host_helpers(&mut self) -> bool {
        let mut any = false;
        while let Ok(finished) = self.host_helpers.finished.try_recv() {
            self.finish_host_helper(finished);
            any = true;
        }
        any
    }

    fn finish_host_helper(&mut self, finished: Finished) {
        match finished {
            Finished::PaneChildren {
                pane_id,
                response_file,
                stale_after_secs,
                child_pids,
            } => {
                log::info!(
                    "host_helper: finished job=pgrep pane_id={pane_id} child_count={}",
                    child_pids.len()
                );
                self.reply_pane_state(pane_id, &response_file, stale_after_secs, Some(&child_pids));
            }
            Finished::WorktreeCreated { request, worktree } => {
                log::info!(
                    "host_helper: finished job=worktree_create ok={}",
                    worktree.is_ok()
                );
                let mut provisioned = Some(worktree);
                match request.as_ref() {
                    HostRequest::SpawnPane { .. } => {
                        self.spawn_pane_request(&request, &mut provisioned);
                    }
                    HostRequest::AgentCall { .. } => {
                        self.agent_call_request(&request, &mut provisioned);
                    }
                    _ => log::error!("host_helper: worktree created for a request that takes none"),
                }
                if let Some(Ok(wt)) = provisioned {
                    log::info!(
                        "pane_worktree: request ended before using branch='{}'; reclaiming",
                        wt.branch
                    );
                    self.close_worktree_off_loop(None, wt, None);
                }
            }
            Finished::WorktreeClosed {
                pane_id,
                report,
                reply,
            } => {
                log::info!(
                    "host_helper: finished job=worktree_close pane_id={pane_id:?} branch='{}' path='{}' action={} reason={}",
                    report.branch,
                    report.path.display(),
                    report.action,
                    report.reason.as_deref().unwrap_or("clean")
                );
                if let Some(rf) = reply {
                    crate::rpc::write_json_response(
                        &rf,
                        serde_json::json!({
                            "ok": true,
                            "pane_id": pane_id,
                            "worktree": report,
                        }),
                    );
                }
                if let Some(pane_id) = pane_id {
                    self.last_worktree_close_reports.insert(pane_id, report);
                }
            }
        }
    }

    /// List `shell_pid`'s children off the loop, then answer `get_pane_state`.
    pub(crate) fn pane_children_off_loop(
        &mut self,
        pane_id: u64,
        shell_pid: u32,
        response_file: &str,
        stale_after_secs: Option<u64>,
    ) {
        let rf = response_file.to_string();
        let unstarted_rf = rf.clone();
        self.start_host_helper(
            "pgrep",
            move |tools| Finished::PaneChildren {
                pane_id,
                response_file: rf,
                stale_after_secs,
                child_pids: tools.pgrep.child_pids(shell_pid),
            },
            move |_| Finished::PaneChildren {
                pane_id,
                response_file: unstarted_rf,
                stale_after_secs,
                child_pids: Vec::new(),
            },
        );
    }

    /// Create `branch`'s worktree off the loop, then finish `request` with it.
    pub(crate) fn create_worktree_off_loop(
        &mut self,
        request: &HostRequest,
        anchor_dir: PathBuf,
        branch: String,
    ) {
        let job_request = Box::new(request.clone());
        let unstarted_request = Box::new(request.clone());
        self.start_host_helper(
            "worktree_create",
            move |tools| Finished::WorktreeCreated {
                request: job_request,
                worktree: worktree::create_worktree(&anchor_dir, &branch, &tools.worktree),
            },
            move |error| Finished::WorktreeCreated {
                request: unstarted_request,
                worktree: Err(format!("could not start the worktree helper: {error}")),
            },
        );
    }

    /// Reclaim or retain `wt` off the loop. `reply` is a `close_pane`
    /// response file answered once the disposition is known.
    pub(crate) fn close_worktree_off_loop(
        &mut self,
        pane_id: Option<u64>,
        wt: ManagedWorktree,
        reply: Option<String>,
    ) {
        let unstarted = WorktreeCloseReport {
            action: "retained".to_string(),
            branch: wt.branch.clone(),
            path: wt.path.clone(),
            reason: Some("helper_not_started".to_string()),
        };
        let unstarted_reply = reply.clone();
        self.start_host_helper(
            "worktree_close",
            move |tools| Finished::WorktreeClosed {
                pane_id,
                report: worktree::reclaim_or_retain_worktree(&wt, &tools.worktree),
                reply,
            },
            move |_| Finished::WorktreeClosed {
                pane_id,
                report: unstarted,
                reply: unstarted_reply,
            },
        );
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::testing::HostHarness;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::time::{Duration, Instant};

    /// Every fake helper hangs; each is stopped at this limit, so a logic
    /// pass that waited on one takes at least this long.
    const HANG_LIMIT: Duration = Duration::from_secs(4);

    fn fake(dir: &Path, name: &str, script: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn git_repo(dir: &Path) -> PathBuf {
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            &["init", "-q", "-b", "alpha"][..],
            &["config", "user.email", "test@plexiapp.com"],
            &["config", "user.name", "Plexi Test"],
            &["commit", "-q", "--allow-empty", "-m", "initial"],
        ] {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}: {out:?}");
        }
        repo
    }

    fn spawn_request(repo: &Path, branch: &str, response_file: &str) -> HostRequest {
        HostRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("split_h".to_string()),
            args: vec!["true".to_string()],
            from_pane_id: None,
            request_id: None,
            response_file: Some(response_file.to_string()),
            ephemeral: false,
            cwd: Some(repo.to_string_lossy().into_owned()),
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            context_name: None,
            name: None,
            agent_cmd: None,
            boot_timeout_secs: None,
            worktree: Some(branch.to_string()),
            result_schema: None,
            peer_ancestry: None,
        }
    }

    /// One hidden pass (the `App::logic` path an occluded host runs) after
    /// `request`: it must finish inside the frame budget with the reply
    /// still owed, then later passes deliver the reply.
    fn reply_without_waiting(
        h: &mut HostHarness,
        request: HostRequest,
        response_file: &str,
    ) -> serde_json::Value {
        h.inject_ipc(request);
        let started = Instant::now();
        h.run_hidden_frames(1);
        let tick = started.elapsed();
        assert!(
            tick < HANG_LIMIT,
            "the logic pass waited {tick:?} on a hung helper"
        );
        assert!(
            !Path::new(response_file).exists(),
            "the reply was written before the hung helper finished"
        );
        let mut passes_while_pending = 0;
        let deadline = Instant::now() + crate::testing::load_aware_timeout(HANG_LIMIT * 10);
        while !Path::new(response_file).exists() {
            assert!(Instant::now() < deadline, "no reply at {response_file}");
            h.run_hidden_frames(1);
            passes_while_pending += 1;
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(passes_while_pending > 1, "the loop did not keep ticking");
        serde_json::from_str(&std::fs::read_to_string(response_file).unwrap()).unwrap()
    }

    #[test]
    fn a_hung_pgrep_never_holds_the_host_loop() {
        let tmp = tempfile::tempdir().unwrap();
        let mut h = HostHarness::new();
        let pane_id = h.add_focused_terminal();
        h.app.host_helpers.set_tools(HelperTools {
            worktree: worktree::Tools::system(),
            pgrep: Pgrep::at(fake(tmp.path(), "pgrep", "sleep 30"), HANG_LIMIT),
        });
        let rf = tmp.path().join("state.json").to_string_lossy().into_owned();
        let reply = reply_without_waiting(
            &mut h,
            HostRequest::GetPaneState {
                pane_id,
                response_file: rf.clone(),
                stale_after_secs: None,
            },
            &rf,
        );
        assert_eq!(reply["pane_id"], pane_id, "{reply}");
        assert_eq!(reply["observed_state"]["process_alive"], true, "{reply}");
        assert_eq!(reply["observed_state"]["child_pids"], serde_json::json!([]));
    }

    #[test]
    fn a_hung_wtp_never_holds_the_host_loop_on_spawn() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = git_repo(tmp.path());
        let mut h = HostHarness::new();
        h.app.set_context_root(repo.clone(), None);
        h.add_test_pane();
        h.app.host_helpers.set_tools(HelperTools {
            worktree: worktree::Tools::with_wtp_program(
                fake(tmp.path(), "wtp", "sleep 30"),
                HANG_LIMIT,
            ),
            pgrep: Pgrep::system(),
        });
        let rf = tmp.path().join("spawn.json").to_string_lossy().into_owned();
        let before = h.state().open_panes.len();
        let reply = reply_without_waiting(&mut h, spawn_request(&repo, "feature/hung", &rf), &rf);
        let error = reply["error"].as_str().expect("error reply");
        assert!(error.contains("wtp cd"), "{error}");
        assert_eq!(
            h.state().open_panes.len(),
            before,
            "no pane without its worktree"
        );
    }

    #[test]
    fn a_hung_wtp_never_holds_the_host_loop_on_close() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = git_repo(tmp.path());
        let mut h = HostHarness::new();
        h.app.set_context_root(repo.clone(), None);
        h.add_test_pane();
        let spawn_rf = tmp.path().join("spawn.json").to_string_lossy().into_owned();
        h.inject_ipc(spawn_request(&repo, "feature/close-hung", &spawn_rf));
        let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(60));
        while !Path::new(&spawn_rf).exists() {
            assert!(Instant::now() < deadline, "spawn never answered");
            h.run_hidden_frames(1);
            std::thread::sleep(Duration::from_millis(20));
        }
        let spawned: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&spawn_rf).unwrap()).unwrap();
        let pane_id = spawned["pane_id"]
            .as_u64()
            .unwrap_or_else(|| panic!("{spawned}"));
        let path = PathBuf::from(spawned["worktree"].as_str().expect("worktree path"));
        assert!(path.is_dir());

        h.app.host_helpers.set_tools(HelperTools {
            worktree: worktree::Tools::with_wtp_program(
                fake(tmp.path(), "wtp", "sleep 30"),
                HANG_LIMIT,
            ),
            pgrep: Pgrep::system(),
        });
        let rf = tmp.path().join("close.json").to_string_lossy().into_owned();
        let reply = reply_without_waiting(
            &mut h,
            HostRequest::ClosePane {
                pane_id,
                response_file: Some(rf.clone()),
            },
            &rf,
        );
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["worktree"]["action"], "retained", "{reply}");
        assert_eq!(reply["worktree"]["reason"], "remove_failed", "{reply}");
        assert!(path.is_dir(), "a worktree whose removal timed out stays");
    }
}
