//! Resumable runner for a saved command's host steps.
//!
//! Plexi CLI steps act on host state, so they run on the logic thread. Shell
//! steps run on a worker thread so a slow command never holds `App::logic`
//! (stint 1625); the run parks until the worker reports the exit code, which
//! `drain_user_command_runs` picks up in logic. Steps still run strictly in
//! order and stop at the first failure, exactly as `cli::run_steps_with` does.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use super::command_palette::PendingUserCommand;
use crate::app::PlexiApp;
use crate::cli::{ParsedStep, StepRunReport};

/// What a host step did when the logic thread started it.
pub(crate) enum HostStepOutcome {
    /// The step finished with this exit code.
    Exited(i32),
    /// A shell step is running off the logic thread.
    Running(ShellStep),
    /// The step opened a pane whose `--on-load` steps run next; their result
    /// is this step's exit code.
    Nested {
        steps: Vec<String>,
        pane: Option<u64>,
    },
}

/// A shell step in flight on its worker thread.
pub(crate) struct ShellStep {
    exit: mpsc::Receiver<i32>,
    started: std::time::Instant,
}

/// Start `sh -c <script>` as a helper waited for on a worker thread. The
/// step is the user's own command and runs off `App::logic`, so it has no
/// wall-clock limit (`Deadline::HostSession`): a long build or server step
/// runs until it exits. It belongs to the host session instead: it is
/// registered as a pane process group and carries the host's session
/// marker, so quitting the host kills it and everything it started, as it
/// does pane processes. The worker sends the exit code and wakes the host;
/// a launch failure, a kill or a step that could not be stopped is logged
/// and reported as a non-zero exit.
pub(crate) fn spawn_shell_step(
    script: &str,
    cwd: Option<&Path>,
    pane_id: Option<u64>,
    wake: std::sync::Arc<dyn crate::app::ui_mailbox::UiWake>,
) -> HostStepOutcome {
    spawn_marked_shell_step(
        script,
        cwd,
        pane_id,
        crate::host::process_cleanup::session_marker(),
        crate::host::process_cleanup::PaneProcessGroup::record,
        wake,
    )
}

fn spawn_marked_shell_step(
    script: &str,
    cwd: Option<&Path>,
    pane_id: Option<u64>,
    marker: &str,
    register: impl FnOnce(u32) -> crate::host::process_cleanup::PaneProcessGroup,
    wake: std::sync::Arc<dyn crate::app::ui_mailbox::UiWake>,
) -> HostStepOutcome {
    let command_chars = script.chars().count();
    let mut helper = plexi_distribution::bounded::Helper::new("sh");
    helper
        .arg("-c")
        .arg(script)
        .env(crate::host::process_cleanup::SESSION_ENV, marker)
        // No one reads a step's output; its size must not fail the step.
        .stdout(plexi_distribution::bounded::OutputPolicy::Discard)
        .stderr(plexi_distribution::bounded::OutputPolicy::Discard);
    if let Some(root) = cwd {
        helper.current_dir(root);
    }
    if let Some(pid) = pane_id {
        helper.env("PLEXI_PANE_ID", pid.to_string());
    }
    let started = match helper.start("user command shell step") {
        Ok(started) => started,
        Err(e) => {
            log::error!("user_command: shell step failed command_chars={command_chars} error={e}");
            return HostStepOutcome::Exited(1);
        }
    };
    let pid = started.id();
    log::info!("user_command: shell step started pid={pid} command_chars={command_chars}");
    // Held until the step ends; quit cleanup stops every registered group.
    let registration = register(pid);
    let (tx, exit) = mpsc::channel();
    // A thread that fails to start drops `started`, which stops the step.
    let spawned = std::thread::Builder::new()
        .name("user-command-step".to_string())
        .spawn(move || {
            let waited = started.wait_for(plexi_distribution::bounded::Deadline::HostSession);
            drop(registration);
            let code = match waited {
                Ok(exited) => match exited.status.code() {
                    Some(code) => {
                        log::info!("user_command: shell step exited pid={pid} code={code}");
                        code
                    }
                    None => {
                        #[cfg(unix)]
                        let signal = std::os::unix::process::ExitStatusExt::signal(&exited.status);
                        #[cfg(not(unix))]
                        let signal: Option<i32> = None;
                        log::info!(
                            "user_command: shell step killed pid={pid} signal={signal:?}"
                        );
                        1
                    }
                },
                Err(e) => {
                    log::error!(
                        "user_command: shell step failed pid={pid} command_chars={command_chars} error={e}"
                    );
                    1
                }
            };
            // The receiver is gone only when the host dropped the run; the
            // exit code then has no reader.
            let _ = tx.send(code);
            wake.wake("user_command_step");
        });
    match spawned {
        Ok(_) => HostStepOutcome::Running(ShellStep {
            exit,
            started: std::time::Instant::now(),
        }),
        Err(e) => {
            log::error!(
                "user_command: could not start shell step thread command_chars={command_chars} error={e}"
            );
            HostStepOutcome::Exited(1)
        }
    }
}

