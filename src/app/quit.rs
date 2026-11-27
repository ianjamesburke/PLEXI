//! The host process's single exit door.
//!
//! eframe 0.34 calls `App::on_exit` before destroying its window, and only
//! ends the *process* on a later
//! `EventResult::Exit`, and the only thing that can produce one is another
//! winit window event arriving after the window is already gone. A window
//! closed from its chrome (`WM_DELETE_WINDOW`) need not deliver one: on Linux
//! the event loop then parks in `epoll_wait` forever with no window, still
//! holding `notify.sock`, the PTY children and the wgpu worker threads. The
//! result is a headless host that `plexi host status` reports as `ready:
//! false` against a live pid (observed 2026-09-20 on X11/llvmpipe, and the
//! same reason `plexi host stop` has always forced its own exit).
//!
//! Ending the process is therefore ours to do, not eframe's: every quit path
//! finishes in [`exit_host`], and it is the only place in the host that calls
//! `std::process::exit`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::quit_state::{AcceptedQuit, QuitState, SaveStage, UserQuitConfirmation, WindowHide};

/// Schedules the relaunch into the installed build for an update restart.
/// Production uses [`schedule_installed_restart`]; headless hosts use
/// `test_restart_scheduler::record`, which never reads an installed receipt.
pub(crate) type RestartScheduler = fn() -> Result<(), String>;

/// Look up this machine's installed receipt and schedule the relaunch.
pub(crate) fn schedule_installed_restart() -> Result<(), String> {
    let receipt = crate::distribution::installed()?
        .ok_or_else(|| "no managed installation available for restart".to_string())?;
    plexi_distribution::transaction::schedule_restart(&receipt, std::process::id())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
pub(crate) mod test_restart_scheduler {
    //! Headless restart scheduler: counts requests on this thread and touches
    //! no installation.
    use std::cell::Cell;

    thread_local! {
        static REQUESTS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn record() -> Result<(), String> {
        log::info!("update restart: test scheduler recorded a request");
        REQUESTS.with(|requests| requests.set(requests.get() + 1));
        Ok(())
    }

    /// Requests recorded on this thread since the last call.
    pub(crate) fn take() -> usize {
        REQUESTS.with(|requests| requests.replace(0))
    }
}
/// Failed final saves tolerated (one per second) before asking the user, or,
/// for `host stop`, before exiting unsaved (stint 1540).
pub(crate) const MAX_QUIT_SAVE_ATTEMPTS: u32 = 5;

/// Process-local monotonic clock, plus wall-clock milliseconds so a quit can
/// be lined up with the system log (stint 1541). Input reception is the
/// plugin callback boundary, not physical input or compositor presentation.
/// External observers measure those boundaries and actual process death
/// using the logged PID. `exit_call` also writes one `quit_summary` line.
pub(crate) fn trace(phase: &str) {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    let origin = ORIGIN.get_or_init(Instant::now);
    let elapsed_us = origin.elapsed().as_micros() as u64;
    let line = format!(
        "quit_trace: pid={} wall_ms={} elapsed_us={elapsed_us} phase={phase}",
        std::process::id(),
        wall_clock_ms()
    );
    log::info!("{line}");
    #[cfg(test)]
    eprintln!("{line}");
    // First time each milestone was traced, as elapsed_us + 1 (0 = never).
    static ACCEPTED_US: AtomicU64 = AtomicU64::new(0);
    static HIDE_REQUESTED_US: AtomicU64 = AtomicU64::new(0);
    static HIDDEN_US: AtomicU64 = AtomicU64::new(0);
    let first = |slot: &AtomicU64| {
        let _ = slot.compare_exchange(0, elapsed_us + 1, Ordering::Relaxed, Ordering::Relaxed);
    };
    match phase {
        "quit_accepted" => first(&ACCEPTED_US),
        "window_hide_requested" => first(&HIDE_REQUESTED_US),
        "window_hidden_native" => first(&HIDDEN_US),
        "exit_call" => {
            let at = |slot: &AtomicU64| slot.load(Ordering::Relaxed).checked_sub(1);
            // The confirmed hide when there is one, else the request.
            let hide = at(&HIDDEN_US).or_else(|| at(&HIDE_REQUESTED_US));
            log::info!(
                "{}",
                quit_summary(
                    std::process::id(),
                    wall_clock_ms(),
                    at(&ACCEPTED_US),
                    hide,
                    elapsed_us
                )
            );
        }
        _ => {}
    }
}

fn wall_clock_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0)
}

/// The one line a quit leaves for "how long was the window gone before the
/// process ended": milliseconds from the hide and from acceptance to the exit
/// call, `none` when that milestone never happened (an `on_exit` teardown has
/// no coordinated hide).
fn quit_summary(
    pid: u32,
    wall_ms: u128,
    accepted_us: Option<u64>,
    hide_us: Option<u64>,
    exit_us: u64,
) -> String {
    let ms = |from: Option<u64>| match from {
        Some(from) => format!("{:.1}", exit_us.saturating_sub(from) as f64 / 1000.0),
        None => "none".to_string(),
    };
    format!(
        "quit_summary: pid={pid} wall_ms={wall_ms} hide_to_exit_ms={} accepted_to_exit_ms={}",
        ms(hide_us),
        ms(accepted_us)
    )
}

fn await_restart_coordinator(mut stream: crate::platform::ipc::IpcStream) -> std::io::Result<()> {
    let timeout = Some(crate::rpc::DEFAULT_TIMEOUT);
    stream.set_write_timeout(timeout)?;
    stream.set_read_timeout(timeout)?;
    let accepted = serde_json::json!({"ok": true, "pid": std::process::id()});
    writeln!(stream, "{accepted}")?;
    stream.flush()?;
    let mut ready = [0];
    stream.read_exact(&mut ready)?;
    if ready != crate::rpc::RESTART_READY {
        return Err(std::io::Error::other(
            "invalid restart coordinator acknowledgement",
        ));
    }
    Ok(())
}

impl super::PlexiApp {
    pub(super) fn queue_restart_shutdown(&mut self, response_file: &str) {
        if self.quit_coordinator.host_stop_queued() || self.quit_coordinator.is_active() {
            crate::rpc::write_json_response(
                response_file,
                serde_json::json!({
                    "ok": false,
                    "error_code": "host_stop_in_progress",
                    "error": "The host is already handling a quit request",
                }),
            );
            return;
        }
        self.quit_coordinator
            .queue_host_stop(Some(response_file.to_owned()), true);
    }

    fn start_restart_handoff(&mut self) {
        let result = self
            .quit_coordinator
            .accepted_mut()
            .and_then(|quit| quit.stop_reply.take())
            .and_then(|token| crate::rpc::ReplyRegistry::take(&token))
            .ok_or_else(|| std::io::Error::other("missing reply connection"))
            .and_then(|stream| {
                let (tx, rx) = mpsc::channel();
                let wake = self.ui_wake.clone();
                std::thread::Builder::new()
                    .name("restart-handoff".into())
                    .spawn(move || {
                        let result = await_restart_coordinator(stream);
                        if tx.send(result).is_ok() {
                            wake.wake("restart_handoff");
                        }
                    })?;
                Ok(rx)
            });
        match result {
            Ok(handoff) => {
                if let Some(quit) = self.quit_coordinator.accepted_mut() {
                    quit.restart_handoff = Some(handoff);
                }
            }
            Err(error) => self.cancel_restart_shutdown(&error),
        }
    }

    fn cancel_restart_shutdown(&mut self, error: &impl std::fmt::Display) {
        log::warn!("quit: restart cancelled during coordinator handoff: {error}");
        self.quit_coordinator
            .go(QuitState::Idle, "restart handoff failed");
    }

    /// Acceptance freezes further UI/IPC mutation. The next logic pass takes
    /// the final snapshot, after the accepting UI pass has finished.
    pub(crate) fn request_quit(&mut self, reason: ExitReason) {
        self.accept_quit(reason, None, false);
    }

    /// `plexi host stop`, taken from the queue by the logic pass.
    pub(crate) fn request_host_stop(&mut self, reply: Option<String>, restart: bool) {
        self.accept_quit(ExitReason::HostStop, reply, restart);
    }

