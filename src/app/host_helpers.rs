//! Host-loop helpers run off the loop (stint 1632).
//!
//! `pgrep -P`, `wtp` and git are bounded by `bounded::Helper` (2 s, 60 s and
//! 15 s), but a bound only caps a stall: a host loop waiting on one still drops
//! input and starves PTY, IPC and screenshots. So the loop never waits. A
//! request that needs a helper starts a job on its own thread and returns
//! without answering; the job's result arrives through a `UiMailbox`, and
//! `App::logic` drains it and finishes the request. A request resumed after a
//! worktree is created carries what it was admitted with (caller authority,
//! destination by stable id), never re-deriving them from state that changed
//! meanwhile.
//!
//! Every started job is owed: `HostHelpers` records it until its result is
//! finished. On quit, [`PlexiApp::settle_host_helpers_for_exit`] stops the
//! queries, lets a `wtp add` or `wtp remove` already running finish, and
//! answers every owed reply, so no request is stranded and no worktree is
//! left half made or half removed.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::agent_tools::AgentAdmission;
use super::launch_spec::TerminalAdmission;
use super::ui_mailbox::{MailboxReceiver, UiMailbox, UiWake};
use super::PlexiApp;
use crate::host::pane_liveness::Pgrep;
use crate::host::worktree::{self, ManagedWorktree, WorktreeCloseReport};
use crate::protocol::HostRequest;

/// How long the quit waits past the longest mutating `wtp` call for owed
/// jobs to report.
const EXIT_SETTLE_MARGIN: Duration = Duration::from_secs(5);

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

/// A request waiting on a worktree, with what it was admitted with.
#[derive(Clone)]
pub(crate) enum Resume {
    SpawnPane {
        request: Box<HostRequest>,
        admission: TerminalAdmission,
    },
    AgentCall {
        request: Box<HostRequest>,
        admission: AgentAdmission,
    },
}

impl Resume {
    /// The reply the request owes, and the error body its caller reads.
    fn reply(&self) -> Option<(String, ReplyShape)> {
        match self {
            Resume::SpawnPane { request, .. } => match request.as_ref() {
                HostRequest::SpawnPane { response_file, .. } => {
                    response_file.clone().map(|rf| (rf, ReplyShape::Error))
                }
                _ => None,
            },
            Resume::AgentCall { request, .. } => match request.as_ref() {
                HostRequest::AgentCall { response_file, .. } => {
                    Some((response_file.clone(), ReplyShape::AgentCall))
                }
                _ => None,
            },
        }
    }
}