/// Which pane a frame's steps target.
enum FramePane {
    Known(Option<u64>),
    /// The pane focused when the frame's first step starts (`on_load` after
    /// the main steps).
    FocusedWhenStarted,
}

/// What happens when a frame's steps all exit 0.
enum AfterFrame {
    Finish,
    /// Run these `on_load` steps against the pane focused then.
    OnLoad(Vec<String>),
    /// The frame belongs to a parent step; its result is that step's code.
    ParentStep,
}

/// One step list being walked in order.
struct StepFrame {
    steps: Vec<String>,
    /// Index of the step in progress, or the next to start.
    next: usize,
    /// Rendered text of the step in progress.
    current: Option<String>,
    pane: FramePane,
    after: AfterFrame,
}

enum Next {
    Start(String, ParsedStep),
    Finished(StepRunReport),
}

/// A saved command whose steps are being run by the host.
pub(crate) struct UserCommandRun {
    command: PendingUserCommand,
    ws_root: Option<PathBuf>,
    frames: Vec<StepFrame>,
    shell: Option<ShellStep>,
}

impl UserCommandRun {
    /// Run `steps` against `pane`, then `on_load` against the pane focused
    /// once they all succeed.
    pub(crate) fn new(
        command: PendingUserCommand,
        ws_root: Option<PathBuf>,
        steps: Vec<String>,
        pane: Option<u64>,
        on_load: Vec<String>,
    ) -> Self {
        let after = if on_load.is_empty() {
            AfterFrame::Finish
        } else {
            AfterFrame::OnLoad(on_load)
        };
        Self {
            command,
            ws_root,
            frames: vec![StepFrame {
                steps,
                next: 0,
                current: None,
                pane: FramePane::Known(pane),
                after,
            }],
            shell: None,
        }
    }

    /// Record the in-progress step's exit code (if any) and find the next
    /// step to start, or the command's final report.
    fn advance(&mut self, mut exit: Option<i32>) -> Next {
        loop {
            let Some(frame) = self.frames.last_mut() else {
                return Next::Finished(crate::cli::completed_steps_report(0));
            };
            if let Some(code) = exit.take() {
                let rendered = frame.current.take().unwrap_or_default();
                if code != 0 {
                    let report = crate::cli::failed_step_report(
                        frame.next + 1,
                        frame.steps.len(),
                        rendered,
                        code,
                    );
                    if let Some(done) = self.end_frame(report) {
                        return Next::Finished(done);
                    }
                    exit = Some(code);
                    continue;
                }
                frame.next += 1;
            }
            let total = frame.steps.len();
            if frame.next == total {
                let report = crate::cli::completed_steps_report(total);
                let finished = self.frames.pop().map(|frame| frame.after);
                match finished {
                    Some(AfterFrame::OnLoad(steps)) => self.frames.push(StepFrame {
                        steps,
                        next: 0,
                        current: None,
                        pane: FramePane::FocusedWhenStarted,
                        after: AfterFrame::Finish,
                    }),
                    Some(AfterFrame::ParentStep) => exit = Some(0),
                    Some(AfterFrame::Finish) | None => return Next::Finished(report),
                }
                continue;
            }
            match crate::cli::prepare_step(&frame.steps[frame.next], frame.next + 1, total, &[]) {
                Ok((rendered, parsed)) => {
                    frame.current = Some(rendered.clone());
                    return Next::Start(rendered, parsed);
                }
                Err(refused) => {
                    let code = refused.exit_code;
                    if let Some(done) = self.end_frame(refused) {
                        return Next::Finished(done);
                    }
                    exit = Some(code);
                }
            }
        }
    }