    fn accept_quit(&mut self, reason: ExitReason, stop_reply: Option<String>, restart: bool) {
        if let Some(quit) = self.quit_coordinator.accepted_mut() {
            // A stop queued in the same pass another quit was accepted is
            // answered by that quit's exit.
            if quit.stop_reply.is_none() {
                quit.stop_reply = stop_reply;
            }
        } else {
            if reason == ExitReason::HostStop {
                if let Some(files) = self.unsaved_editor_files() {
                    refuse_host_stop_for_unsaved_editor(stop_reply, &files);
                    return;
                }
            }
            trace("quit_accepted");
            log::info!("quit: coordinator accepted reason={}", reason.label());
            self.quit_coordinator.go(
                QuitState::Accepted(AcceptedQuit::new(reason, stop_reply)),
                "quit accepted",
            );
            if restart {
                self.start_restart_handoff();
            }
        }
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::CancelClose);
        self.ui_wake.wake("quit_coordinator");
    }

    /// Workspace snapshots only remember the editor's file and view state.
    /// Host stop has no one to answer a save prompt, so keep dirty buffers live.
    fn unsaved_editor_files(&self) -> Option<String> {
        let dirty_files: Vec<_> = self
            .windows
            .iter()
            .flat_map(|window| window.panes.values())
            .filter_map(|pane| pane.as_view()?.runtime.editor_buffer())
            .filter(|buffer| buffer.dirty)
            .map(|buffer| format!("{:?}", buffer.path))
            .collect();
        (!dirty_files.is_empty()).then(|| dirty_files.join(", "))
    }

    pub(crate) fn persist_quit_state(&mut self) -> bool {
        let Some(quit) = self.quit_coordinator.accepted() else {
            log::error!("quit: final save requested with no accepted quit");
            return false;
        };
        let reason = quit.reason;
        let failures = match &quit.save {
            SaveStage::Saved if !self.workspace_dirty => {
                trace("save_reused");
                return true;
            }
            SaveStage::Waived { .. } => {
                trace("save_waived");
                log::warn!(
                    "quit: continuing without saving the workspace ({})",
                    if reason == ExitReason::HostStop {
                        "host stop has nobody to ask"
                    } else {
                        "the user chose to quit without saving"
                    }
                );
                return true;
            }
            save => save.failures(),
        };
        trace("save_begin");
        let result = self.save_workspace_result();
        let saved = result.is_ok();
        trace(if saved { "save_ok" } else { "save_failed" });
        self.workspace_dirty = !saved;
        let next = match result {
            Ok(()) => SaveStage::Saved,
            Err(error) => {
                let failures = failures + 1;
                if self.workspace_load_failure.is_some() && reason == ExitReason::HostStop {
                    // `plexi host stop` has nobody to answer a dialog, and a
                    // boot-time block never clears. The file stays as found
                    // and the exit reports the workspace unsaved.
                    log::error!(
                        "quit: host stop exits without saving; the session never restored its workspace: {error}"
                    );
                    SaveStage::Waived { failures }
                } else if reason == ExitReason::HostStop && failures >= MAX_QUIT_SAVE_ATTEMPTS {
                    // `plexi host stop` has nobody to answer the dialog either:
                    // once the bounded retries are spent it exits unsaved and
                    // says so in its reply (stint 1540).
                    log::error!(
                        "quit: host stop exits without saving after {failures} failed saves: {error}"
                    );
                    SaveStage::Waived { failures }
                } else if self.workspace_load_failure.is_some()
                    || failures >= MAX_QUIT_SAVE_ATTEMPTS
                {
                    // A boot-time block cannot clear by retrying: ask at once.
                    SaveStage::AskingUser {
                        failures,
                        cause: error.to_string(),
                    }
                } else {
                    SaveStage::Pending { failures }
                }
            }
        };
        let done = matches!(next, SaveStage::Saved | SaveStage::Waived { .. });
        if let Some(quit) = self.quit_coordinator.accepted_mut() {
            quit.save = next;
        }
        done
    }

    /// The final save for a backend teardown (`on_exit`: Dock Quit, logout),
    /// where no dialog can answer (stint 1539). Retries a failing save at most
    /// `MAX_QUIT_SAVE_ATTEMPTS` times, `retry_delay` apart, then gives up; a
    /// boot-time block gives up at once. The workspace on disk may then not
    /// hold the latest changes (a failed attempt can already have rotated the
    /// `.prev` copy or renamed a new file in). Returns what the exit records.
    pub(crate) fn persist_for_teardown(&mut self, retry_delay: Duration) -> WorkspaceDurability {
        self.accept_teardown();
        // At most MAX_QUIT_SAVE_ATTEMPTS tries and one fewer delays.
        let mut saved = false;
        for attempt in 1..=MAX_QUIT_SAVE_ATTEMPTS {
            if self.persist_quit_state() {
                saved = true;
                break;
            }
            if let Some(block) = self.workspace_save_block() {
                log::error!("quit: backend teardown cannot save the workspace: {block}");
                break;
            }
            log::error!(
                "quit: backend teardown save attempt {attempt} of {MAX_QUIT_SAVE_ATTEMPTS} failed"
            );
            if attempt < MAX_QUIT_SAVE_ATTEMPTS {
                std::thread::sleep(retry_delay);
            }
        }
        if !saved {
            log::error!(
                "quit: backend teardown gives up on the final save; the workspace on disk may not hold the latest changes"
            );
        }
        self.quit_workspace_durability()
    }

    /// A backend teardown is a quit nobody can veto: one already accepted
    /// keeps its reason and progress, anything else is accepted as a window
    /// close.
    fn accept_teardown(&mut self) {
        if self.quit_coordinator.is_active() {
            return;
        }
        self.quit_coordinator.go(
            QuitState::Accepted(AcceptedQuit::new(ExitReason::WindowClosed, None)),
            "backend teardown",
        );
    }

    /// `on_exit`: the logged last resort. Saves within its bound, then leaves
    /// through the same flush list and exit as the coordinated quit.
    pub(crate) fn exit_from_teardown(&mut self, retry_delay: Duration) -> ! {
        self.persist_for_teardown(retry_delay);
        self.finish_exit()
    }

    /// Whether the workspace this process is about to exit with is on disk.
    /// Only a successful final save counts; a waived save does not.
    pub(crate) fn quit_workspace_durability(&self) -> WorkspaceDurability {
        if self
            .quit_coordinator
            .accepted()
            .is_some_and(|quit| quit.save == SaveStage::Saved)
        {
            WorkspaceDurability::Saved
        } else {
            WorkspaceDurability::Unsaved
        }
    }

    /// Dialog outcome for a quit whose save kept failing. `quit_anyway` skips
    /// the save (the file on disk is left as it is); otherwise the quit is
    /// cancelled and the window stays.
    pub(crate) fn resolve_quit_save_failure(&mut self, quit_anyway: bool) {
        self.pane_close_dialog = None;
        if quit_anyway {
            if let Some(quit) = self.quit_coordinator.accepted_mut() {
                log::warn!(
                    "quit: user chose Quit without saving after {} failed saves: {}",
                    quit.save.failures(),
                    match &quit.save {
                        SaveStage::AskingUser { cause, .. } => cause.as_str(),
                        _ => "unknown",
                    }
                );
                quit.save = SaveStage::Waived {
                    failures: quit.save.failures(),
                };
            }
            self.ui_wake.wake("quit_without_saving");
        } else {
            log::info!("quit: user cancelled after save failures; keeping the window");
            self.quit_coordinator
                .go(QuitState::Idle, "save failure dialog cancelled");
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
    }

    /// Returns true while the coordinator owns servicing. Hidden windows do
    /// not run App::ui, so retry, restart and cleanup must all live in logic.
    pub(crate) fn service_quit(&mut self, frame: &eframe::Frame) -> bool {
        self.service_quit_with_restart(frame, self.restart_scheduler)
    }

    pub(crate) fn service_quit_with_restart(
        &mut self,
        frame: &eframe::Frame,
        schedule_restart: impl FnOnce() -> Result<(), String>,
    ) -> bool {
        if let Some(pane) = self.quit_coordinator.pending_editor() {
            if !self.windows.iter().any(|w| w.panes.contains_key(&pane)) {
                self.quit_coordinator
                    .go(QuitState::Idle, "awaited editor pane closed");
                log::info!("quit: editor pane={pane} closed while awaiting an answer");
            }
            return false;
        }
        let Some(quit) = self.quit_coordinator.accepted() else {
            return false;
        };
        let reason = quit.reason;
        if let Some(received) = quit.restart_handoff.as_ref().map(Receiver::try_recv) {
            match received {
                Ok(Ok(())) => {
                    log::info!("quit: restart coordinator detached; proceeding with stop");
                    if let Some(quit) = self.quit_coordinator.accepted_mut() {
                        quit.restart_handoff = None;
                    }
                }
                Err(TryRecvError::Empty) => return true,
                Ok(Err(error)) => {
                    self.cancel_restart_shutdown(&error);
                    return false;
                }
                Err(TryRecvError::Disconnected) => {
                    self.cancel_restart_shutdown(&"handoff worker exited without confirmation");
                    return false;
                }
            }
        }
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::CancelClose);
        let retrying = match self.quit_coordinator.accepted().map(|quit| &quit.save) {
            // Waiting on the user's answer to the dialog.
            Some(SaveStage::AskingUser { .. }) => return false,
            Some(SaveStage::Retrying { at, .. }) if Instant::now() < *at => return false,
            Some(SaveStage::Retrying { .. }) => true,
            _ => false,
        };
        // A failed save leaves the UI usable, so the editor may now be dirty.
        if retrying {
            if reason == ExitReason::HostStop {
                if let Some(files) = self.unsaved_editor_files() {
                    let reply = self
                        .quit_coordinator
                        .accepted_mut()
                        .and_then(|quit| quit.stop_reply.take());
                    refuse_host_stop_for_unsaved_editor(reply, &files);
                    self.quit_coordinator
                        .go(QuitState::Idle, "host stop refused for unsaved editor");
                    return false;
                }
            } else if !self.prepare_user_quit_for(reason) {
                return false;
            }
        }
        if !self.persist_quit_state() {
            let Some(quit) = self.quit_coordinator.accepted_mut() else {
                return false;
            };
            if let SaveStage::AskingUser { failures, cause } = &quit.save {
                log::error!(
                    "quit: workspace save failed {failures} time(s), asking the user: {cause}"
                );
                self.ui_wake.wake("quit_save_failed");
                return false;
            }
            let delay = Duration::from_secs(1);
            quit.save = SaveStage::Retrying {
                failures: quit.save.failures(),
                at: Instant::now() + delay,
            };
            self.schedule_workspace_save_wake(delay);
            log::error!("quit: save failed; keeping window and retrying before cleanup");
            return false;
        }
        let Some(quit) = self.quit_coordinator.accepted_mut() else {
            return false;
        };
        if reason == ExitReason::UpdateRestart && !quit.restart_scheduled {
            if let Err(error) = schedule_restart() {
                log::error!("update restart failed: {error}");
                trace("restart_failed");
                self.quit_coordinator
                    .go(QuitState::Idle, "update restart could not be scheduled");
                return false;
            }
            quit.restart_scheduled = true;
            trace("restart_scheduled");
        }
        if quit.hide == WindowHide::NotRequested {
            trace("window_hide_requested");
            // AppKit confirmation is distinct from egui's occlusion flag:
            // occluded means covered, and cannot prove the window disappeared.
            let hidden = hide_native_window(frame);
            quit.hide = WindowHide::Requested {
                handed_off: hidden.is_ok(),
            };
            match hidden {
                Ok(true) => {
                    trace("window_hidden_native");
                    // Native hide is acknowledged now. Deferred backends only
                    // enter this watchdog mode on the following logic pass.
                    crate::platform::logging::mark_shutdown_cleanup(&self.ui_phase, true, true);
                }
                Ok(false) => trace("window_hide_deferred"),
                // The window is cosmetic; the quit is not (stint 1542). A
                // failed hide is logged and the quit goes on: egui is still
                // asked to hide below, and cleanup and exit follow.
                Err(error) => {
                    trace("window_hide_failed");
                    log::error!("quit: cannot hide native window: {error}; continuing the quit");
                }
            }
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
            // Return control to the window system before any child grace wait.
            self.ui_wake.wake("quit_window_hidden");
            return true;
        }
        trace("post_hide_logic");
        self.finish_exit()
    }

    /// The one flush list and exit, shared by the coordinated quit and
    /// `on_exit`. The final save has already run.
    fn finish_exit(&mut self) -> ! {
        self.begin_quit_cleanup();
        self.flush_ui_scale_persist_now();
        self.flush_editor_positions_now();
        self.bank_final_focus_segment();
        self.flush_admission_summaries();
        let reason = self
            .quit_coordinator
            .accepted_reason()
            .unwrap_or(ExitReason::WindowClosed);
        let durability = self.quit_workspace_durability();
        let reply = self
            .quit_coordinator
            .accepted_mut()
            .and_then(|quit| quit.stop_reply.take());
        exit_host(reason, durability, reply)
    }

    pub(super) fn begin_quit_cleanup(&self) {
        crate::platform::logging::mark_shutdown_cleanup(
            &self.ui_phase,
            self.quit_coordinator.is_active(),
            self.quit_coordinator.window_handed_off(),
        );
    }

    pub(crate) fn working_agent_count(&self) -> usize {
        self.windows
            .iter()
            .flat_map(|window| window.panes.values())
            .filter(|pane| {
                pane.agent()
                    .is_some_and(|agent| agent.state == crate::protocol::AgentState::Working)
            })
            .count()
    }

    pub(crate) fn resolve_quit_confirmation(&mut self, confirmed: bool) {
        trace(if confirmed {
            "confirmation_accepted"
        } else {
            "confirmation_cancelled"
        });
        self.pane_close_dialog = None;
        let QuitState::ConfirmingAgents { reason } = *self.quit_coordinator.state() else {
            self.reset_keyboard_quit_confirmation();
            return;
        };
        self.quit_coordinator
            .go(QuitState::Idle, "working-agent confirmation answered");
        if confirmed {
            log::info!(
                "quit: working-agent confirmation accepted reason={}",
                reason.label()
            );
            self.continue_user_quit(reason, UserQuitConfirmation::Accepted);
        } else {
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            log::info!("quit: working-agent confirmation cancelled");
        }
    }

    pub(super) fn request_keyboard_quit(&mut self, ctx: &egui::Context) {
        trace("quit_action");
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        self.continue_user_quit(ExitReason::WindowClosed, UserQuitConfirmation::Keyboard);
    }

    pub(super) fn keyboard_quit_timed_out(&self, now: Instant) -> bool {
        self.quit_coordinator.keyboard_presses_timed_out(now)
    }

    pub(super) fn reset_keyboard_quit_confirmation(&mut self) {
        if self.quit_coordinator.keyboard_presses() > 0 {
            self.quit_coordinator
                .go(QuitState::Idle, "keyboard confirmation reset");
        }
    }

    fn advance_keyboard_quit_confirmation(&mut self) -> bool {
        // An accepted quit (a save retry) has no presses left to count.
        if !self.confirm_quit() || self.quit_coordinator.is_active() {
            return true;
        }
        let now = Instant::now();
        let count = self.quit_coordinator.next_keyboard_press(now);
        if count >= 3 {
            self.quit_coordinator
                .go(QuitState::Idle, "third Cmd+Q press");
            return true;
        }
        self.quit_coordinator.go(
            QuitState::CountingPresses { count, last: now },
            "Cmd+Q press",
        );
        log::info!("quit: keyboard confirmation press={count}");
        false
    }

    fn continue_user_quit(&mut self, reason: ExitReason, confirmation: UserQuitConfirmation) {
        if reason == ExitReason::UpdateRestart && self.update_status.ready().is_none() {
            log::info!("update: restart cancelled because the update is no longer ready");
            return;
        }
        if self.prepare_user_quit_with_confirmation(reason, confirmation) {
            self.request_quit(reason);
        }
    }

    pub(super) fn resolve_quit_editor(&mut self, pane: u64, cancelled: bool) {
        let QuitState::AwaitingEditor {
            reason,
            pane: pending,
            confirmation,
        } = *self.quit_coordinator.state()
        else {
            return;
        };
        if pane != pending {
            return;
        }
        self.quit_coordinator
            .go(QuitState::Idle, "unsaved editor answered");
        if cancelled {
            log::info!("quit: cancelled at unsaved editor pane={pane}");
        } else {
            log::info!("quit: continuing after unsaved editor pane={pane}");
            self.continue_user_quit(reason, confirmation);
        }
    }

    /// Dock Quit, a quit Apple event, logout, restart and shutdown (stint
    /// 1538). AppKit is waiting on `NSTerminateLater`; the request takes the
    /// Cmd+Q decision. If the quit goes ahead, `exit_host` ends the process
    /// and AppKit's question is moot. Whenever Plexi is not exiting without
    /// waiting on anyone (a prompt is up, a save failed, the person cancelled)
    /// the system request is cancelled at once, so a logout never sits on an
    /// unanswered prompt: nothing is lost and the person logs out again after
    /// answering.
    pub(crate) fn service_system_quit(&mut self) {
        while let Ok(reason) = self.system_quit.rx.try_recv() {
            self.system_quit.pending = true;
            if self.quit_coordinator.is_active() {
                log::info!(
                    "quit: system quit reason={} joins the quit already under way",
                    reason.label()
                );
                continue;
            }
            trace("system_quit");
            if self.prepare_user_quit() {
                log::info!(
                    "quit: system quit reason={} accepted; nothing to ask",
                    reason.label()
                );
                self.request_quit(ExitReason::WindowClosed);
            }
        }
        if self.system_quit.pending && !self.quit_coordinator.blocks_input() {
            self.system_quit.pending = false;
            log::info!("quit: system quit cancelled; Plexi waits for the person to answer");
            crate::platform::system_quit::cancel_terminate();
        }
    }

    pub(crate) fn prepare_user_quit(&mut self) -> bool {
        self.prepare_user_quit_for(ExitReason::WindowClosed)
    }

    /// Check for interactive vetoes before accepting a user quit.
    pub(super) fn prepare_user_quit_for(&mut self, reason: ExitReason) -> bool {
        let confirmation = if self.quit_coordinator.is_active() {
            // The coordinator may be retrying a save after accepting the quit.
            UserQuitConfirmation::Accepted
        } else {
            UserQuitConfirmation::WorkingAgents
        };
        self.prepare_user_quit_with_confirmation(reason, confirmation)
    }

    fn prepare_user_quit_with_confirmation(
        &mut self,
        reason: ExitReason,
        confirmation: UserQuitConfirmation,
    ) -> bool {
        if self.quit_coordinator.accepted_reason() == Some(ExitReason::HostStop) {
            return false;
        }
        if let Some(pane) = self.quit_coordinator.pending_editor() {
            if self.focused_pane_id() != Some(pane) {
                self.pane_navigate(pane);
            }
            return false;
        }
        if self.quit_coordinator.confirming_agents() {
            return false;
        }
        let veto = self.windows.iter_mut().find_map(|window| {
            window.panes.iter_mut().find_map(|(id, pane)| {
                let view = pane.as_view_mut()?;
                match &mut view.runtime {
                    crate::host::pane::ViewRuntime::Builtin(app) => {
                        (!app.request_quit()).then_some(*id)
                    }
                    _ => None,
                }
            })
        });
        if let Some(pane) = veto {
            self.quit_coordinator.go(
                QuitState::AwaitingEditor {
                    reason,
                    pane,
                    confirmation,
                },
                "unsaved editor vetoed the quit",
            );
            self.pane_navigate(pane);
            log::info!(
                "quit: awaiting unsaved editor pane={pane} reason={}",
                reason.label()
            );
            return false;
        }
        let working = self.working_agent_count();
        if !self.quit_coordinator.is_active()
            && !matches!(confirmation, UserQuitConfirmation::Accepted)
            && working > 0
        {
            self.quit_coordinator
                .go(QuitState::ConfirmingAgents { reason }, "agents are working");
            trace("confirmation_required");
            log::info!("quit: confirmation required working_agents={working}");
            return false;
        }
        match confirmation {
            UserQuitConfirmation::Keyboard => self.advance_keyboard_quit_confirmation(),
            UserQuitConfirmation::WorkingAgents | UserQuitConfirmation::Accepted => true,
        }
    }
}