/// A resumed request's admission and the worktree job's result. The handler
/// takes `worktree` when it owns it; whatever it leaves behind is reclaimed.
pub(crate) struct Resumed<A> {
    pub(crate) admission: A,
    pub(crate) worktree: Option<Result<ManagedWorktree, String>>,
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
        resume: Resume,
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

/// How a request reports an error to its caller.
#[derive(Clone, Copy)]
enum ReplyShape {
    /// `{"error": ...}`
    Error,
    /// `{"ok": false, "error": ...}`
    AgentCall,
}

impl ReplyShape {
    fn write(self, response_file: &str, error: &str) {
        let body = match self {
            ReplyShape::Error => serde_json::json!({ "error": error }),
            ReplyShape::AgentCall => serde_json::json!({ "ok": false, "error": error }),
        };
        crate::rpc::write_json_response(response_file, body);
    }
}

/// A started job not yet finished: what it owes if the host exits first.
struct Owed {
    what: &'static str,
    reply: Option<(String, ReplyShape)>,
    /// A worktree the job may leave on disk.
    worktree: Option<PathBuf>,
}

pub(crate) struct HostHelpers {
    tools: Arc<HelperTools>,
    done: UiMailbox<(u64, Finished)>,
    finished: MailboxReceiver<(u64, Finished)>,
    owed: HashMap<u64, Owed>,
    next_job: u64,
    /// Set by `settle_host_helpers_for_exit`: results finish without
    /// resuming any request or starting another job.
    exiting: bool,
    #[cfg(test)]
    refuse_threads: bool,
}

impl HostHelpers {
    pub(crate) fn new(wake: Arc<dyn UiWake>) -> Self {
        let (done, finished) = UiMailbox::channel(wake, "host_helpers");
        Self {
            tools: Arc::new(HelperTools::system()),
            done,
            finished,
            owed: HashMap::new(),
            next_job: 0,
            exiting: false,
            #[cfg(test)]
            refuse_threads: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_tools(&mut self, tools: HelperTools) {
        self.tools = Arc::new(tools);
    }

    /// Make every thread start fail, as when the process is out of threads.
    #[cfg(test)]
    pub(crate) fn refuse_threads(&mut self) {
        self.refuse_threads = true;
    }

    #[cfg(test)]
    pub(crate) fn owed_count(&self) -> usize {
        self.owed.len()
    }

    fn spawn(
        &self,
        what: &'static str,
        run: impl FnOnce() + Send + 'static,
    ) -> std::io::Result<std::thread::JoinHandle<()>> {
        #[cfg(test)]
        if self.refuse_threads {
            return Err(std::io::Error::other("thread start refused by the test"));
        }
        std::thread::Builder::new()
            .name(format!("host-helper-{what}"))
            .spawn(run)
    }
}

impl PlexiApp {
    /// Run `job` on its own thread and record what it owes; its result is
    /// drained by [`Self::service_host_helpers`]. When no thread can be
    /// started, `unstarted` builds the result now and it is finished in
    /// place, so every request is still answered.
    fn start_host_helper(
        &mut self,
        owed: Owed,
        job: impl FnOnce(&HelperTools) -> Finished + Send + 'static,
        unstarted: impl FnOnce(std::io::Error) -> Finished,
    ) {
        let what = owed.what;
        let id = self.host_helpers.next_job;
        self.host_helpers.next_job += 1;
        log::info!("host_helper: dispatch job={what} id={id}");
        let tools = Arc::clone(&self.host_helpers.tools);
        let done = self.host_helpers.done.clone();
        let spawned = self.host_helpers.spawn(what, move || {
            if done.send((id, job(&tools))).is_err() {
                log::warn!(
                    "host_helper: job={what} id={id} finished after the host loop went away"
                );
            }
        });
        match spawned {
            Ok(_) => {
                self.host_helpers.owed.insert(id, owed);
            }
            Err(error) => {
                log::error!(
                    "host_helper: could not start a thread for job={what} id={id}: {error}"
                );
                self.finish_host_helper(id, unstarted(error));
            }
        }
    }

    /// Drain finished jobs. Lives in `App::logic`, so an occluded host still
    /// answers. Returns true when anything was finished.
    pub(crate) fn service_host_helpers(&mut self) -> bool {
        let mut any = false;
        while let Ok((id, finished)) = self.host_helpers.finished.try_recv() {
            self.host_helpers.owed.remove(&id);
            self.finish_host_helper(id, finished);
            any = true;
        }
        any
    }

    /// Quit path: stop helper queries, wait (bounded) for owed jobs, and
    /// answer every reply still owed. A running `wtp add` or `wtp remove`
    /// finishes or hits its limit inside the wait, so its worktree ends
    /// whole or untouched; a created worktree no request will use is kept
    /// on disk and its path reported.
    pub(crate) fn settle_host_helpers_for_exit(&mut self) {
        self.host_helpers.exiting = true;
        let tools = Arc::clone(&self.host_helpers.tools);
        tools.worktree.begin_exit();
        tools.pgrep.begin_exit();
        // Results already queued but not yet drained are owed too.
        self.service_host_helpers();
        if self.host_helpers.owed.is_empty() {
            return;
        }
        let limit = tools.worktree.wtp_limit() + EXIT_SETTLE_MARGIN;
        log::info!(
            "host_helper: exit settling owed_jobs={} limit_ms={}",
            self.host_helpers.owed.len(),
            limit.as_millis()
        );
        let deadline = Instant::now() + limit;
        while !self.host_helpers.owed.is_empty() {
            let left = deadline.saturating_duration_since(Instant::now());
            let Ok((id, finished)) = self.host_helpers.finished.recv_timeout(left) else {
                break;
            };
            self.host_helpers.owed.remove(&id);
            self.finish_host_helper(id, finished);
        }
        for (id, owed) in std::mem::take(&mut self.host_helpers.owed) {
            log::error!(
                "host_helper: job={} id={id} still running at exit; worktree={:?}",
                owed.what,
                owed.worktree
            );
            if let Some((rf, shape)) = owed.reply {
                shape.write(
                    &rf,
                    &format!("the host exited before {} finished", owed.what),
                );
            }
        }
        log::info!("host_helper: exit settled");
    }

    fn finish_host_helper(&mut self, id: u64, finished: Finished) {
        match finished {
            Finished::PaneChildren {
                pane_id,
                response_file,
                stale_after_secs,
                child_pids,
            } => {
                log::info!(
                    "host_helper: finished job=pgrep id={id} pane_id={pane_id} child_count={}",
                    child_pids.len()
                );
                self.reply_pane_state(pane_id, &response_file, stale_after_secs, Some(&child_pids));
            }
            Finished::WorktreeCreated { resume, worktree } => {
                log::info!(
                    "host_helper: finished job=worktree_create id={id} ok={}",
                    worktree.is_ok()
                );
                if self.host_helpers.exiting {
                    self.refuse_resume_at_exit(resume, worktree);
                    return;
                }
                let unused = match resume {
                    Resume::SpawnPane { request, admission } => {
                        let mut resumed = Some(Resumed {
                            admission,
                            worktree: Some(worktree),
                        });
                        self.spawn_pane_request(&request, &mut resumed);
                        resumed.and_then(|r| r.worktree)
                    }
                    Resume::AgentCall { request, admission } => {
                        let mut resumed = Some(Resumed {
                            admission,
                            worktree: Some(worktree),
                        });
                        self.agent_call_request(&request, &mut resumed);
                        resumed.and_then(|r| r.worktree)
                    }
                };
                if let Some(Ok(wt)) = unused {
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
                    "host_helper: finished job=worktree_close id={id} pane_id={pane_id:?} branch='{}' path='{}' action={} reason={}",
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

    /// A worktree finished while the host exits: no pane will use it, and a
    /// `wtp remove` now could be cut off. Keep it, report its path.
    fn refuse_resume_at_exit(&mut self, resume: Resume, worktree: Result<ManagedWorktree, String>) {
        let error = match worktree {
            Ok(wt) => {
                log::warn!(
                    "pane_worktree: host exiting; retaining unused worktree branch='{}' path='{}'",
                    wt.branch,
                    wt.path.display()
                );
                format!(
                    "the host is exiting; worktree for branch '{}' was kept at '{}'",
                    wt.branch,
                    wt.path.display()
                )
            }
            Err(error) => error,
        };
        if let Some((rf, shape)) = resume.reply() {
            shape.write(&rf, &error);
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
            Owed {
                what: "pgrep",
                reply: Some((rf.clone(), ReplyShape::Error)),
                worktree: None,
            },
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

    /// Create `branch`'s worktree off the loop, then resume the request.
    pub(crate) fn create_worktree_off_loop(
        &mut self,
        resume: Resume,
        anchor_dir: PathBuf,
        branch: String,
    ) {
        if self.host_helpers.exiting {
            self.refuse_resume_at_exit(
                resume,
                Err("the host is exiting; no worktree was created".to_string()),
            );
            return;
        }
        let owed = Owed {
            what: "worktree_create",
            reply: resume.reply(),
            worktree: None,
        };
        let unstarted_resume = resume.clone();
        self.start_host_helper(
            owed,
            move |tools| Finished::WorktreeCreated {
                resume,
                worktree: worktree::create_worktree(&anchor_dir, &branch, &tools.worktree),
            },
            move |error| Finished::WorktreeCreated {
                resume: unstarted_resume,
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
        let owed = Owed {
            what: "worktree_close",
            reply: reply.clone().map(|rf| (rf, ReplyShape::Error)),
            worktree: Some(wt.path.clone()),
        };
        let unstarted = WorktreeCloseReport {
            action: "retained".to_string(),
            branch: wt.branch.clone(),
            path: wt.path.clone(),
            reason: Some("helper_not_started".to_string()),
        };
        let unstarted_reply = reply.clone();
        self.start_host_helper(
            owed,
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
pub(crate) mod tests {
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

    pub(crate) fn git_repo(dir: &Path) -> PathBuf {
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
        spawn_request_from(repo, branch, response_file, None)
    }

    fn spawn_request_from(
        repo: &Path,
        branch: &str,
        response_file: &str,
        from_pane_id: Option<u64>,
    ) -> HostRequest {
        HostRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("split_h".to_string()),
            args: vec!["true".to_string()],
            from_pane_id,
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

    /// Hidden passes (the `App::logic` path an occluded host runs) after
    /// `request`: every pass, the first and each one while the helper hangs,
    /// finishes under the helper's limit; the first leaves the reply owed,
    /// and a later one delivers it.
    fn reply_without_waiting(
        h: &mut HostHarness,
        request: HostRequest,
        response_file: &str,
    ) -> serde_json::Value {
        h.inject_ipc(request);
        let timed_pass = |h: &mut HostHarness| {
            let started = Instant::now();
            h.run_hidden_frames(1);
            let tick = started.elapsed();
            assert!(
                tick < HANG_LIMIT,
                "a logic pass waited {tick:?} on a hung helper"
            );
        };
        timed_pass(h);
        assert!(
            !Path::new(response_file).exists(),
            "the reply was written before the hung helper finished"
        );
        let mut passes_while_pending = 0;
        let deadline = Instant::now() + crate::testing::load_aware_timeout(HANG_LIMIT * 10);
        while !Path::new(response_file).exists() {
            assert!(Instant::now() < deadline, "no reply at {response_file}");
            timed_pass(h);
            passes_while_pending += 1;
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(passes_while_pending > 1, "the loop did not keep ticking");
        read_reply(response_file)
    }

    fn read_reply(response_file: &str) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(response_file).unwrap()).unwrap()
    }

    fn pump_until(h: &mut HostHarness, what: &str, mut done: impl FnMut(&HostHarness) -> bool) {
        let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(60));
        while !done(h) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            h.run_hidden_frames(1);
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Spawn a terminal with a worktree through the real (or shim) wtp and
    /// return its pane id and worktree path.
    fn spawn_worktree_pane(
        h: &mut HostHarness,
        tmp: &Path,
        repo: &Path,
        branch: &str,
    ) -> (u64, PathBuf) {
        let rf = tmp.join(format!("spawn-{}.json", branch.replace('/', "-")));
        let rf = rf.to_string_lossy().into_owned();
        h.inject_ipc(spawn_request(repo, branch, &rf));
        pump_until(h, "the worktree spawn", |_| Path::new(&rf).exists());
        let spawned = read_reply(&rf);
        let pane_id = spawned["pane_id"]
            .as_u64()
            .unwrap_or_else(|| panic!("{spawned}"));
        let path = PathBuf::from(spawned["worktree"].as_str().expect("worktree path"));
        assert!(path.is_dir());
        (pane_id, path)
    }

    /// A `wtp` that waits `delay_secs` in `add`, then behaves as the shim.
    pub(crate) fn slow_add_wtp(dir: &Path, delay_secs: u32) -> PathBuf {
        fake(
            dir,
            "wtp",
            &format!(
                "if [ \"$1\" = add ]; then sleep {delay_secs}; fi\nexec python3 '{}' \"$@\"",
                worktree::test_wtp_shim_script().display()
            ),
        )
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
        let (pane_id, path) = spawn_worktree_pane(&mut h, tmp.path(), &repo, "feature/close-hung");

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

    /// Quit with a pgrep, a worktree create and a worktree close all owed:
    /// queries stop at once, every reply is answered, and the worktree being
    /// closed ends retained, never half removed.
    #[test]
    fn exit_answers_every_owed_job_and_orphans_no_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = git_repo(tmp.path());
        let mut h = HostHarness::new();
        h.app.set_context_root(repo.clone(), None);
        let terminal = h.add_focused_terminal();
        let (wt_pane, wt_path) = spawn_worktree_pane(&mut h, tmp.path(), &repo, "feature/exit");

        // Long limits: only the exit, not a limit, can end these quickly.
        let slow = Duration::from_secs(60);
        h.app.host_helpers.set_tools(HelperTools {
            worktree: worktree::Tools::with_wtp_program(
                fake(tmp.path(), "wtp", "sleep 120"),
                HANG_LIMIT,
            ),
            pgrep: Pgrep::at(fake(tmp.path(), "pgrep", "sleep 120"), slow),
        });
        let state_rf = tmp.path().join("state.json").to_string_lossy().into_owned();
        let spawn_rf = tmp
            .path()
            .join("spawn2.json")
            .to_string_lossy()
            .into_owned();
        let close_rf = tmp.path().join("close.json").to_string_lossy().into_owned();
        h.inject_ipc(HostRequest::GetPaneState {
            pane_id: terminal,
            response_file: state_rf.clone(),
            stale_after_secs: None,
        });
        h.inject_ipc(spawn_request(&repo, "feature/exit-pending", &spawn_rf));
        h.inject_ipc(HostRequest::ClosePane {
            pane_id: wt_pane,
            response_file: Some(close_rf.clone()),
        });
        h.run_hidden_frames(1);
        assert_eq!(h.app.host_helpers.owed_count(), 3);

        let started = Instant::now();
        h.app.settle_host_helpers_for_exit();
        let settled = started.elapsed();

        assert_eq!(h.app.host_helpers.owed_count(), 0);
        // The pgrep and `wtp cd` queries were stopped at exit; the close
        // waited at most for a `wtp remove` already running (`HANG_LIMIT`).
        assert!(
            settled < HANG_LIMIT + Duration::from_secs(10),
            "exit waited {settled:?}"
        );
        let state = read_reply(&state_rf);
        assert_eq!(state["pane_id"], terminal, "{state}");
        assert_eq!(state["observed_state"]["child_pids"], serde_json::json!([]));
        let spawned = read_reply(&spawn_rf);
        assert!(spawned["error"].is_string(), "{spawned}");
        let closed = read_reply(&close_rf);
        assert_eq!(closed["worktree"]["action"], "retained", "{closed}");
        // Which step the exit caught decides the reason (a stopped query,
        // a remove never started, or one that hit its limit); the worktree
        // is kept whole either way.
        assert!(closed["worktree"]["reason"].is_string(), "{closed}");
        assert!(wt_path.is_dir(), "a retained worktree stays whole on disk");
        assert!(
            !wt_path.with_file_name("exit-pending").exists(),
            "no worktree was started after the exit began"
        );
    }

    /// The caller's pane closes while the worktree is created: the spawn is
    /// refused (never placed beside whatever is focused now) and the
    /// unused worktree is reclaimed.
    #[test]
    fn a_caller_pane_closed_during_provisioning_refuses_the_spawn() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = git_repo(tmp.path());
        let mut h = HostHarness::new();
        h.app.set_context_root(repo.clone(), None);
        h.add_test_pane();
        let caller = h.add_test_pane();
        h.app.host_helpers.set_tools(HelperTools {
            worktree: worktree::Tools::with_wtp_program(
                slow_add_wtp(tmp.path(), 1),
                Duration::from_secs(60),
            ),
            pgrep: Pgrep::system(),
        });
        let rf = tmp.path().join("spawn.json").to_string_lossy().into_owned();
        h.inject_ipc(spawn_request_from(&repo, "feature/gone", &rf, Some(caller)));
        h.run_hidden_frames(1);
        assert_eq!(h.app.host_helpers.owed_count(), 1);
        let panes_before_close = h.state().open_panes.len();
        h.app.close_pane_by_id(caller);
        pump_until(&mut h, "the spawn reply", |_| Path::new(&rf).exists());
        let reply = read_reply(&rf);
        let error = reply["error"].as_str().unwrap_or_else(|| panic!("{reply}"));
        assert!(error.contains("destination is gone"), "{error}");
        assert_eq!(h.state().open_panes.len(), panes_before_close - 1);
        pump_until(&mut h, "the reclaim", |h| {
            h.app.host_helpers.owed_count() == 0
        });
        assert!(
            !repo.join("worktrees/feature/gone").exists(),
            "the unused worktree was reclaimed"
        );
    }

    /// No thread can start: every request is still answered at once.
    #[test]
    fn a_failed_thread_start_still_answers_every_request() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = git_repo(tmp.path());
        let mut h = HostHarness::new();
        h.app.set_context_root(repo.clone(), None);
        let terminal = h.add_focused_terminal();
        let (wt_pane, wt_path) =
            spawn_worktree_pane(&mut h, tmp.path(), &repo, "feature/no-thread");
        h.app.host_helpers.refuse_threads();

        let state_rf = tmp.path().join("state.json").to_string_lossy().into_owned();
        let spawn_rf = tmp
            .path()
            .join("spawn2.json")
            .to_string_lossy()
            .into_owned();
        let close_rf = tmp.path().join("close.json").to_string_lossy().into_owned();
        h.inject_ipc(HostRequest::GetPaneState {
            pane_id: terminal,
            response_file: state_rf.clone(),
            stale_after_secs: None,
        });
        h.inject_ipc(spawn_request(&repo, "feature/never", &spawn_rf));
        h.inject_ipc(HostRequest::ClosePane {
            pane_id: wt_pane,
            response_file: Some(close_rf.clone()),
        });
        h.run_hidden_frames(1);

        assert_eq!(h.app.host_helpers.owed_count(), 0);
        let state = read_reply(&state_rf);
        assert_eq!(state["observed_state"]["child_pids"], serde_json::json!([]));
        let spawned = read_reply(&spawn_rf);
        assert!(
            spawned["error"]
                .as_str()
                .is_some_and(|e| e.contains("could not start the worktree helper")),
            "{spawned}"
        );
        let closed = read_reply(&close_rf);
        assert_eq!(closed["worktree"]["action"], "retained", "{closed}");
        assert_eq!(
            closed["worktree"]["reason"], "helper_not_started",
            "{closed}"
        );
        assert!(wt_path.is_dir());
    }

    /// A `wtp remove` already running when the host exits is not cut off:
    /// the exit waits for it, so the worktree is fully removed.
    #[test]
    fn exit_lets_a_running_remove_finish() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = git_repo(tmp.path());
        let mut h = HostHarness::new();
        h.app.set_context_root(repo.clone(), None);
        h.add_test_pane();
        h.app.host_helpers.set_tools(HelperTools {
            worktree: worktree::Tools::with_wtp_program(
                slow_add_wtp(tmp.path(), 0),
                Duration::from_secs(60),
            ),
            pgrep: Pgrep::system(),
        });
        let (pane_id, path) = spawn_worktree_pane(&mut h, tmp.path(), &repo, "feature/finish");
        let started_marker = tmp.path().join("remove-started");
        h.app.host_helpers.set_tools(HelperTools {
            worktree: worktree::Tools::with_wtp_program(
                fake(
                    tmp.path(),
                    "wtp-remove",
                    &format!(
                        "if [ \"$1\" = remove ]; then touch '{}'; sleep 2; fi\nexec python3 '{}' \"$@\"",
                        started_marker.display(),
                        worktree::test_wtp_shim_script().display()
                    ),
                ),
                Duration::from_secs(60),
            ),
            pgrep: Pgrep::system(),
        });
        let rf = tmp.path().join("close.json").to_string_lossy().into_owned();
        h.inject_ipc(HostRequest::ClosePane {
            pane_id,
            response_file: Some(rf.clone()),
        });
        pump_until(&mut h, "wtp remove to start", |_| started_marker.exists());
        assert!(!Path::new(&rf).exists());
        h.app.settle_host_helpers_for_exit();
        let closed = read_reply(&rf);
        assert_eq!(closed["worktree"]["action"], "reclaimed", "{closed}");
        assert!(!path.exists(), "the remove ran to completion");
        assert_eq!(h.app.host_helpers.owed_count(), 0);
    }
}