    /// Drop the failed top frame. Returns the command's final report, or
    /// `None` when the failure becomes the parent step's exit code.
    fn end_frame(&mut self, report: StepRunReport) -> Option<StepRunReport> {
        match self.frames.pop().map(|frame| frame.after) {
            Some(AfterFrame::ParentStep) => None,
            // A failed main step skips `on_load`.
            Some(AfterFrame::Finish | AfterFrame::OnLoad(_)) | None => {
                self.frames.clear();
                Some(report)
            }
        }
    }

    fn top_pane(&self) -> Option<&FramePane> {
        self.frames.last().map(|frame| &frame.pane)
    }

    fn pin_top_pane(&mut self, pane: Option<u64>) {
        if let Some(frame) = self.frames.last_mut() {
            frame.pane = FramePane::Known(pane);
        }
    }
}

impl PlexiApp {
    /// Start a saved command's steps. Steps up to the first shell step run
    /// now; the rest resume from `drain_user_command_runs`.
    pub(crate) fn start_user_command_run(&mut self, run: UserCommandRun) {
        log::info!(
            "user_command: run started name={} steps={} in_flight={}",
            run.command.name,
            run.frames.first().map_or(0, |frame| frame.steps.len()),
            self.user_command_runs.len()
        );
        if let Some(parked) = self.advance_user_command_run(run, None) {
            self.user_command_runs.push(parked);
        }
    }