/// Answer a `host stop` refused because editors hold unsaved changes.
fn refuse_host_stop_for_unsaved_editor(reply: Option<String>, files: &str) {
    log::info!("quit: host stop refused for unsaved editors: {files}");
    if let Some(reply) = reply {
        crate::rpc::write_json_response(
            &reply,
            serde_json::json!({
                "ok": false,
                "error_code": "unsaved_editor_changes",
                "error": format!("Save or discard the editor changes in {files} before stopping the host"),
            }),
        );
    }
}

#[cfg(test)]
thread_local! {
    // Test seam: the result every native hide reports while set (stint 1542).
    static HIDE_RESULT: std::cell::Cell<Option<Result<bool, &'static str>>> =
        const { std::cell::Cell::new(None) };
}

/// `true` means AppKit reports the window removed from its visible list.
/// This is not a compositor timestamp. Other platforms use the viewport
/// command and yield to the backend before cleanup without claiming an ack.
fn hide_native_window(frame: &eframe::Frame) -> Result<bool, &'static str> {
    #[cfg(test)]
    if let Some(result) = HIDE_RESULT.with(std::cell::Cell::get) {
        return result;
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSView;
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        #[cfg(test)]
        if frame.window_handle().is_err() {
            return Ok(false); // Headless harness, never installed visibility evidence.
        }
        let _main = objc2::MainThreadMarker::new().ok_or("not on AppKit main thread")?;
        let handle = frame
            .window_handle()
            .map_err(|_| "window handle unavailable")?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err("not an AppKit window");
        };
        // SAFETY: borrowed eframe NSView remains alive; AppKit main thread above.
        let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        let window = view.window().ok_or("NSView has no window")?;
        window.orderOut(None);
        // AppKit hands `orderOut` to the window server when the run loop next
        // turns, and quit cleanup blocks the main thread straight after, so
        // the window stayed on screen until exit (stint 1522). Commit now.
        match objc2::runtime::AnyClass::get(c"CATransaction") {
            // SAFETY: `+[CATransaction flush]` takes no arguments, returns
            // void, and is called on the AppKit main thread checked above.
            Some(transaction) => unsafe { objc2::msg_send![transaction, flush] },
            None => log::warn!("quit: CATransaction unavailable; window hide is not flushed"),
        }
        if window.isVisible() {
            Err("AppKit window remains visible")
        } else {
            Ok(true)
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = frame;
        Ok(false)
    }
}

/// The notify socket this process bound, with the identity it had at bind
/// time. Written once by the socket listener; read by teardown and by the
/// ownership check.
static BOUND_NOTIFY_SOCKET: OnceLock<BoundSocket> = OnceLock::new();

#[derive(Debug, Clone)]
struct BoundSocket {
    path: PathBuf,
    /// `(dev, ino)` when the socket could be stat'd at bind time.
    identity: Option<(u64, u64)>,
}

/// What teardown did with the notify socket. Named cases rather than a bool so
/// the log line says which one happened and the tests can tell them apart.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SocketRelease {
    /// This process never bound one (CLI invocations, tests).
    NotBound,
    Removed,
    /// Already gone — a second quit path, or someone cleaned up for us.
    Missing,
    /// A *different* socket sits at the path now: another host owns it, so
    /// removing the file would take that host's IPC down with us.
    Reused,
    Failed(String),
}

fn identity(path: &Path) -> Option<(u64, u64)> {
    // Unix notify sockets are filesystem inodes; (dev, ino) detects reuse.
    // Windows uses named pipes with no filesystem entry — identity is unused.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

/// Remember the socket [`exit_host`] must remove. Called once, right after the
/// listener binds.
pub(crate) fn record_notify_socket(path: &Path) {
    let bound = BoundSocket {
        path: path.to_path_buf(),
        identity: identity(path),
    };
    if BOUND_NOTIFY_SOCKET.set(bound).is_err() {
        log::warn!(
            "quit: notify socket already recorded — keeping the first binding ({:?})",
            BOUND_NOTIFY_SOCKET.get().map(|b| &b.path)
        );
    }
}

/// Whether the socket file at the bound path is still the one this host bound.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SocketOwnership {
    /// The file is the one this host bound.
    Owned,
    /// No file identity was recorded (Windows named pipes have none).
    Untracked,
    /// A different socket file sits at the path: another process took it.
    Replaced,
    /// The file is gone; the CLI can no longer reach this host.
    Missing,
}

fn ownership(bound: &BoundSocket) -> SocketOwnership {
    match (bound.identity, identity(&bound.path)) {
        (None, _) => SocketOwnership::Untracked,
        (Some(_), None) => SocketOwnership::Missing,
        (Some(at_bind), Some(now)) if now != at_bind => SocketOwnership::Replaced,
        (Some(_), Some(_)) => SocketOwnership::Owned,
    }
}

/// How often the host re-checks that it still owns its socket.
const SOCKET_OWNERSHIP_INTERVAL: Duration = Duration::from_secs(2);

/// Warn once, the first time `bound` is found replaced or missing, and return
/// the message to surface. `reported` keeps it to one report per process.
fn report_lost_socket(
    bound: &BoundSocket,
    reported: &std::sync::atomic::AtomicBool,
) -> Option<String> {
    let state = ownership(bound);
    if !matches!(state, SocketOwnership::Replaced | SocketOwnership::Missing) {
        return None;
    }
    if reported.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let message = match state {
        SocketOwnership::Replaced => format!(
            "another process replaced this host's socket {}; Plexi commands now reach that process, not this window",
            bound.path.display()
        ),
        _ => format!(
            "this host's socket {} was removed; Plexi commands can no longer reach this window",
            bound.path.display()
        ),
    };
    log::warn!("pane_ipc: {message}");
    Some(message)
}

/// Called from `App::logic`: re-check the bound socket at most every
/// [`SOCKET_OWNERSHIP_INTERVAL`] and return a message the first time it is
/// lost (stint 1459).
pub(crate) fn check_notify_socket_ownership() -> Option<String> {
    static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static LAST_CHECK: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let bound = BOUND_NOTIFY_SOCKET.get()?;
    {
        let mut last = LAST_CHECK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.is_some_and(|at| at.elapsed() < SOCKET_OWNERSHIP_INTERVAL) {
            return None;
        }
        *last = Some(Instant::now());
    }
    report_lost_socket(bound, &REPORTED)
}

fn release(bound: &BoundSocket) -> SocketRelease {
    match (identity(&bound.path), bound.identity) {
        (None, _) => SocketRelease::Missing,
        (Some(now), Some(at_bind)) if now != at_bind => SocketRelease::Reused,
        _ => match std::fs::remove_file(&bound.path) {
            Ok(()) => SocketRelease::Removed,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => SocketRelease::Missing,
            Err(e) => SocketRelease::Failed(format!("{}: {e}", bound.path.display())),
        },
    }
}

/// Unbind pane IPC by removing the socket file this process bound, so no CLI
/// can connect to a host that is on its way out.
pub(crate) fn release_notify_socket() -> SocketRelease {
    match BOUND_NOTIFY_SOCKET.get() {
        None => SocketRelease::NotBound,
        Some(bound) => release(bound),
    }
}

/// Why the host is exiting. Each path owns its pane-process grace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExitReason {
    WindowClosed,
    HostStop,
    UpdateRestart,
}

impl ExitReason {
    fn label(self) -> &'static str {
        match self {
            Self::WindowClosed => "window closed",
            Self::HostStop => "host stop",
            Self::UpdateRestart => "update restart",
        }
    }

    /// The user is watching a window close, so keep it short. `host stop` and
    /// update restarts keep 2s so agent processes can flush.
    fn grace(self) -> Duration {
        match self {
            Self::WindowClosed => Duration::from_millis(500),
            Self::HostStop | Self::UpdateRestart => Duration::from_secs(2),
        }
    }
}

/// Whether the final workspace save reached disk before exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceDurability {
    Saved,
    /// The save failed and the user quit without it, or a blocked session
    /// was torn down. The file on disk is whatever was there before.
    Unsaved,
}

/// Process exit status: zero only for a clean quit. Leftover pane
/// processes, a socket that could not be released, or a workspace that was
/// not saved each make the exit unclean.
fn exit_status(remaining: usize, socket: &SocketRelease, workspace: WorkspaceDurability) -> i32 {
    i32::from(
        remaining != 0
            || matches!(socket, SocketRelease::Failed(_))
            || workspace == WorkspaceDurability::Unsaved,
    )
}

/// What `plexi host stop` is told just before the process exits (stint 1430).
pub(crate) fn stop_reply(
    status: i32,
    remaining: usize,
    socket: &SocketRelease,
    workspace: WorkspaceDurability,
) -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "status": status,
        "workspace": match workspace {
            WorkspaceDurability::Saved => "saved",
            WorkspaceDurability::Unsaved => "unsaved",
        },
        "remaining_processes": remaining,
        "socket_released": !matches!(socket, SocketRelease::Failed(_)),
    })
}

/// Stamp a clean shutdown only when no owned process is left, so the next
/// start can tell a clean quit from a death (stint 1523). Returns whether the
/// stamp was written.
pub(crate) fn record_quit_integrity(remaining: usize, dir: &std::path::Path) -> bool {
    if remaining == 0 {
        crate::broker::integrity::mark_clean_shutdown(dir);
        trace("integrity_written");
        true
    } else {
        log::error!("quit: refusing clean-shutdown stamp; {remaining} owned processes remain");
        false
    }
}

/// Tear the host down and end the process. Never returns. `stop_reply` is
/// `plexi host stop`'s reply token, answered with the real outcome before the
/// process ends.
pub(crate) fn exit_host(
    reason: ExitReason,
    workspace: WorkspaceDurability,
    stop_reply_to: Option<String>,
) -> ! {
    trace("cleanup_begin");
    let start = Instant::now();
    let reason_label = reason.label();
    let k = crate::host::process_cleanup::stop_pane_processes(reason.grace());
    log::info!(
        "quit: killed {} pane process groups ({} SIGKILL) reason={reason_label} grace_ms={} tmux_ms={} scan_ms={} hup_term_ms={} grace_waited_ms={} kill_ms={} total_ms={}",
        k.groups,
        k.escalated,
        k.grace.as_millis(),
        k.tmux.as_millis(),
        k.scan.as_millis(),
        k.signal.as_millis(),
        k.grace_waited.as_millis(),
        k.kill.as_millis(),
        start.elapsed().as_millis()
    );
    record_quit_integrity(k.remaining, &crate::config::config_dir());
    trace("socket_release_begin");
    let socket = release_notify_socket();
    trace("socket_release_end");
    let status = exit_status(k.remaining, &socket, workspace);
    let message = format!(
        "quit_phase: process exit — reason={reason_label} notify_socket={socket:?} workspace={workspace:?} status={status} total_ms={}",
        start.elapsed().as_millis()
    );
    if workspace == WorkspaceDurability::Unsaved {
        log::error!("{message}; the workspace was not saved");
    } else {
        log::info!("{message}");
    }
    if let Some(reply) = stop_reply_to {
        let delivered = crate::rpc::write_json_response(
            &reply,
            stop_reply(status, k.remaining, &socket, workspace),
        );
        log::info!("quit: host stop reply status={status} delivered={delivered}");
    }
    // The event log is written on its own thread; `_exit` would end the
    // process with the last events (the Shutdown focus segment) still queued.
    let flush_deadline = event_log_flush_deadline();
    if !crate::host::event_log::flush_global(flush_deadline) {
        log::warn!(
            "quit: event log writer did not confirm within {}ms; ending without it",
            flush_deadline.as_millis()
        );
    }
    // The log file is the only record of a quit; flush before the fd dies.
    trace("exit_call");
    log::logger().flush();
    // Deliberately *before* eframe's own `painter.destroy()`, which runs after
    // `on_exit` returns and which this call therefore skips: the wgpu device
    // and its worker threads are reclaimed by process teardown, exactly as on
    // the `plexi host stop` path that has always exited from mid-frame.
    end_process(status);
}

/// Longest the quit waits for the event-log writer to drain (stint 1537).
const EVENT_LOG_FLUSH_DEADLINE: Duration = Duration::from_millis(250);

/// `EVENT_LOG_FLUSH_DEADLINE`; tests running the real quit in a child set
/// their own so they never race the production deadline on a slow runner.
fn event_log_flush_deadline() -> Duration {
    #[cfg(test)]
    if let Some(ms) = std::env::var("PLEXI_TEST_EVENT_LOG_FLUSH_MS")
        .ok()
        .and_then(|ms| ms.parse().ok())
    {
        return Duration::from_millis(ms);
    }
    EVENT_LOG_FLUSH_DEADLINE
}

/// End the host process now, without C exit handlers (stint 1537).
///
/// `std::process::exit` runs the handlers registered with `atexit`; on macOS
/// one of them (CoreAnalytics) was measured waiting its full 300 ms on most
/// quits of a young host, with the window already hidden, which left a
/// window for "The application is not open anymore". `_exit` skips them.
/// What exit used to do for us is done before this:
/// - the integrity stamp and the `host stop` reply are written synchronously
///   (an atomic file write, or a socket write that is flushed and shut down);
/// - the notify socket is removed;
/// - the event log, written on its own `plexi-event-log` thread, is drained
///   by `exit_host` (`event_log::flush_global`, bounded);
/// - the logger is flushed by `exit_host` (its file writes are unbuffered);
/// - Rust's stdout buffer is flushed here, which `exit` did in
///   `std::rt::cleanup`. That cleanup only `try_lock`s stdout so a thread
///   that leaked the lock cannot hang exit; the public API has no try-lock,
///   so the flush runs on a helper thread and is given `STDOUT_FLUSH_DEADLINE`.
///   stderr is unbuffered.
///
/// Rust statics never run `Drop` at exit either way, and the host registers
/// no `atexit` handler of its own.
pub(crate) fn end_process(status: i32) -> ! {
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let flusher = std::thread::Builder::new()
        .name("plexi-exit-stdout".into())
        .spawn(move || {
            use std::io::Write as _;
            let _ = std::io::stdout().flush();
            let _ = done_tx.send(());
        });
    if flusher.is_ok() {
        let _ = done_rx.recv_timeout(STDOUT_FLUSH_DEADLINE);
    }
    // SAFETY: `_exit` takes no pointers and does not return.
    unsafe { libc::_exit(status) }
}

/// Longest the exit waits for Rust's stdout buffer to flush.
const STDOUT_FLUSH_DEADLINE: Duration = Duration::from_millis(20);