    /// Resume every run whose shell step has exited. Called from `App::logic`.
    pub(crate) fn drain_user_command_runs(&mut self) {
        for mut run in std::mem::take(&mut self.user_command_runs) {
            let polled = run.shell.as_ref().map(|shell| shell.exit.try_recv());
            let code = match polled {
                Some(Err(mpsc::TryRecvError::Empty)) => {
                    self.user_command_runs.push(run);
                    continue;
                }
                Some(Ok(code)) => code,
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    log::error!(
                        "user_command: shell step worker ended without an exit code name={}",
                        run.command.name
                    );
                    1
                }
                // A parked run always has a shell step; finish it as failed.
                None => 1,
            };
            if let Some(shell) = run.shell.take() {
                log::info!(
                    "user_command: shell step exited name={} code={code} elapsed_ms={}",
                    run.command.name,
                    shell.started.elapsed().as_millis()
                );
            }
            if let Some(parked) = self.advance_user_command_run(run, Some(code)) {
                self.user_command_runs.push(parked);
            }
        }
    }

    /// Run steps until one parks on a shell step (returns the run) or the
    /// command finishes (returns `None`).
    fn advance_user_command_run(
        &mut self,
        mut run: UserCommandRun,
        mut exit: Option<i32>,
    ) -> Option<UserCommandRun> {
        loop {
            let (rendered, parsed) = match run.advance(exit.take()) {
                Next::Start(rendered, parsed) => (rendered, parsed),
                Next::Finished(report) => {
                    log::info!(
                        "user_command: run finished name={} exit={} completed={}/{}",
                        run.command.name,
                        report.exit_code,
                        report.completed_steps,
                        report.total_steps
                    );
                    self.mark_workspace_dirty();
                    return None;
                }
            };
            let pane = match run.top_pane() {
                Some(FramePane::Known(pane)) => *pane,
                Some(FramePane::FocusedWhenStarted) | None => {
                    let pane = self.user_command_focused_pane(&run.command);
                    run.pin_top_pane(pane);
                    pane
                }
            };
            let outcome = self.execute_host_step(
                &run.command,
                &rendered,
                &parsed,
                run.ws_root.as_deref(),
                pane,
            );
            match outcome {
                HostStepOutcome::Exited(code) => exit = Some(code),
                HostStepOutcome::Running(shell) => {
                    run.shell = Some(shell);
                    return Some(run);
                }
                HostStepOutcome::Nested { steps, pane } => run.frames.push(StepFrame {
                    steps,
                    next: 0,
                    current: None,
                    pane: FramePane::Known(pane),
                    after: AfterFrame::ParentStep,
                }),
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::host::process_cleanup::PaneProcessGroup;
    use std::time::{Duration, Instant};

    fn recv(outcome: HostStepOutcome, within: Duration) -> Option<i32> {
        let HostStepOutcome::Running(step) = outcome else {
            panic!("the shell step did not start");
        };
        step.exit.recv_timeout(within).ok()
    }

    fn wake() -> std::sync::Arc<dyn crate::app::ui_mailbox::UiWake> {
        std::sync::Arc::new(crate::app::ui_mailbox::RecordingWake::new())
    }

    /// Stint 1626: host quit cleanup finds a running shell step by the host's
    /// session marker and kills it with what it started; the step then
    /// reports failure instead of running on after the host.
    #[test]
    fn host_quit_cleanup_kills_a_running_shell_step() {
        let dir = tempfile::tempdir().unwrap();
        let (step_pid, sleeper) = (dir.path().join("step"), dir.path().join("sleeper"));
        let marker = uuid::Uuid::new_v4().to_string();
        let script = format!(
            "echo $$ > '{}'; sleep 600 & echo $! > '{}'; wait",
            step_pid.display(),
            sleeper.display()
        );
        let outcome = spawn_marked_shell_step(
            &script,
            None,
            None,
            &marker,
            |pid| PaneProcessGroup::record_for(pid, &marker),
            wake(),
        );
        let read_pid = |path: &Path| -> i32 {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Ok(text) = std::fs::read_to_string(path) {
                    if let Ok(pid) = text.trim().parse() {
                        return pid;
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "{} never written",
                    path.display()
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let (step_pid, sleeper) = (read_pid(&step_pid), read_pid(&sleeper));
        crate::host::process_cleanup::stop_pane_processes_for_marker(
            &marker,
            Duration::from_millis(200),
        );
        let code = recv(outcome, Duration::from_secs(10));
        // SAFETY: kill(pid, 0) only probes; SIGKILL cleans up test processes.
        let alive = |pid: i32| unsafe { libc::kill(pid, 0) } == 0;
        let gone_by = Instant::now() + Duration::from_secs(3);
        while (alive(step_pid) || alive(sleeper)) && Instant::now() < gone_by {
            std::thread::sleep(Duration::from_millis(20));
        }
        let survivors: Vec<i32> = [step_pid, sleeper]
            .into_iter()
            .filter(|&p| alive(p))
            .collect();
        for &pid in &survivors {
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        assert!(survivors.is_empty(), "outlived host quit: {survivors:?}");
        assert!(matches!(code, Some(c) if c != 0), "step reported {code:?}");
    }

    /// A step's exit code, its working directory and pane id reach the run.
    #[test]
    fn a_shell_step_reports_its_exit_code_in_its_cwd() {
        let marker = uuid::Uuid::new_v4().to_string();
        let dir = tempfile::tempdir().unwrap();
        let script = "test \"$PLEXI_PANE_ID\" = 7 && test -f here && exit 3";
        std::fs::write(dir.path().join("here"), "").unwrap();
        let outcome = spawn_marked_shell_step(
            script,
            Some(dir.path()),
            Some(7),
            &marker,
            |pid| PaneProcessGroup::record_for(pid, &marker),
            wake(),
        );
        assert_eq!(recv(outcome, Duration::from_secs(10)), Some(3));
    }
}