#[cfg(test)]
mod tests {
    /// Stint 1541: every quit trace line carries wall-clock milliseconds, and
    /// `exit_call` adds one summary line with hide-to-exit time.
    #[test]
    fn quit_traces_carry_wall_clock_ms_and_a_summary() {
        let capture = crate::testing::test_log::capture();
        for phase in ["quit_accepted", "window_hidden_native", "exit_call"] {
            trace(phase);
        }
        let lines = capture.lines();
        let traces: Vec<&String> = lines
            .iter()
            .filter(|l| l.starts_with("quit_trace:"))
            .collect();
        assert_eq!(traces.len(), 3, "{lines:?}");
        let field = |line: &str, key: &str| -> String {
            line.split_whitespace()
                .find_map(|part| part.strip_prefix(&format!("{key}=")))
                .unwrap_or_default()
                .to_string()
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        for line in &traces {
            let wall: u128 = field(line, "wall_ms").parse().expect("wall_ms is a number");
            assert!(now.abs_diff(wall) < 60_000, "{line}");
        }
        let summaries: Vec<&String> = lines
            .iter()
            .filter(|l| l.starts_with("quit_summary:"))
            .collect();
        assert_eq!(summaries.len(), 1, "{lines:?}");
        let hide: f64 = field(summaries[0], "hide_to_exit_ms")
            .parse()
            .expect("a hide time");
        assert!(hide >= 0.0, "{}", summaries[0]);
        assert!(field(summaries[0], "wall_ms").parse::<u128>().is_ok());
    }

    #[test]
    fn quit_summary_names_each_interval_or_none() {
        assert_eq!(
            quit_summary(7, 42, Some(1_000), Some(3_500), 13_500),
            "quit_summary: pid=7 wall_ms=42 hide_to_exit_ms=10.0 accepted_to_exit_ms=12.5"
        );
        assert_eq!(
            quit_summary(7, 42, None, None, 5),
            "quit_summary: pid=7 wall_ms=42 hide_to_exit_ms=none accepted_to_exit_ms=none"
        );
    }

    /// Stint 1459: the first host notices, once, when another process
    /// replaces or removes its socket file.
    #[cfg(unix)]
    #[test]
    fn a_replaced_socket_is_reported_once_at_warn() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notify.sock");
        std::fs::write(&path, b"").unwrap();
        let bound = BoundSocket {
            path: path.clone(),
            identity: identity(&path),
        };
        let reported = std::sync::atomic::AtomicBool::new(false);
        assert_eq!(ownership(&bound), SocketOwnership::Owned);
        assert_eq!(report_lost_socket(&bound, &reported), None);

        // A newcomer puts its own socket at the path. It is created while the
        // old file still exists, so its inode is necessarily a different one.
        let newcomer = dir.path().join("newcomer.sock");
        std::fs::write(&newcomer, b"new").unwrap();
        std::fs::rename(&newcomer, &path).unwrap();
        assert_eq!(ownership(&bound), SocketOwnership::Replaced);
        let message = report_lost_socket(&bound, &reported).expect("first loss is reported");
        assert!(message.contains("replaced"), "{message}");
        assert_eq!(report_lost_socket(&bound, &reported), None, "reported once");

        std::fs::remove_file(&path).unwrap();
        assert_eq!(ownership(&bound), SocketOwnership::Missing);
        let untracked = BoundSocket {
            path,
            identity: None,
        };
        assert_eq!(ownership(&untracked), SocketOwnership::Untracked);
    }

    use super::*;

    /// Marks that a C exit handler ran, for the `_exit` tests (stint 1537).
    extern "C" fn mark_atexit_ran() {
        if let Ok(dir) = std::env::var("PLEXI_TEST_EXIT_DIR") {
            let _ = std::fs::write(std::path::Path::new(&dir).join("atexit-ran"), "atexit ran");
        }
    }

    /// Run `exit_host(HostStop)` in a child copy of this test binary, with a
    /// C exit handler registered and the event log writing to `dir`, after
    /// queueing `events` events and a final marker. Nothing exits here.
    /// `writer_delay_ms` slows the event-log writer per event; `flush_ms`, when
    /// set, replaces the quit's event-log flush deadline for this child.
    fn run_exit_host_child(
        test: &str,
        dir: &std::path::Path,
        events: usize,
        writer_delay_ms: u64,
        flush_ms: Option<u128>,
    ) -> std::process::Output {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        if let Some(ms) = flush_ms {
            command.env("PLEXI_TEST_EVENT_LOG_FLUSH_MS", ms.to_string());
        }
        let child = command
            .args(["--exact", test, "--nocapture"])
            .env("PLEXI_TEST_EXIT_HOST_CHILD", "1")
            .env("PLEXI_TEST_EXIT_DIR", dir)
            .env("PLEXI_TEST_EXIT_EVENTS", events.to_string())
            .env("PLEXI_TEST_EVENT_LOG_DELAY_MS", writer_delay_ms.to_string())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run exit_host in a child");
        let pid = child.id();
        let output = child.wait_with_output().expect("child output");
        // The child's test profile fallback (`config_dir`), left by its quit.
        let _ = std::fs::remove_dir_all(format!("/tmp/plexi-t-{pid}"));
        output
    }

    /// The child half of the `_exit` tests: register the exit handler, queue
    /// the events, quit. Returns only when not running as the child.
    fn exit_host_child_if_asked() {
        if std::env::var("PLEXI_TEST_EXIT_HOST_CHILD").is_err() {
            return;
        }
        // Warn and error lines reach stdout, for the parent to read.
        crate::testing::test_log::install();
        let dir = std::path::PathBuf::from(std::env::var("PLEXI_TEST_EXIT_DIR").unwrap());
        assert_eq!(
            unsafe { libc::atexit(mark_atexit_ran) },
            0,
            "atexit registration"
        );
        std::fs::write(dir.join("registered"), "yes").unwrap();
        crate::host::event_log::init_global(dir.join("events.jsonl"));
        let events: usize = std::env::var("PLEXI_TEST_EXIT_EVENTS")
            .unwrap()
            .parse()
            .unwrap();
        for index in 0..=events {
            let title = if index == events {
                "FINAL-MARK".to_string()
            } else {
                format!("event-{index}")
            };
            crate::host::event_log::emit_scoped(
                crate::host::event_log::HostEvent::NotificationPosted {
                    id: index.to_string(),
                    title,
                    timestamp: crate::host::event_log::now_timestamp(),
                },
                None,
            );
        }
        exit_host(
            ExitReason::HostStop,
            WorkspaceDurability::Saved,
            Some(dir.join("stop-reply.json").display().to_string()),
        );
    }

    fn child_report(output: &std::process::Output) -> String {
        format!(
            "child status {:?}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
    }

    /// Stint 1537: `exit_host` ends in `_exit`, so a C exit handler (where
    /// macOS spent 300 ms) never runs, while the `host stop` reply it owes is
    /// still written and the exit status is the one it reports.
    #[test]
    fn exit_host_ends_without_running_exit_handlers() {
        exit_host_child_if_asked();
        let dir = tempfile::tempdir().unwrap();
        let output = run_exit_host_child(
            "app::quit::tests::exit_host_ends_without_running_exit_handlers",
            dir.path(),
            0,
            0,
            None,
        );
        let report = child_report(&output);
        assert!(
            dir.path().join("registered").exists(),
            "the child never registered its handler\n{report}"
        );
        assert!(
            !dir.path().join("atexit-ran").exists(),
            "a C exit handler ran: exit_host did not end in _exit\n{report}"
        );
        let reply: serde_json::Value = serde_json::from_slice(
            &std::fs::read(dir.path().join("stop-reply.json")).expect("the host stop reply"),
        )
        .expect("the reply is JSON");
        assert_eq!(
            reply["status"],
            output.status.code().expect("exit code"),
            "{reply}\n{report}"
        );
    }

    /// Stint 1537: the event log is written on its own thread, and `_exit`
    /// does not wait for it, so `exit_host` waits for the writer. Every event
    /// queued before the quit, the last one included, is in the file whole.
    #[test]
    fn exit_host_waits_for_the_event_log_writer() {
        exit_host_child_if_asked();
        let dir = tempfile::tempdir().unwrap();
        // Writing takes about 10 ms per event here, so the last of these is
        // still queued when cleanup ends unless the quit waits. The child's
        // flush deadline is load-scaled, so a slow runner cannot make the
        // quit give up before the writer finishes.
        let events = 15;
        let output = run_exit_host_child(
            "app::quit::tests::exit_host_waits_for_the_event_log_writer",
            dir.path(),
            events,
            10,
            Some(crate::testing::load_aware_timeout(Duration::from_secs(10)).as_millis()),
        );
        let report = child_report(&output);
        assert!(
            dir.path().join("registered").exists(),
            "the child never registered its handler\n{report}"
        );
        let text = std::fs::read_to_string(dir.path().join("events.jsonl")).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.len(),
            events + 1,
            "events written before the quit\n{report}"
        );
        let last: serde_json::Value =
            serde_json::from_str(lines[events]).expect("the last line is whole JSON");
        assert_eq!(last["title"], "FINAL-MARK", "{last}");
    }

    /// Stint 1537: a writer that never confirms cannot hold the quit. The
    /// writer is stuck on its first event; the quit gives up at its flush
    /// deadline, logs that, and still exits.
    #[test]
    fn exit_host_gives_up_on_a_stuck_event_log_writer() {
        exit_host_child_if_asked();
        let dir = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        let output = run_exit_host_child(
            "app::quit::tests::exit_host_gives_up_on_a_stuck_event_log_writer",
            dir.path(),
            1,
            600_000,
            Some(200),
        );
        let report = child_report(&output);
        assert!(output.status.code().is_some(), "the child exited\n{report}");
        assert!(
            started.elapsed() < crate::testing::load_aware_timeout(Duration::from_secs(20)),
            "the quit waited on the writer: {:?}\n{report}",
            started.elapsed()
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("quit: event log writer did not confirm within 200ms; ending without it"),
            "{report}"
        );
    }

    /// Stint 1542: the window is cosmetic and the quit is not. A native hide
    /// that fails is logged and the quit goes on to cleanup and `exit_host`.
    /// Runs the quit in a child copy of this test binary, since `exit_host`
    /// ends the process; the child's quit trace shows how far it got.
    #[test]
    fn a_failed_window_hide_does_not_stop_the_quit() {
        const CHILD: &str = "PLEXI_TEST_HIDE_FAILS_CHILD";
        if std::env::var(CHILD).is_ok() {
            let mut h = crate::testing::HostHarness::new();
            h.add_test_pane();
            HIDE_RESULT.with(|slot| slot.set(Some(Err("NSView has no window"))));
            h.app.request_quit(ExitReason::WindowClosed);
            for _ in 0..10 {
                h.app.quit_coordinator.clear_retry();
                h.app
                    .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Ok(()));
            }
            panic!("the quit never reached exit_host");
        }
        let mut running = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "app::quit::tests::a_failed_window_hide_does_not_stop_the_quit",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run the quit in a child");
        let pid = running.id();
        // A hung child fails this test instead of hanging the run.
        let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(20));
        let finished = loop {
            if running.try_wait().unwrap().is_some() {
                break true;
            }
            if Instant::now() >= deadline {
                let _ = running.kill();
                break false;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let child = running.wait_with_output().unwrap();
        let _ = std::fs::remove_dir_all(format!("/tmp/plexi-t-{pid}"));
        let stderr = String::from_utf8_lossy(&child.stderr);
        assert!(finished, "the quit hung after a failed hide:\n{stderr}");
        assert!(
            stderr.contains("phase=exit_call"),
            "the quit did not reach exit_host (status {:?}):\n{stderr}",
            child.status
        );
        assert_eq!(
            child.status.code(),
            Some(0),
            "a saved quit after a failed hide exits 0:\n{stderr}"
        );
        assert!(stderr.contains("phase=window_hide_failed"), "{stderr}");
        let stdout = String::from_utf8_lossy(&child.stdout);
        assert!(
            stdout.contains(
                "quit: cannot hide native window: NSView has no window; continuing the quit"
            ),
            "the failure is logged with its text:\n{stdout}"
        );
    }

    /// Stint 1542: after a failed native hide the window may still be on
    /// screen, so the watchdog must report a stalled cleanup as `[FREEZE]`,
    /// not as an expected `[SHUTDOWN_WAIT]`. A confirmed hide still is one.
    #[test]
    fn a_failed_window_hide_leaves_the_watchdog_on_freeze() {
        for (hide, expect_wait) in [(Err("NSView has no window"), false), (Ok(true), true)] {
            let mut h = crate::testing::HostHarness::new();
            h.add_test_pane();
            HIDE_RESULT.with(|slot| slot.set(Some(hide)));
            h.app.request_quit(ExitReason::WindowClosed);
            for _ in 0..10 {
                if h.app.quit_coordinator.hide_requested() {
                    break;
                }
                h.app.quit_coordinator.clear_retry();
                h.app
                    .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Ok(()));
            }
            HIDE_RESULT.with(|slot| slot.set(None));
            assert!(h.app.quit_coordinator.hide_requested(), "the hide step ran");
            h.app.begin_quit_cleanup();
            assert_eq!(
                crate::platform::logging::expects_shutdown_wait(&h.app.ui_phase),
                expect_wait,
                "hide {hide:?}"
            );
        }
    }

    #[test]
    fn quit_cleanup_replaces_the_stale_workspace_save_heartbeat_phase() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.request_quit(ExitReason::HostStop);
        assert!(h.app.service_quit(&eframe::Frame::_new_kittest()));
        assert!(h.app.quit_coordinator.hide_requested());
        // Model the next logic pass after the backend has applied hide,
        // stopping before exit_host so the watchdog phase is observable.
        h.app.begin_quit_cleanup();
        assert_eq!(
            crate::platform::logging::current_ui_phase_name(&h.app.ui_phase),
            "shutdown_cleanup"
        );
    }

    #[test]
    fn quit_save_is_not_duplicated_by_a_second_quit_path() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.request_quit(ExitReason::WindowClosed);
        assert!(h.app.persist_quit_state());
        let path = crate::workspace::workspace_path_in(h.profile_dir());
        let first = std::fs::metadata(&path).unwrap().modified().unwrap();
        // Distinct filesystem timestamps even on coarse filesystems.
        std::thread::sleep(Duration::from_millis(20));
        assert!(h.app.persist_quit_state());
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            first,
            "an unchanged quit must not write another workspace generation"
        );
    }

    #[test]
    fn quit_save_persists_a_later_dirty_mutation() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.request_quit(ExitReason::WindowClosed);
        assert!(h.app.persist_quit_state());
        h.app.sidebar_visible = !h.app.sidebar_visible;
        h.app.mark_workspace_dirty();
        assert!(h.app.persist_quit_state());
        let saved = crate::workspace::WorkspaceFile::load_from(h.profile_dir()).unwrap();
        assert_eq!(saved.sidebar_visible, h.app.sidebar_visible);
        assert!(!h.app.workspace_dirty);
    }

    #[test]
    fn quit_save_failure_stays_dirty_and_retries() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.request_quit(ExitReason::WindowClosed);
        let path = crate::workspace::workspace_path_in(h.profile_dir());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"invalid workspace").unwrap();
        assert!(!h.app.persist_quit_state());
        assert!(
            h.app.workspace_dirty,
            "a failed final save must remain pending even if initially clean"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid workspace");
        std::fs::remove_file(&path).unwrap();
        assert!(h.app.persist_quit_state());
        assert!(crate::workspace::WorkspaceFile::load_from(h.profile_dir()).is_ok());
        assert!(!h.app.workspace_dirty);
    }

    #[test]
    fn quit_confirmation_does_not_close_after_failed_save() {
        let mut h = crate::testing::HostHarness::new();
        let pane = h.add_test_pane();
        let path = crate::workspace::workspace_path_in(h.profile_dir());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"invalid workspace").unwrap();
        h.app.windows[0]
            .panes
            .get_mut(&pane)
            .unwrap()
            .set_agent(Some(crate::protocol::PaneAgentState {
                pane_id: pane,
                state: crate::protocol::AgentState::Working,
                agent: "test-agent".into(),
                detail: None,
                session_id: None,
            }));
        assert!(!h.app.prepare_user_quit());
        let output = h.app.ctx.clone().run_ui(egui::RawInput::default(), |_| {
            h.app.resolve_quit_confirmation(true);
        });
        assert_eq!(
            h.app.quit_coordinator.reason(),
            Some(ExitReason::WindowClosed)
        );
        assert!(
            !output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close)),
            "failed durability must not allow eframe to destroy the window"
        );
    }

    #[test]
    fn quit_reasons_preserve_window_host_stop_and_update_grace() {
        assert_eq!(ExitReason::WindowClosed.grace(), Duration::from_millis(500));
        assert_eq!(ExitReason::HostStop.grace(), Duration::from_secs(2));
        assert_eq!(ExitReason::UpdateRestart.grace(), Duration::from_secs(2));
    }

    #[test]
    fn quit_coordinator_retries_before_hiding_or_scheduling_restart() {
        for reason in [
            ExitReason::WindowClosed,
            ExitReason::HostStop,
            ExitReason::UpdateRestart,
        ] {
            let mut h = crate::testing::HostHarness::new();
            h.add_test_pane();
            let path = crate::workspace::workspace_path_in(h.profile_dir());
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"invalid workspace").unwrap();
            h.app.request_quit(reason);
            assert!(!h
                .app
                .service_quit_with_restart(&eframe::Frame::_new_kittest(), || {
                    panic!("restart must wait for durability")
                }));
            assert!(h.app.workspace_dirty);
            assert!(!h.app.quit_coordinator.hide_requested());
            assert_eq!(h.app.quit_coordinator.reason(), Some(reason));
            std::fs::remove_file(&path).unwrap();
            h.app.quit_coordinator.clear_retry();
            let scheduled = std::cell::Cell::new(0);
            assert!(h
                .app
                .service_quit_with_restart(&eframe::Frame::_new_kittest(), || {
                    assert!(path.is_file());
                    scheduled.set(scheduled.get() + 1);
                    Ok(())
                }));
            assert!(h.app.quit_coordinator.hide_requested());
            assert!(!h.app.workspace_dirty);
            assert_eq!(
                scheduled.get(),
                usize::from(reason == ExitReason::UpdateRestart)
            );
        }
    }

    /// Makes the profile's workspace folder unwritable for the guard's life;
    /// dropping it, even in a panic, makes the folder writable again.
    #[cfg(unix)]
    struct UnwritableWorkspaceFolder(std::path::PathBuf);

    #[cfg(unix)]
    impl UnwritableWorkspaceFolder {
        fn new(profile: &std::path::Path) -> Self {
            use std::os::unix::fs::PermissionsExt as _;
            let path = crate::workspace::workspace_path_in(profile);
            let folder = path.parent().unwrap().to_path_buf();
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o500)).unwrap();
            Self(folder)
        }
    }

    #[cfg(unix)]
    impl Drop for UnwritableWorkspaceFolder {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
        }
    }

    /// Stint 1539: the real `on_exit` (Dock Quit, logout) with a save that
    /// keeps failing ends the process within its bound and reports the
    /// workspace unsaved, instead of retrying on the main thread forever.
    /// `on_exit` ends in `exit_host`, so it runs in a child copy of this test
    /// binary; a hung child is killed and fails the test.
    #[cfg(unix)]
    #[test]
    fn on_exit_with_a_failing_save_exits_unsaved_within_its_bound() {
        const CHILD: &str = "PLEXI_TEST_ON_EXIT_CHILD";
        if std::env::var(CHILD).is_ok() {
            let mut h = crate::testing::HostHarness::new();
            h.add_test_pane();
            h.app.workspace_dirty = true;
            let _unwritable = UnwritableWorkspaceFolder::new(h.profile_dir());
            eframe::App::on_exit(&mut h.app);
            panic!("on_exit returned instead of exiting");
        }
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "app::quit::tests::on_exit_with_a_failing_save_exits_unsaved_within_its_bound",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run on_exit in a child");
        let pid = child.id();
        // Five attempts one second apart, plus cleanup.
        let deadline =
            std::time::Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(20));
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                break None;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let output = child.wait_with_output().unwrap();
        let _ = std::fs::remove_dir_all(format!("/tmp/plexi-t-{pid}"));
        let stderr = String::from_utf8_lossy(&output.stderr);
        let status = status.unwrap_or_else(|| panic!("on_exit never ended the process:\n{stderr}"));
        assert_eq!(
            status.code(),
            Some(1),
            "an unsaved exit reports status 1:\n{stderr}"
        );
        assert!(stderr.contains("phase=exit_call"), "{stderr}");
    }

    /// Stint 1539: a backend teardown (`on_exit`) whose save keeps failing
    /// returns within its bound and records the workspace unsaved, instead of
    /// retrying forever on the main thread. The workspace's folder is made
    /// unwritable; the harness runs on its own thread so a hang shows up as a
    /// timeout here, not a stuck test run.
    #[cfg(unix)]
    #[test]
    fn teardown_save_failure_is_bounded_and_records_unsaved() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut h = crate::testing::HostHarness::new();
            h.add_test_pane();
            h.app.workspace_dirty = true;
            let _unwritable = UnwritableWorkspaceFolder::new(h.profile_dir());
            let started = std::time::Instant::now();
            let durability = h.app.persist_for_teardown(Duration::from_millis(20));
            let elapsed = started.elapsed();
            let failures = h.app.quit_coordinator.save_failures();
            let _ = tx.send((durability, elapsed, failures));
        });
        let (durability, elapsed, failures) = rx
            .recv_timeout(crate::testing::load_aware_timeout(Duration::from_secs(10)))
            .expect("the teardown save returns instead of retrying forever");
        assert_eq!(durability, WorkspaceDurability::Unsaved);
        assert_eq!(failures, MAX_QUIT_SAVE_ATTEMPTS, "it tried the full budget");
        assert!(
            elapsed < crate::testing::load_aware_timeout(Duration::from_secs(2)),
            "{elapsed:?}"
        );
    }

    /// Drives a desktop quit (window close) against an unwritable-as-valid
    /// workspace file and returns the harness after the retry budget is
    /// exhausted, with the "Quit without saving" dialog up. (`host stop` has
    /// no dialog: see `host_stop_failing_save_exits_unsaved_without_a_dialog`.)
    fn quit_with_unsaveable_workspace() -> (crate::testing::HostHarness, std::path::PathBuf) {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        let path = crate::workspace::workspace_path_in(h.profile_dir());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"invalid workspace").unwrap();
        h.app.request_quit(ExitReason::WindowClosed);
        for attempt in 1..=MAX_QUIT_SAVE_ATTEMPTS {
            h.app.quit_coordinator.clear_retry();
            assert!(!h
                .app
                .service_quit_with_restart(&eframe::Frame::_new_kittest(), || {
                    panic!("restart must wait for durability")
                }));
            assert_eq!(h.app.quit_coordinator.save_failures(), attempt);
            assert_eq!(
                h.app.quit_coordinator.save_failure().is_some(),
                attempt == MAX_QUIT_SAVE_ATTEMPTS,
                "the error is surfaced only once the budget is spent (attempt {attempt})"
            );
        }
        (h, path)
    }

    /// Stint 1493 critic blocker 2 and stint 1505: in "Workspace could not be
    /// saved", Tab then Space answers nothing and Tab then Enter cancels (the
    /// safe default); neither quits without saving.
    #[test]
    fn quit_save_failure_tab_then_enter_does_not_quit() {
        for key in [egui::Key::Enter, egui::Key::Space] {
            let (mut h, path) = quit_with_unsaveable_workspace();
            h.press_key(egui::Key::Tab, egui::Modifiers::NONE);
            h.press_key(key, egui::Modifiers::NONE);
            h.hidden_frame();
            if key == egui::Key::Space {
                assert!(
                    h.app.quit_coordinator.save_failure().is_some(),
                    "Tab then Space must not answer the dialog"
                );
            } else {
                assert!(
                    h.app.quit_coordinator.save_failure().is_none()
                        && !h.app.quit_coordinator.is_active(),
                    "Tab then Enter cancels"
                );
            }
            assert!(
                !h.app.quit_coordinator.save_waived(),
                "nothing quit without saving"
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"invalid workspace");
        }
    }

    #[test]
    fn quit_save_retry_is_bounded_and_surfaces_the_cause_once() {
        let (mut h, path) = quit_with_unsaveable_workspace();
        let cause = h.app.quit_coordinator.save_failure().unwrap().to_owned();
        assert!(
            cause.contains("invalid workspace"),
            "cause names the failure: {cause}"
        );
        assert!(h.app.confirm_close_wanted(), "dialog is requested");
        assert!(
            !h.app.quit_coordinator.blocks_input(),
            "the dialog must be usable"
        );
        // No further save attempt while the dialog waits.
        for _ in 0..3 {
            assert!(!h
                .app
                .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Ok(())));
        }
        assert_eq!(
            h.app.quit_coordinator.save_failures(),
            MAX_QUIT_SAVE_ATTEMPTS
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid workspace");
    }

    #[test]
    fn quit_without_saving_proceeds_and_preserves_the_file_on_disk() {
        let (mut h, path) = quit_with_unsaveable_workspace();
        h.app.resolve_quit_save_failure(true);
        assert!(!h.app.confirm_close_wanted());
        assert!(h
            .app
            .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Ok(())));
        assert!(
            h.app.quit_coordinator.hide_requested(),
            "quit proceeds to hide and exit"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid workspace");
    }

    /// Stint 1127: quitting without saving is not a clean exit.
    #[test]
    fn quit_without_saving_is_not_reported_as_a_clean_exit() {
        let (mut h, _path) = quit_with_unsaveable_workspace();
        h.app.resolve_quit_save_failure(true);
        assert!(h
            .app
            .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Ok(())));
        assert_eq!(
            h.app.quit_workspace_durability(),
            WorkspaceDurability::Unsaved
        );

        let mut saved = crate::testing::HostHarness::new();
        saved.add_test_pane();
        saved.app.request_quit(ExitReason::HostStop);
        assert!(saved
            .app
            .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Ok(())));
        assert_eq!(
            saved.app.quit_workspace_durability(),
            WorkspaceDurability::Saved
        );

        assert_eq!(
            exit_status(0, &SocketRelease::Removed, WorkspaceDurability::Saved),
            0
        );
        assert_eq!(
            exit_status(0, &SocketRelease::Removed, WorkspaceDurability::Unsaved),
            1
        );
        assert_eq!(
            exit_status(1, &SocketRelease::Removed, WorkspaceDurability::Saved),
            1
        );
        assert_eq!(
            exit_status(
                0,
                &SocketRelease::Failed("fixture".into()),
                WorkspaceDurability::Saved
            ),
            1
        );
    }

    fn harness_with_corrupt_workspace(profile: &Path) -> (crate::testing::HostHarness, PathBuf) {
        let path = crate::workspace::workspace_path_in(profile);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{corrupt workspace").unwrap();
        let h = crate::testing::HostHarness::from_profile_dir(profile.to_path_buf());
        (h, path)
    }

    /// Stint 1127: a workspace file that exists but cannot be restored is
    /// surfaced with its path, and the fresh session never saves over it.
    #[test]
    fn unusable_workspace_at_boot_blocks_every_save_and_is_surfaced() {
        let profile = tempfile::tempdir().unwrap();
        let (mut h, path) = harness_with_corrupt_workspace(profile.path());
        let block = h
            .app
            .workspace_save_block()
            .expect("boot must record the unusable workspace");
        assert!(block.contains(&path.display().to_string()), "{block}");
        h.add_test_pane();
        h.app.mark_workspace_dirty();
        assert!(
            !h.app.save_workspace_now(),
            "a blocked session must not save"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"{corrupt workspace");

        // A host integrity notice ignores the notifications master switch.
        h.app.notifications_enabled = false;
        h.app.surface_workspace_load_failure();
        assert!(
            h.app.pending_notifications.iter().any(|notification| {
                notification.title == "Workspace not restored"
                    && notification.body.contains(&path.display().to_string())
            }),
            "the load failure must be surfaced with the file path even with notifications off"
        );
    }

    /// Stint 1127: a valid workspace whose plugin migration fails must not be
    /// replaced by the fresh session the host falls back to.
    #[test]
    fn failed_plugin_migration_at_boot_never_saves_over_the_workspace() {
        let profile = tempfile::tempdir().unwrap();
        {
            let mut first =
                crate::testing::HostHarness::from_profile_dir(profile.path().to_path_buf());
            first.add_test_pane();
            assert!(first.app.save_workspace_now());
        }
        let path = crate::workspace::workspace_path_in(profile.path());
        let original = std::fs::read(&path).unwrap();
        // Both trees present is a classified migration failure.
        std::fs::create_dir_all(profile.path().join("apps/legacy")).unwrap();
        std::fs::create_dir_all(profile.path().join("plugins")).unwrap();
        let mut h = crate::testing::HostHarness::from_profile_dir(profile.path().to_path_buf());
        let block = h
            .app
            .workspace_save_block()
            .expect("a failed plugin migration blocks saves");
        assert!(
            block.contains("fix the plugin migration problem") && !block.contains("move the file"),
            "an intact file must not be blamed: {block}"
        );
        h.add_test_pane();
        h.app.mark_workspace_dirty();
        assert!(!h.app.save_workspace_now());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    /// Stint 1127: a block cannot clear by retrying, so the quit asks at once.
    #[test]
    fn blocked_workspace_quit_asks_on_the_first_failure() {
        let profile = tempfile::tempdir().unwrap();
        let (mut h, path) = harness_with_corrupt_workspace(profile.path());
        h.add_test_pane();
        h.app.request_quit(ExitReason::WindowClosed);
        assert!(!h
            .app
            .service_quit_with_restart(&eframe::Frame::_new_kittest(), || {
                panic!("restart must wait for durability")
            }));
        assert_eq!(h.app.quit_coordinator.save_failures(), 1);
        let cause = h
            .app
            .quit_coordinator
            .save_failure()
            .expect("a permanent block is not retried")
            .to_owned();
        assert!(cause.contains(&path.display().to_string()), "{cause}");
        assert!(
            h.app.confirm_close_wanted(),
            "the save-failure dialog is requested"
        );
        assert!(!h.app.quit_coordinator.hide_requested());
        assert_eq!(std::fs::read(&path).unwrap(), b"{corrupt workspace");
    }

    /// Stint 1127: `plexi host stop` (IPC `Shutdown`) on a session whose
    /// workspace never restored has nobody to answer the save dialog. It
    /// exits unsaved instead of hanging until the CLI kills it, and the file
    /// is left as found. The file parses but fails validation, so the boot
    /// block, not `save_to`'s refusal of unparseable JSON, is what is tested.
    #[test]
    fn host_stop_over_ipc_on_a_blocked_workspace_exits_unsaved_without_asking() {
        let profile = tempfile::tempdir().unwrap();
        {
            let mut first =
                crate::testing::HostHarness::from_profile_dir(profile.path().to_path_buf());
            first.add_test_pane();
            assert!(first.app.save_workspace_now());
        }
        let path = crate::workspace::workspace_path_in(profile.path());
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["active_context"] = serde_json::json!(999);
        let original = serde_json::to_vec_pretty(&value).unwrap();
        std::fs::write(&path, &original).unwrap();

        let mut h = crate::testing::HostHarness::from_profile_dir(profile.path().to_path_buf());
        let block = h
            .app
            .workspace_save_block()
            .expect("a workspace that fails validation blocks saves");
        assert!(block.contains("invalid workspace"), "{block}");
        h.add_test_pane();
        h.app.mark_workspace_dirty();
        h.inject_ipc(crate::protocol::HostRequest::Shutdown {
            response_file: Some("host-stop-reply".to_string()),
        });
        h.hidden_frame();
        assert_eq!(h.app.quit_coordinator.reason(), Some(ExitReason::HostStop));
        assert_eq!(
            h.app.quit_coordinator.stop_reply(),
            Some("host-stop-reply"),
            "the reply token reaches exit_host (stint 1430)"
        );
        assert!(
            !h.app.confirm_close_wanted(),
            "host stop must not raise the dialog"
        );
        assert!(h.app.quit_coordinator.save_failure().is_none());
        assert!(
            h.app.quit_coordinator.hide_requested(),
            "host stop must proceed to exit"
        );
        assert_eq!(
            h.app.quit_workspace_durability(),
            WorkspaceDurability::Unsaved
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    /// Stint 1430: the reply `exit_host` sends `plexi host stop` carries the
    /// same status the process exits with, and why it is not clean.
    #[test]
    fn stop_reply_carries_the_exit_status_and_its_cause() {
        let unsaved = stop_reply(
            exit_status(0, &SocketRelease::Removed, WorkspaceDurability::Unsaved),
            0,
            &SocketRelease::Removed,
            WorkspaceDurability::Unsaved,
        );
        assert_eq!(unsaved["status"], 1);
        assert_eq!(unsaved["workspace"], "unsaved");
        let clean = stop_reply(
            exit_status(0, &SocketRelease::Removed, WorkspaceDurability::Saved),
            0,
            &SocketRelease::Removed,
            WorkspaceDurability::Saved,
        );
        assert_eq!(clean["status"], 0);
        assert_eq!(clean["workspace"], "saved");
        assert_eq!(clean["socket_released"], true);
        let leftovers = stop_reply(
            exit_status(2, &SocketRelease::Removed, WorkspaceDurability::Saved),
            2,
            &SocketRelease::Removed,
            WorkspaceDurability::Saved,
        );
        assert_eq!(leftovers["status"], 1);
        assert_eq!(leftovers["remaining_processes"], 2);
    }

    /// Stint 1540: `plexi host stop` has nobody to answer a dialog. A save
    /// that keeps failing for a reason other than a boot-time block is
    /// retried the bounded number of times, then the quit goes on unsaved:
    /// no dialog, and the exit records Unsaved for the
    /// stop reply. The file on disk is not rewritten.
    #[test]
    fn host_stop_failing_save_exits_unsaved_without_a_dialog() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        let path = crate::workspace::workspace_path_in(h.profile_dir());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"invalid workspace").unwrap();
        assert!(
            h.app.workspace_load_failure.is_none(),
            "not a boot-time block"
        );
        h.app.request_quit(ExitReason::HostStop);
        let mut proceeded = false;
        for _ in 0..MAX_QUIT_SAVE_ATTEMPTS {
            h.app.quit_coordinator.clear_retry();
            proceeded = h
                .app
                .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Ok(()));
            assert!(!h.app.confirm_close_wanted(), "no dialog for host stop");
            assert!(
                h.app.quit_coordinator.save_failure().is_none(),
                "no dialog cause set"
            );
        }
        assert!(proceeded, "the quit goes on after the last failed save");
        assert_eq!(
            h.app.quit_coordinator.save_failures(),
            MAX_QUIT_SAVE_ATTEMPTS
        );
        assert_eq!(
            h.app.quit_workspace_durability(),
            WorkspaceDurability::Unsaved
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid workspace");
    }

    /// Stint 1540: a host stop whose save fails and then succeeds before the
    /// cap is not waived: it exits with the workspace saved.
    #[test]
    fn host_stop_save_that_recovers_before_the_cap_is_saved() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        let path = crate::workspace::workspace_path_in(h.profile_dir());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"invalid workspace").unwrap();
        h.app.request_quit(ExitReason::HostStop);
        for _ in 0..2 {
            h.app.quit_coordinator.clear_retry();
            assert!(!h
                .app
                .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Ok(())));
        }
        // The cause clears (the file becomes writable as a workspace).
        std::fs::remove_file(&path).unwrap();
        h.app.quit_coordinator.clear_retry();
        assert!(h
            .app
            .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Ok(())));
        assert!(
            !h.app.quit_coordinator.save_waived(),
            "a recovered save is not waived"
        );
        assert_eq!(
            h.app.quit_workspace_durability(),
            WorkspaceDurability::Saved
        );
    }

    #[test]
    fn cancelling_after_save_failures_keeps_the_window() {
        let (mut h, path) = quit_with_unsaveable_workspace();
        h.app.resolve_quit_save_failure(false);
        assert!(!h.app.quit_coordinator.is_active());
        assert!(!h.app.quit_coordinator.is_active());
        assert!(!h.app.confirm_close_wanted());
        assert!(!h
            .app
            .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Ok(())));
        assert!(!h.app.quit_coordinator.hide_requested());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid workspace");
    }

    #[test]
    fn save_failure_dialog_is_keyboard_driven() {
        let (mut h, _path) = quit_with_unsaveable_workspace();
        h.run_frames(2);
        assert!(h
            .app
            .focus_stack
            .contains(&crate::app::focus::FocusKind::ConfirmClose));
        h.press_key(egui::Key::Escape, egui::Modifiers::NONE);
        assert!(
            !h.app.quit_coordinator.is_active(),
            "Esc cancels and keeps the window"
        );
        let (mut h, _path) = quit_with_unsaveable_workspace();
        h.run_frames(2);
        h.app.arm_destructive_dialogs_for_test();
        h.press_key(egui::Key::Q, egui::Modifiers::NONE);
        assert!(
            h.app.quit_coordinator.save_waived(),
            "Q quits without saving"
        );
        assert!(h.app.quit_coordinator.save_failure().is_none());
    }

    /// A real frame with an update restart pending goes through the injected
    /// scheduler: the harness records exactly one request and never reads an
    /// installed receipt or schedules a real restart.
    #[test]
    fn update_restart_frame_records_one_request_through_the_injected_scheduler() {
        test_restart_scheduler::take();
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.quit_coordinator.queue_update_restart();
        h.run_frames(1);
        assert_eq!(test_restart_scheduler::take(), 1);
        assert_eq!(
            h.app.quit_coordinator.reason(),
            Some(ExitReason::UpdateRestart)
        );
        assert!(h.app.quit_coordinator.restart_scheduled());
        assert!(h.app.quit_coordinator.hide_requested());
        // A second pass after the hide would call `exit_host`; stop here.
    }

    #[test]
    fn failed_update_schedule_keeps_window_and_allows_a_fresh_quit() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.request_quit(ExitReason::UpdateRestart);
        assert!(!h
            .app
            .service_quit_with_restart(&eframe::Frame::_new_kittest(), || Err(
                "fixture error".into()
            )));
        assert!(!h.app.quit_coordinator.is_active());
        assert!(!h.app.quit_coordinator.hide_requested());
        assert!(!h.app.quit_coordinator.is_active());
        assert!(crate::workspace::WorkspaceFile::load_from(h.profile_dir()).is_ok());
        h.app.request_quit(ExitReason::WindowClosed);
        assert!(h.app.service_quit(&eframe::Frame::_new_kittest()));
        assert!(h.app.quit_coordinator.hide_requested());
    }

    #[test]
    fn duplicate_desktop_close_is_cancelled_and_coordinated_once() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.config.confirm_quit = Some(false);
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::Q,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
        h.frame(input);
        assert_eq!(
            h.app.quit_coordinator.reason(),
            Some(ExitReason::WindowClosed)
        );
        let output = h.last_full_output.as_ref().unwrap();
        let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::CancelClose)));
        assert!(!commands
            .iter()
            .any(|command| matches!(command, egui::ViewportCommand::Close)));
        h.hidden_frame(); // saves and hides; cleanup needs a subsequent pass
        assert!(h.app.quit_coordinator.saved());
        assert!(h.app.quit_coordinator.hide_requested());
    }

    #[test]
    fn host_stop_does_not_debounce_save_before_its_final_save() {
        let mut h = crate::testing::HostHarness::new();
        h.add_test_pane();
        h.app.workspace_dirty = true;
        h.app.last_workspace_save = Instant::now() - Duration::from_secs(2);
        h.inject_ipc(crate::protocol::HostRequest::Shutdown {
            response_file: None,
        });
        h.hidden_frame(); // coordinator saves and hides, but does not exit yet
        let path = crate::workspace::workspace_path_in(h.profile_dir());
        assert!(path.exists());
        assert!(
            !path.with_extension("json.prev").exists(),
            "the debounce must not create an extra generation before final save"
        );
        assert!(h.app.quit_coordinator.hide_requested());
    }

    #[cfg(unix)]
    #[test]
    fn restart_waits_for_the_detached_coordinator_before_hiding() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixStream;

        for coordinator_ready in [true, false] {
            let mut h = crate::testing::HostHarness::new();
            h.add_test_pane();
            let token = crate::rpc::response_file_in(h.profile_dir(), "restart-handoff", "json");
            let (client, server) = UnixStream::pair().unwrap();
            let timeout = crate::testing::load_aware_timeout(Duration::from_secs(2));
            client.set_read_timeout(Some(timeout)).unwrap();
            assert!(crate::rpc::ReplyRegistry::register(token.clone(), server));
            h.inject_ipc(crate::protocol::HostRequest::ShutdownForRestart {
                response_file: token,
            });
            h.hidden_frame();
            assert!(h.app.quit_coordinator.is_active());
            assert!(!h.app.quit_coordinator.hide_requested());

            let mut client = BufReader::new(client);
            let mut line = String::new();
            client.read_line(&mut line).unwrap();
            let reply: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(reply["ok"], true);
            assert_eq!(reply["pid"], std::process::id());
            if coordinator_ready {
                client
                    .get_mut()
                    .write_all(&crate::rpc::RESTART_READY)
                    .unwrap();
            }
            drop(client);

            let deadline = Instant::now() + timeout;
            while h.app.quit_coordinator.restart_handoff_pending() {
                assert!(
                    Instant::now() < deadline,
                    "restart handoff was not serviced"
                );
                h.hidden_frame();
                std::thread::yield_now();
            }
            assert_eq!(h.app.quit_coordinator.hide_requested(), coordinator_ready);
            assert_eq!(h.app.quit_coordinator.is_active(), coordinator_ready);
        }
    }

    fn bind_fake_socket(dir: &Path, name: &str) -> BoundSocket {
        let path = dir.join(name);
        std::fs::write(&path, b"").expect("create stand-in socket");
        BoundSocket {
            path: path.clone(),
            identity: identity(&path),
        }
    }

    #[test]
    fn release_removes_the_socket_this_process_bound() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bound = bind_fake_socket(dir.path(), "notify.sock");

        assert_eq!(release(&bound), SocketRelease::Removed);
        assert!(
            !bound.path.exists(),
            "quitting must leave no socket for a CLI to connect to"
        );
    }

    #[test]
    fn release_leaves_a_socket_another_host_has_rebound() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bound = bind_fake_socket(dir.path(), "notify.sock");
        // A successor host binds the same path while we are quitting. Renamed
        // into place rather than recreated: ext4 hands the just-freed inode
        // straight back, which would make the successor indistinguishable
        // from ours.
        let successor = dir.path().join("successor.sock");
        std::fs::write(&successor, b"").expect("successor binds");
        std::fs::rename(&successor, &bound.path).expect("successor takes the path");

        assert_eq!(release(&bound), SocketRelease::Reused);
        assert!(
            bound.path.exists(),
            "teardown must not unbind a socket a different host now owns"
        );
    }

    #[test]
    fn release_is_quiet_when_the_socket_is_already_gone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bound = bind_fake_socket(dir.path(), "notify.sock");
        std::fs::remove_file(&bound.path).expect("remove");

        assert_eq!(release(&bound), SocketRelease::Missing);
    }

    /// The only test that touches the process-wide binding, because the
    /// `OnceLock` it sets is visible to every other test in this binary.
    #[test]
    fn a_recorded_socket_is_the_one_teardown_releases() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("notify.sock");
        std::fs::write(&path, b"").expect("create stand-in socket");

        record_notify_socket(&path);

        assert_eq!(release_notify_socket(), SocketRelease::Removed);
        assert!(!path.exists());
        // Idempotent: a second quit path finds nothing left to do.
        assert_eq!(release_notify_socket(), SocketRelease::Missing);
    }
}
