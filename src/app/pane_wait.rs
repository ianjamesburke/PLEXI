//! Host-side blocking waits for the pane orchestration verbs.
//!
//! An agent that shells out to `plexi pane slot wait` wants one round trip
//! that returns when a condition holds, not a poll loop it has to write
//! itself. The CLI therefore sends a single request carrying its own deadline
//! and parks on the response file; the host keeps the request here and answers
//! it from the code path that changes the watched state.
//!
//! Three rules keep that honest:
//!
//! 1. **Level-triggered, never edge-triggered.** The handler checks the
//!    current value before parking a waiter, so a caller that arms the wait
//!    after the value already settled returns immediately instead of hanging
//!    until its timeout. Nothing in the protocol lets a caller guarantee it
//!    armed the wait first.
//! 2. **The host owns the deadline.** [`PlexiApp::service_pending_slot_waits`]
//!    expires a waiter at the timeout the caller asked for, which is strictly
//!    below the CLI's own poll window. The caller therefore sees the host's
//!    typed reason — and the distinct timeout exit code — rather than a
//!    generic client-side timeout with nothing in the host log.
//! 3. **Servicing runs from `App::logic`, never `App::ui`.** eframe skips `ui`
//!    entirely while the window is minimized or fully occluded, so a wait
//!    serviced from `ui` would neither complete nor expire on an occluded
//!    host. Guarded by `slot_wait_completes_while_window_hidden`.
//!
//! `plexi pane new --agent` parks here too. Its watched state is the pane's
//! agent self-report rather than a slot file, but the shape is identical: the
//! spawn's response file is withheld until the agent running in the new pane
//! reports itself idle, so a pane id on the caller's stdout means "this pane
//! exists and is ready to receive a brief" and nothing weaker.
//!
//! `plexi pane send --submit` is the third resident. It is not waiting on one
//! value but driving a short sequence — settle, Enter, confirm — that a caller
//! can only get right with host-side visibility into PTY timing and the pane's
//! rendered grid. Its state machine lives in [`PendingSubmit`].

use std::time::{Duration, Instant};

use serde_json::Value;

/// Upper bound for caller-supplied wait timeouts (24 h), applied before any
/// `Duration::from_secs_f64` conversion. Finite f64 values beyond `Duration`'s
/// range make that constructor panic, so an unvalidated `--timeout 1e308`
/// would crash the host from CLI input; every timeout ingress rejects values
/// above this bound instead.
pub(crate) const MAX_WAIT_TIMEOUT_SECS: f64 = 86_400.0;

use super::PlexiApp;
use crate::protocol::AgentState;

/// No PTY event for this long means the pane has finished redrawing and is
/// ready to receive Enter.
const SUBMIT_QUIET_WINDOW: Duration = Duration::from_millis(350);

/// Upper bound on the settle phase. A pane that never goes quiet (a spinner, a
/// tailing log) still gets its Enter rather than hanging the caller.
const SUBMIT_SETTLE_MAX: Duration = Duration::from_secs(5);

/// How long each confirm window watches the tail after an Enter. Two of these
/// plus the settle bound is the whole operation's host-side ceiling, which the
/// CLI's own poll window sits strictly above.
const SUBMIT_CONFIRM_WINDOW: Duration = Duration::from_secs(5);

/// Tail depth read from the pane grid for the pre/post-submit comparison.
const SUBMIT_TAIL_LINES: usize = 20;

/// Tail lines echoed back to the caller when a submit goes unconfirmed, so it
/// can branch on what the input line actually shows.
const SUBMIT_REPORTED_TAIL_LINES: usize = 4;

/// The one reliable on-screen marker of a paste that an agent TUI has collapsed
/// into a block instead of accepting as a prompt.
const COLLAPSED_PASTE_HINT: &str = "paste again to expand";

/// Poll cadence while a submit is in flight. Unlike the other waits here, this
/// one is not idling until a deadline — it samples the grid — so it schedules a
/// sub-second repaint rather than a single wake at expiry.
const SUBMIT_SERVICE_INTERVAL: Duration = Duration::from_millis(50);

/// Frame cadence while an agent boot is parked. The host-observed boot signal
/// (`tick_terminal_activity` synthesis) updates at most once per second and
/// only when frames run, so the boot service keeps frames coming at half that
/// period rather than sleeping until the timeout deadline.
const AGENT_BOOT_POLL_INTERVAL: Duration = Duration::from_millis(500);
const AGENT_BOOT_INTERPRETER_STABLE: Duration = Duration::from_secs(1);

/// The single collapsed-paste retry, spent by moving it out of the
/// [`PendingSubmit`].
///
/// A consumed-once value rather than a counter on purpose: there is no field to
/// increment and no loop to widen, so "exactly one retry" cannot drift to two
/// through a later edit. Taking it is the only way to send a second Enter.
struct CollapsedPasteRetry;

/// Where a parked submit is in its sequence.
enum SubmitPhase {
    /// Text is typed; waiting for the pane to stop producing before Enter.
    Settle { until: Instant },
    /// Enter is written; watching the tail for the prompt to leave the input
    /// line. `tail_hash` is the tail as it looked immediately before that
    /// Enter.
    Confirm { until: Instant, tail_hash: u64 },
}

/// A parked `plexi pane send --submit` request.
///
/// Confirmation is deliberately *not* "the typed text is no longer on screen":
/// a submitted prompt echoes into the transcript, so the text staying visible
/// proves nothing. Two signals carry the whole contract instead — the tail
/// changing at all is the generic evidence something was accepted, and
/// [`COLLAPSED_PASTE_HINT`] is the one marker specific to the failure this verb
/// heals.
pub(crate) struct PendingSubmit {
    pub pane_id: u64,
    /// Plain JSON response file, like the agent-boot path. `None` when the
    /// caller wanted the sequence run but no answer — the host still settles,
    /// submits and confirms, and logs the outcome.
    pub response_file: Option<String>,
    phase: SubmitPhase,
    retry: Option<CollapsedPasteRetry>,
    /// Whether the retry was spent, reported back so a caller can tell a clean
    /// submit from a healed one.
    retried: bool,
    requested_at: Instant,
}

/// A parked `plexi pane slot wait` request.
pub(crate) struct PendingSlotWait {
    pub pane_id: u64,
    pub slot_name: String,
    pub pattern: regex::Regex,
    pub response_file: String,
    /// Reply with a typed timeout once this instant passes.
    pub expires_at: Instant,
}

/// One structured message in a pane's host-managed FIFO inbox.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PaneMessage {
    pub id: u64,
    pub pane_id: u64,
    pub to_pane_id: u64,
    pub from_pane_id: Option<u64>,
    pub sender_pane_id: Option<u64>,
    pub payload: String,
    pub timestamp_ms: u64,
}

/// A parked `plexi pane message recv --wait` request.
pub(crate) struct PendingMessageRecv {
    pub pane_id: u64,
    pub response_file: String,
    /// Reply with a typed timeout once this instant passes.
    pub expires_at: Instant,
}

/// A parked `plexi pane result --wait` request (stint 0666).
pub(crate) struct PendingResultWait {
    pub pane_id: u64,
    pub response_file: String,
    /// Reply with a typed timeout once this instant passes.
    pub expires_at: Instant,
}

/// A parked `plexi agent wait <pane-id...>` request (stint 0961).
pub(crate) struct PendingAgentWait {
    pub pane_ids: Vec<u64>,
    pub response_file: String,
    pub expires_at: Instant,
}

/// A `plexi pane new --agent` spawn whose response is held until the agent
/// boots.
pub(crate) struct PendingAgentBoot {
    pub pane_id: u64,
    /// The alias typed into the pane's PTY, kept for the log lines and the
    /// timeout message — the caller needs to know *what* failed to boot.
    pub agent_cmd: String,
    /// Plain JSON response file. Unlike `pane slot wait`, the spawn reply
    /// channel has no `.err` sidecar: success and failure both land here.
    pub response_file: String,
    pub requested_at: Instant,
    pub expires_at: Instant,
}

impl PlexiApp {
    pub(crate) fn pane_heartbeat_json(&self, pane_id: u64) -> serde_json::Value {
        self.pane_heartbeats
            .get(&pane_id)
            .map_or(serde_json::Value::Null, |heartbeat| {
                serde_json::json!({
                    "every_ms": heartbeat.every.as_millis(),
                    "text": heartbeat.text,
                    "while_idle_only": heartbeat.while_idle_only,
                })
            })
    }

    /// Fire due host-owned prompts from logic so hidden hosts remain wakeable.
    pub(crate) fn service_pane_heartbeats(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let due: Vec<u64> = self
            .pane_heartbeats
            .iter()
            .filter_map(|(&pane_id, heartbeat)| (now >= heartbeat.next_fire).then_some(pane_id))
            .collect();
        for pane_id in due {
            let idle = self.agent_reports_idle(pane_id);
            let submit_in_flight = self.submit_in_flight(pane_id);
            let Some(heartbeat) = self.pane_heartbeats.get_mut(&pane_id) else {
                continue;
            };
            heartbeat.next_fire = now + heartbeat.every;
            if heartbeat.while_idle_only && !idle {
                log::info!("pane_heartbeat: pane_id={pane_id} skipped reason=working");
                continue;
            }
            let text = heartbeat.text.clone();
            let Some(term) = self
                .windows
                .iter_mut()
                .find_map(|window| window.panes.get_mut(&pane_id))
                .and_then(crate::host::pane::Pane::as_terminal_mut)
            else {
                self.pane_heartbeats.remove(&pane_id);
                log::info!("pane_heartbeat: pane_id={pane_id} removed reason=closed");
                continue;
            };
            if submit_in_flight {
                log::info!("pane_heartbeat: pane_id={pane_id} skipped reason=submit_in_flight");
                continue;
            }
            term.backend
                .process_command(egui_term::BackendCommand::Write(text.clone().into_bytes()));
            self.register_pending_submit(pane_id, None, text.chars().count());
            log::info!("pane_heartbeat: pane_id={pane_id} fired");
        }
        if let Some(next) = self
            .pane_heartbeats
            .values()
            .map(|heartbeat| heartbeat.next_fire)
            .min()
        {
            let remaining = next.saturating_duration_since(Instant::now());
            let predicted = Duration::from_secs_f32(ctx.input(|input| input.predicted_dt));
            ctx.request_repaint_after(remaining + predicted);
        }
    }
    /// Path the host currently tracks for `slot_name` on `pane_id`.
    ///
    /// The tracked path is authoritative: a slot may live outside the default
    /// per-pane directory once it has been written through an explicit path.
    pub(crate) fn tracked_slot_path(
        &self,
        pane_id: u64,
        slot_name: &str,
    ) -> Option<std::path::PathBuf> {
        self.windows
            .iter()
            .find_map(|win| win.panes.get(&pane_id))
            .and_then(|pane| pane.slots())
            .and_then(|slots| slots.get(slot_name).cloned())
            .or_else(|| {
                self.closed_pane_slots
                    .get(&pane_id)
                    .and_then(|slots| slots.get(slot_name).cloned())
            })
    }

    /// Expire overdue slot waits and keep frames coming until the next one.
    ///
    /// Call from `App::logic` only — see the module docs. The repaint is
    /// scheduled at the earliest remaining expiry rather than on a fast poll
    /// interval: a five-minute wait must cost the host one wake, not thousands.
    pub(crate) fn service_pending_slot_waits(&mut self, ctx: &egui::Context) {
        if self.pending_slot_waits.is_empty() {
            return;
        }
        let now = Instant::now();
        if self
            .pending_slot_waits
            .iter()
            .any(|wait| now >= wait.expires_at)
        {
            let (expired, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_slot_waits)
                .into_iter()
                .partition(|wait| now >= wait.expires_at);
            self.pending_slot_waits = live;
            for wait in expired {
                log::info!(
                    "pane_slot_wait: pane_id={} slot={:?} pattern={:?} timed out",
                    wait.pane_id,
                    wait.slot_name,
                    wait.pattern.as_str()
                );
                crate::rpc::write_json_response(
                    &format!("{}.err", wait.response_file),
                    serde_json::json!({
                        "ok": false,
                        "timeout": true,
                        "error": format!(
                            "timed out waiting for slot '{}' on pane {} to match {:?}",
                            wait.slot_name, wait.pane_id, wait.pattern.as_str()
                        ),
                    }),
                );
            }
        }
        let Some(next_expiry) = self.pending_slot_waits.iter().map(|w| w.expires_at).min() else {
            return;
        };
        let predicted_frame =
            std::time::Duration::from_secs_f32(ctx.input(|input| input.predicted_dt));
        ctx.request_repaint_after(crate::host::wasm_frame::repaint_delay_until(
            next_expiry,
            Instant::now(),
            predicted_frame,
        ));
    }

    /// Answer a finished `spawn_pane`, parking the reply when the caller asked
    /// for an agent.
    ///
    /// Every `spawn_pane` reply goes through here so the `--agent` contract
    /// holds on all placement branches: a failed launch reports its error, a
    /// plain spawn reports the pane id immediately, and an agent spawn types
    /// the alias into the new pane's PTY and owes its answer to
    /// [`PlexiApp::complete_agent_boots`] instead.
    ///
    /// Bytes written to the PTY before the shell has finished starting are held
    /// by the kernel until the shell reads them, so launching the alias this way
    /// does not race shell startup.
    pub(crate) fn reply_spawn_pane(
        &mut self,
        spec: &crate::app::launch_spec::PaneLaunchSpec,
        pane_id: u64,
        launch_result: &Result<(), String>,
    ) {
        let error = match launch_result {
            Ok(()) => None,
            Err(msg) => {
                log::warn!("pane_ipc: spawn_pane: launch failed, returning error to caller: {msg}");
                self.reclaim_pane_worktree(pane_id);
                Some(msg.clone())
            }
        };
        if let (None, Some(agent)) = (&error, &spec.agent) {
            let typed = self
                .windows
                .iter_mut()
                .find_map(|win| win.panes.get_mut(&pane_id))
                .and_then(crate::host::pane::Pane::as_terminal_mut)
                .map(|term| {
                    term.backend
                        .process_command(egui_term::BackendCommand::Write(
                            format!("{}\n", agent.command).into_bytes(),
                        ));
                });
            if typed.is_none() {
                let msg = format!("pane {pane_id} is not a terminal, cannot launch an agent in it");
                log::warn!("pane_agent_boot: {msg}");
                self.reclaim_pane_worktree(pane_id);
                if let Some(rf) = &spec.response_file {
                    crate::rpc::write_json_response(rf, serde_json::json!({ "error": msg }));
                }
                return;
            }
            let Some(response_file) = spec.response_file.clone() else {
                // Nothing to park a reply on: the alias is launched, and a
                // caller that asked for no response cannot be told about boot.
                log::info!(
                    "pane_agent_boot: pane_id={pane_id} agent_cmd={:?} launched with no response file; not waiting for boot",
                    agent.command
                );
                return;
            };
            let requested_at = Instant::now();
            log::info!(
                "pane_agent_boot: pane_id={pane_id} agent_cmd={:?} launched, waiting up to {:.1}s for an idle report",
                agent.command,
                agent.timeout.as_secs_f64()
            );
            self.pending_agent_boots.push(PendingAgentBoot {
                pane_id,
                agent_cmd: agent.command.clone(),
                response_file,
                requested_at,
                expires_at: requested_at + agent.timeout,
            });
            // Level-triggered, like every other wait here: an agent that
            // already reported idle answers now rather than on the next report.
            self.complete_agent_boots(pane_id);
            return;
        }
        let Some(rf) = &spec.response_file else {
            return;
        };
        let json = match error {
            None => self.spawn_pane_success_json(pane_id),
            Some(msg) => serde_json::json!({ "error": msg }),
        };
        crate::rpc::write_json_response(rf, json);
    }

    pub(crate) fn spawn_pane_success_json(&self, pane_id: u64) -> serde_json::Value {
        match self.pane_worktrees.get(&pane_id) {
            Some(wt) => serde_json::json!({
                "pane_id": pane_id,
                "worktree": wt.path,
                "branch": wt.branch,
            }),
            None => serde_json::json!({ "pane_id": pane_id }),
        }
    }

    /// True once the pane's agent reports idle — the boot predicate.
    ///
    /// Reads the shared agent detector through `Pane::agent()`, which merges
    /// its two signal sources: hook-reported state (authoritative once any
    /// lifecycle hook fires — Claude Code's `SessionStart` lands ~1s after
    /// boot) and the host-observed synthesis from `tick_terminal_activity`
    /// (known agent process in the PTY foreground + output settle), which
    /// covers agents whose hooks stay silent until first interaction — Codex
    /// defers `session_start` to the first prompt submission. There is
    /// deliberately no prompt-signature parsing anywhere in this predicate:
    /// both sources live in the one detector that also feeds `pane list`.
    pub(crate) fn agent_reports_idle(&self, pane_id: u64) -> bool {
        self.windows
            .iter()
            .find_map(|win| win.panes.get(&pane_id))
            .and_then(|pane| pane.agent())
            .is_some_and(|agent| agent.state == AgentState::Idle)
    }

    fn unsupported_interpreter_boot(
        &self,
        pane_id: u64,
        agent_cmd: &str,
    ) -> Option<(String, Option<String>)> {
        let requested = agent_cmd.split_whitespace().next()?;
        let requested_name = std::path::Path::new(requested)
            .file_name()
            .and_then(|name| name.to_str())?;
        // An unknown custom command may still have a valid lifecycle hook.
        // Fail fast only when the caller explicitly requested a built-in
        // known agent identity but the stable interpreter argv contradicts
        // our ability to observe that identity.
        crate::host::shell::known_agent_process(requested_name)?;
        let terminal = self
            .windows
            .iter()
            .find_map(|window| window.panes.get(&pane_id))
            .and_then(crate::host::pane::Pane::as_terminal)?;
        if !terminal
            .last_pty_output_at
            .is_some_and(|at| at.elapsed() >= AGENT_BOOT_INTERPRETER_STABLE)
        {
            return None;
        }
        let shell_pid = terminal.backend.child_pid();
        let foreground = terminal
            .backend
            .foreground_pid()
            .filter(|pid| *pid != shell_pid as i32)?;
        match crate::host::shell::get_pid_agent_probe(foreground as u32) {
            crate::host::shell::AgentProcessProbe::UnsupportedInterpreter {
                interpreter,
                script,
            } => Some((interpreter, script)),
            crate::host::shell::AgentProcessProbe::Known(_)
            | crate::host::shell::AgentProcessProbe::Unknown => None,
        }
    }

    /// Fail every parked boot whose pane is gone, expire the overdue ones, and
    /// keep frames coming until the next deadline.
    ///
    /// Call from `App::logic` only — see the module docs. A pane that closes or
    /// whose shell exits resolves its waiter here rather than at the timeout:
    /// closing a pane always drives a host pass, so the caller learns within a
    /// frame instead of blocking for the full boot window on a pane that can
    /// never report.
    pub(crate) fn service_pending_agent_boots(&mut self, ctx: &egui::Context) {
        if self.pending_agent_boots.is_empty() {
            return;
        }
        // Level-triggered completion: the predicate may flip through the
        // host-observed detector (`tick_terminal_activity` synthesis) rather
        // than a hook report, and that path has no completion callback — so
        // re-check every parked pane each pass instead of relying solely on
        // the `SetAgentState` edge in `complete_agent_boots`.
        let mut parked: Vec<u64> = self.pending_agent_boots.iter().map(|b| b.pane_id).collect();
        parked.dedup();
        for pane_id in parked {
            self.complete_agent_boots(pane_id);
        }
        if self.pending_agent_boots.is_empty() {
            return;
        }
        let unsupported: Vec<_> = self
            .pending_agent_boots
            .iter()
            .filter_map(|boot| {
                self.unsupported_interpreter_boot(boot.pane_id, &boot.agent_cmd)
                    .map(|reason| (boot.pane_id, reason))
            })
            .collect();
        for (pane_id, (interpreter, script)) in unsupported {
            let (failed, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_agent_boots)
                .into_iter()
                .partition(|boot| boot.pane_id == pane_id);
            self.pending_agent_boots = live;
            for boot in failed {
                self.emit_agent_boot_failure(boot.pane_id);
                let script_detail = script
                    .as_deref()
                    .map(|path| format!(" script {path:?}"))
                    .unwrap_or_else(|| " an unreadable script argv".to_string());
                let error = format!(
                    "`{}` is running under interpreter `{interpreter}` with{script_detail}, which does not identify a known agent; readiness cannot be observed at boot",
                    boot.agent_cmd
                );
                log::warn!(
                    "pane_agent_boot: pane_id={} agent_cmd={:?} fail-fast unsupported interpreter={interpreter:?} script={script:?}",
                    boot.pane_id,
                    boot.agent_cmd
                );
                crate::rpc::write_json_response(
                    &boot.response_file,
                    serde_json::json!({
                        "ok": false,
                        "timeout": false,
                        "unsupported_interpreter": true,
                        "pane_id": boot.pane_id,
                        "error": error,
                    }),
                );
            }
        }
        if self.pending_agent_boots.is_empty() {
            return;
        }
        let now = Instant::now();
        let resolved = self
            .pending_agent_boots
            .iter()
            .any(|boot| now >= boot.expires_at || !self.pane_exists(boot.pane_id));
        if resolved {
            let (done, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_agent_boots)
                .into_iter()
                .partition(|boot| now >= boot.expires_at || !self.pane_exists(boot.pane_id));
            self.pending_agent_boots = live;
            for boot in done {
                self.emit_agent_boot_failure(boot.pane_id);
                let elapsed = now
                    .saturating_duration_since(boot.requested_at)
                    .as_secs_f64();
                // A vanished pane and an expired deadline are different
                // failures: the caller must close a pane that is still open,
                // and has nothing to clean up when it is gone. The `timeout`
                // flag is what tells the two apart on the wire.
                let timed_out = self.pane_exists(boot.pane_id);
                let error = if timed_out {
                    log::info!(
                        "pane_agent_boot: pane_id={} agent_cmd={:?} timed out after {elapsed:.2}s",
                        boot.pane_id,
                        boot.agent_cmd
                    );
                    format!(
                        "timed out after {elapsed:.1}s waiting for `{}` to report ready in pane {}",
                        boot.agent_cmd, boot.pane_id
                    )
                } else {
                    log::info!(
                        "pane_agent_boot: pane_id={} agent_cmd={:?} pane closed after {elapsed:.2}s before reporting ready",
                        boot.pane_id,
                        boot.agent_cmd
                    );
                    format!(
                        "pane {} closed after {elapsed:.1}s before `{}` reported ready",
                        boot.pane_id, boot.agent_cmd
                    )
                };
                crate::rpc::write_json_response(
                    &boot.response_file,
                    serde_json::json!({
                        "ok": false,
                        "timeout": timed_out,
                        "pane_id": boot.pane_id,
                        "error": error,
                    }),
                );
            }
        }
        let Some(next_expiry) = self.pending_agent_boots.iter().map(|b| b.expires_at).min() else {
            return;
        };
        let predicted_frame =
            std::time::Duration::from_secs_f32(ctx.input(|input| input.predicted_dt));
        // A booted-but-quiet agent produces no PTY events, so nothing else
        // drives frames between here and the boot deadline — and without
        // frames the 1 Hz activity tick never runs the observed-agent
        // synthesis the predicate above is waiting on. Poll at a bounded
        // cadence while any boot is parked, capped by the timeout deadline.
        let delay = crate::host::wasm_frame::repaint_delay_until(
            next_expiry,
            Instant::now(),
            predicted_frame,
        )
        .min(AGENT_BOOT_POLL_INTERVAL);
        ctx.request_repaint_after(delay);
    }

    /// Answer the parked spawn for `pane_id` if its agent now reports idle.
    /// Called on the `SetAgentState` edge (a hook report lands) and from the
    /// level-triggered re-check in `service_pending_agent_boots` (the
    /// host-observed synthesis has no completion callback of its own).
    pub(crate) fn complete_agent_boots(&mut self, pane_id: u64) {
        if !self
            .pending_agent_boots
            .iter()
            .any(|boot| boot.pane_id == pane_id)
            || !self.agent_reports_idle(pane_id)
        {
            return;
        }
        let (booted, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_agent_boots)
            .into_iter()
            .partition(|boot| boot.pane_id == pane_id);
        self.pending_agent_boots = live;
        self.emit_agent_booted(pane_id, self.pane_observation_provenance(pane_id));
        let payload = self.spawn_pane_success_json(pane_id);
        for boot in booted {
            log::info!(
                "pane_agent_boot: pane_id={pane_id} agent_cmd={:?} reported ready after {:.2}s",
                boot.agent_cmd,
                boot.requested_at.elapsed().as_secs_f64()
            );
            crate::rpc::write_json_response(&boot.response_file, payload.clone());
        }
    }

    /// A provider failure/end cannot satisfy readiness via its legacy idle projection.
    pub(crate) fn fail_agent_boots_from_terminal_report(&mut self, pane_id: u64) {
        let (failed, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_agent_boots)
            .into_iter()
            .partition(|boot| boot.pane_id == pane_id);
        self.pending_agent_boots = live;
        if !failed.is_empty() {
            self.emit_agent_boot_failure(pane_id);
        }
        for boot in failed {
            crate::rpc::write_json_response(
                &boot.response_file,
                serde_json::json!({
                    "ok": false, "timeout": false, "pane_id": pane_id,
                    "error": "agent failed or ended its session before reporting ready",
                }),
            );
        }
    }

    fn pane_exists(&self, pane_id: u64) -> bool {
        self.windows
            .iter()
            .any(|win| win.panes.contains_key(&pane_id))
    }

    /// True while `pane_id` already owes a `pane send --submit` answer.
    pub(crate) fn submit_in_flight(&self, pane_id: u64) -> bool {
        self.pending_submits
            .iter()
            .any(|submit| submit.pane_id == pane_id)
    }

    /// Park a submit for `pane_id` whose text has just been typed into the PTY.
    pub(crate) fn register_pending_submit(
        &mut self,
        pane_id: u64,
        response_file: Option<String>,
        text_chars: usize,
    ) {
        let requested_at = Instant::now();
        log::info!(
            "pane_submit: pane_id={pane_id} typed {text_chars} chars, settling before Enter (quiet={:.0}ms, settle_max={:.1}s)",
            SUBMIT_QUIET_WINDOW.as_secs_f64() * 1000.0,
            SUBMIT_SETTLE_MAX.as_secs_f64()
        );
        self.pending_submits.push(PendingSubmit {
            pane_id,
            response_file,
            phase: SubmitPhase::Settle {
                until: requested_at + SUBMIT_SETTLE_MAX,
            },
            retry: Some(CollapsedPasteRetry),
            retried: false,
            requested_at,
        });
    }

    /// Advance every parked submit and answer the ones that resolved.
    ///
    /// Call from `App::logic` only — see the module docs. This one keeps a
    /// sub-second repaint cadence while any submit is live, because it is
    /// sampling the pane grid rather than idling until a deadline: waking only
    /// at the next expiry would read the tail exactly once and miss the moment
    /// the prompt left the input line.
    pub(crate) fn service_pending_submits(&mut self, ctx: &egui::Context) {
        if self.pending_submits.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut still_pending = Vec::with_capacity(self.pending_submits.len());
        for mut submit in std::mem::take(&mut self.pending_submits) {
            match self.advance_submit(&mut submit, now) {
                Some(reply) => {
                    log::info!(
                        "pane_submit: pane_id={} resolved after {:.2}s reply={reply}",
                        submit.pane_id,
                        submit.requested_at.elapsed().as_secs_f64()
                    );
                    if let Some(rf) = &submit.response_file {
                        crate::rpc::write_json_response(rf, reply);
                    }
                }
                None => still_pending.push(submit),
            }
        }
        self.pending_submits = still_pending;
        if self.pending_submits.is_empty() {
            return;
        }
        let predicted_frame = Duration::from_secs_f32(ctx.input(|input| input.predicted_dt));
        ctx.request_repaint_after(predicted_frame + SUBMIT_SERVICE_INTERVAL);
    }

    /// Run one servicing pass over a single submit. Returns the reply JSON once
    /// the submit has resolved, `None` while it is still in flight.
    fn advance_submit(&mut self, submit: &mut PendingSubmit, now: Instant) -> Option<Value> {
        let pane_id = submit.pane_id;
        if !self.pane_exists(pane_id) {
            log::info!(
                "pane_submit: pane_id={pane_id} closed before the submit could be confirmed"
            );
            return Some(serde_json::json!({
                "error": format!("pane {pane_id} closed before the submission could be confirmed"),
                "retried": submit.retried,
            }));
        }
        match submit.phase {
            SubmitPhase::Settle { until } => {
                let quiet = self
                    .pane_quiet_for(pane_id)
                    .is_none_or(|quiet_for| quiet_for >= SUBMIT_QUIET_WINDOW);
                if !quiet && now < until {
                    return None;
                }
                let tail_hash = tail_hash(&self.pane_tail(pane_id));
                self.press_enter(pane_id);
                log::info!(
                    "pane_submit: pane_id={pane_id} settled after {:.2}s (quiet={quiet}), Enter sent, confirming for {:.1}s",
                    submit.requested_at.elapsed().as_secs_f64(),
                    SUBMIT_CONFIRM_WINDOW.as_secs_f64()
                );
                submit.phase = SubmitPhase::Confirm {
                    until: now + SUBMIT_CONFIRM_WINDOW,
                    tail_hash,
                };
                None
            }
            SubmitPhase::Confirm { until, tail_hash } => {
                let tail = self.pane_tail(pane_id);
                let changed = self::tail_hash(&tail) != tail_hash;
                let collapsed = tail.iter().any(|line| line.contains(COLLAPSED_PASTE_HINT));
                if changed && !collapsed {
                    log::info!(
                        "pane_submit: pane_id={pane_id} confirmed submitted after {:.2}s retried={}",
                        submit.requested_at.elapsed().as_secs_f64(),
                        submit.retried
                    );
                    return Some(serde_json::json!({ "ok": true, "retried": submit.retried }));
                }
                // The collapsed-paste heal. Fire as soon as a redraw kept the
                // hint up, and otherwise at the window's end — never before the
                // first Enter has had the whole window to work.
                if collapsed && (changed || now >= until) {
                    if submit.retry.take().is_some() {
                        self.press_enter(pane_id);
                        submit.retried = true;
                        log::info!(
                            "pane_submit: pane_id={pane_id} still shows {COLLAPSED_PASTE_HINT:?}, sending the one retry Enter"
                        );
                        submit.phase = SubmitPhase::Confirm {
                            until: now + SUBMIT_CONFIRM_WINDOW,
                            tail_hash: self::tail_hash(&tail),
                        };
                        return None;
                    }
                    log::info!(
                        "pane_submit: pane_id={pane_id} still collapsed with the retry already spent"
                    );
                    return Some(unconfirmed_reply(pane_id, &tail, submit.retried));
                }
                if now >= until {
                    log::info!(
                        "pane_submit: pane_id={pane_id} unconfirmed after {:.2}s (tail_changed={changed})",
                        submit.requested_at.elapsed().as_secs_f64()
                    );
                    return Some(unconfirmed_reply(pane_id, &tail, submit.retried));
                }
                None
            }
        }
    }

    /// How long `pane_id` has been producing nothing, or `None` when it has
    /// never produced anything this session — which counts as settled.
    fn pane_quiet_for(&self, pane_id: u64) -> Option<Duration> {
        self.windows
            .iter()
            .find_map(|win| win.panes.get(&pane_id))
            .and_then(crate::host::pane::Pane::as_terminal)
            .and_then(|term| term.last_pty_output_at)
            .map(|at| at.elapsed())
    }

    /// The pane's rendered tail, trailing blank lines dropped. Read from the
    /// grid rather than the capture cursor: an in-place TUI redraw never
    /// advances that cursor, and those panes are exactly this verb's subject.
    pub(crate) fn pane_tail(&self, pane_id: u64) -> Vec<String> {
        let mut lines = self
            .windows
            .iter()
            .find_map(|win| win.panes.get(&pane_id))
            .and_then(crate::host::pane::Pane::as_terminal)
            .map(|term| term.backend.capture_lines(SUBMIT_TAIL_LINES))
            .unwrap_or_default();
        let content_len = lines
            .iter()
            .rposition(|line| !line.trim().is_empty())
            .map_or(0, |pos| pos + 1);
        lines.truncate(content_len);
        lines
    }

    fn press_enter(&mut self, pane_id: u64) {
        if let Some(term) = self
            .windows
            .iter_mut()
            .find_map(|win| win.panes.get_mut(&pane_id))
            .and_then(crate::host::pane::Pane::as_terminal_mut)
        {
            term.backend
                .process_command(egui_term::BackendCommand::Write(b"\r".to_vec()));
        }
    }

    /// Answer every waiter on `(pane_id, slot_name)` whose pattern matches the
    /// slot's new value. Called from the slot write path, which is the only
    /// thing that can change a slot's contents through the host.
    pub(crate) fn complete_slot_waits(&mut self, pane_id: u64, slot_name: &str) {
        if !self
            .pending_slot_waits
            .iter()
            .any(|wait| wait.pane_id == pane_id && wait.slot_name == slot_name)
        {
            return;
        }
        let Some(path) = self.tracked_slot_path(pane_id, slot_name) else {
            return;
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                log::warn!(
                    "pane_slot_wait: could not read slot {slot_name:?} at {} to match waiters: {e}",
                    path.display()
                );
                return;
            }
        };
        let text = String::from_utf8_lossy(&bytes);
        let (matched, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_slot_waits)
            .into_iter()
            .partition(|wait| {
                wait.pane_id == pane_id
                    && wait.slot_name == slot_name
                    && wait.pattern.is_match(&text)
            });
        self.pending_slot_waits = live;
        for wait in matched {
            log::info!(
                "pane_slot_wait: pane_id={pane_id} slot={slot_name:?} pattern={:?} matched {} bytes",
                wait.pattern.as_str(),
                bytes.len()
            );
            crate::rpc::write_response(&wait.response_file, &bytes);
        }
    }

    /// Drain all queued messages from `pane_id`'s FIFO inbox in arrival order.
    pub(crate) fn drain_pane_messages(&mut self, pane_id: u64) -> Vec<PaneMessage> {
        self.pane_mailboxes
            .get_mut(&pane_id)
            .map(|queue| queue.drain(..).collect())
            .unwrap_or_default()
    }

    /// Answer the oldest parked `pane message recv --wait` waiter on `pane_id`
    /// if its inbox now holds at least one message. Returns `true` when a
    /// waiter consumed the inbox.
    pub(crate) fn complete_message_recvs(&mut self, pane_id: u64) -> bool {
        let Some(pos) = self
            .pending_message_recvs
            .iter()
            .position(|wait| wait.pane_id == pane_id)
        else {
            return false;
        };
        if !self
            .pane_mailboxes
            .get(&pane_id)
            .is_some_and(|queue| !queue.is_empty())
        {
            return false;
        }
        let wait = self.pending_message_recvs.remove(pos);
        let messages = self.drain_pane_messages(pane_id);
        log::info!(
            "pane_message_recv: pane_id={pane_id} delivered {} message(s) to parked waiter",
            messages.len()
        );
        crate::rpc::write_json_response(&wait.response_file, serde_json::json!(messages));
        true
    }

    /// Expire overdue `pane message recv --wait` requests, fail waiters whose
    /// pane closed, and schedule the next deadline wakeup from `App::logic`.
    pub(crate) fn service_pending_message_recvs(&mut self, ctx: &egui::Context) {
        if self.pending_message_recvs.is_empty() {
            return;
        }
        let now = Instant::now();
        if self
            .pending_message_recvs
            .iter()
            .any(|wait| now >= wait.expires_at || !self.pane_exists(wait.pane_id))
        {
            let (done, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_message_recvs)
                .into_iter()
                .partition(|wait| now >= wait.expires_at || !self.pane_exists(wait.pane_id));
            self.pending_message_recvs = live;
            for wait in done {
                let pane_alive = self.pane_exists(wait.pane_id);
                let (timeout, error) = if pane_alive {
                    log::info!(
                        "pane_message_recv: pane_id={} timed out",
                        wait.pane_id
                    );
                    (
                        true,
                        format!("timed out waiting for message on pane {}", wait.pane_id),
                    )
                } else {
                    log::info!(
                        "pane_message_recv: pane_id={} closed while waiting for message",
                        wait.pane_id
                    );
                    (
                        false,
                        format!("pane {} closed while waiting for a message", wait.pane_id),
                    )
                };
                crate::rpc::write_json_response(
                    &format!("{}.err", wait.response_file),
                    serde_json::json!({
                        "ok": false,
                        "timeout": timeout,
                        "error": error,
                    }),
                );
            }
        }
        let Some(next_expiry) = self
            .pending_message_recvs
            .iter()
            .map(|w| w.expires_at)
            .min()
        else {
            return;
        };
        let predicted_frame =
            std::time::Duration::from_secs_f32(ctx.input(|input| input.predicted_dt));
        ctx.request_repaint_after(crate::host::wasm_frame::repaint_delay_until(
            next_expiry,
            Instant::now(),
            predicted_frame,
        ));
    }

    /// Answer every parked `plexi pane result --wait` waiter on `pane_id` once a
    /// validated result has been committed (stint 0666).
    pub(crate) fn complete_result_waits(&mut self, pane_id: u64) {
        if !self
            .pending_result_waits
            .iter()
            .any(|wait| wait.pane_id == pane_id)
        {
            return;
        }
        let Some(committed) = self
            .pane_results
            .get(&pane_id)
            .and_then(|state| state.committed.as_ref())
            .cloned()
        else {
            return;
        };
        let mut bytes = serde_json::to_vec(&committed).unwrap_or_else(|_| b"{}".to_vec());
        bytes.push(b'\n');
        let (matched, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_result_waits)
            .into_iter()
            .partition(|wait| wait.pane_id == pane_id);
        self.pending_result_waits = live;
        for wait in matched {
            log::info!(
                "pane_result_wait: pane_id={pane_id} delivered validated result ({} bytes) to parked waiter",
                bytes.len()
            );
            crate::rpc::write_response(&wait.response_file, &bytes);
        }
    }

    /// Expire overdue `plexi pane result --wait` requests, fail waiters whose
    /// pane closed without committing a valid result, and schedule the next
    /// deadline wakeup from `App::logic` (stint 0666).
    pub(crate) fn service_pending_result_waits(&mut self, ctx: &egui::Context) {
        if self.pending_result_waits.is_empty() {
            return;
        }
        let now = Instant::now();
        if self
            .pending_result_waits
            .iter()
            .any(|wait| now >= wait.expires_at || !self.pane_exists(wait.pane_id))
        {
            let (done, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_result_waits)
                .into_iter()
                .partition(|wait| now >= wait.expires_at || !self.pane_exists(wait.pane_id));
            self.pending_result_waits = live;
            for wait in done {
                if let Some(committed) = self
                    .pane_results
                    .get(&wait.pane_id)
                    .and_then(|state| state.committed.as_ref())
                {
                    let mut bytes =
                        serde_json::to_vec(committed).unwrap_or_else(|_| b"{}".to_vec());
                    bytes.push(b'\n');
                    log::info!(
                        "pane_result_wait: pane_id={} delivered committed result on close/expiry check",
                        wait.pane_id
                    );
                    crate::rpc::write_response(&wait.response_file, &bytes);
                    continue;
                }
                let pane_alive = self.pane_exists(wait.pane_id);
                let (timeout, error_code, error) = if pane_alive {
                    log::info!("pane_result_wait: pane_id={} timed out", wait.pane_id);
                    (
                        true,
                        "timeout",
                        format!("timed out waiting for result on pane {}", wait.pane_id),
                    )
                } else if let Some(reason) = self
                    .pane_results
                    .get(&wait.pane_id)
                    .and_then(|s| s.last_invalid_error.as_deref())
                {
                    log::info!(
                        "pane_result_wait: pane_id={} closed after invalid result write: {reason}",
                        wait.pane_id
                    );
                    (
                        false,
                        "invalid_result",
                        format!(
                            "pane {} closed without a valid result (last write rejected: {reason})",
                            wait.pane_id
                        ),
                    )
                } else {
                    log::info!(
                        "pane_result_wait: pane_id={} closed before writing a result",
                        wait.pane_id
                    );
                    (
                        false,
                        "no_result_yet",
                        format!("pane {} closed before writing a result", wait.pane_id),
                    )
                };
                crate::rpc::write_json_response(
                    &format!("{}.err", wait.response_file),
                    serde_json::json!({
                        "ok": false,
                        "timeout": timeout,
                        "error_code": error_code,
                        "error": error,
                    }),
                );
            }
        }
        let Some(next_expiry) = self
            .pending_result_waits
            .iter()
            .map(|w| w.expires_at)
            .min()
        else {
            return;
        };
        let predicted_frame =
            std::time::Duration::from_secs_f32(ctx.input(|input| input.predicted_dt));
        ctx.request_repaint_after(crate::host::wasm_frame::repaint_delay_until(
            next_expiry,
            Instant::now(),
            predicted_frame,
        ));
    }
}

/// Digest of a rendered tail, so a submit compares two grids across polls at
/// the cost of a `u64` rather than a retained copy of the screen.
fn tail_hash(lines: &[String]) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    lines.hash(&mut hasher);
    hasher.finish()
}

/// The typed "typed but not confirmed" answer. It carries the observed input
/// line because that is what lets a caller branch — retry differently, escalate
/// to a human, or accept — instead of guessing why the Enter did not take.
fn unconfirmed_reply(pane_id: u64, tail: &[String], retried: bool) -> Value {
    let observed = &tail[tail.len().saturating_sub(SUBMIT_REPORTED_TAIL_LINES)..];
    serde_json::json!({
        "error": format!("submission not confirmed on pane {pane_id}"),
        "input_line": observed,
        "retried": retried,
    })
}

impl PlexiApp {
    /// Common mailbox delivery for CLI send and the Agents pane reply composer.
    pub(crate) fn enqueue_pane_message(&mut self, pane_id: u64, from_pane_id: Option<u64>, payload: String) -> (u64, bool) {
        let id = self.next_pane_message_id;
        self.next_pane_message_id = id.saturating_add(1);
        let timestamp_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        self.pane_mailboxes.entry(pane_id).or_default().push_back(PaneMessage {
            id, pane_id, to_pane_id: pane_id, from_pane_id, sender_pane_id: from_pane_id, payload, timestamp_ms,
        });
        let delivered = self.complete_message_recvs(pane_id);
        (id, delivered)
    }

    /// Format queued child results into a single batched turn for an idle parent agent.
    pub(crate) fn format_batched_child_results(
        results: &[crate::workspace::QueuedChildResult],
    ) -> String {
        let mut lines = Vec::with_capacity(results.len() + 1);
        if results.len() == 1 {
            lines.push("[Plexi child result]".to_string());
        } else {
            lines.push(format!("[Plexi child results ({})]", results.len()));
        }
        for r in results {
            let mut entry = format!("- {} (#{}) [{}]", r.child_agent, r.child_pane_id, r.status);
            if let Some(ref s) = r.summary {
                if !s.trim().is_empty() {
                    entry.push_str(&format!(": {}", s.trim()));
                }
            }
            if !r.result_json.trim().is_empty() && r.result_json.trim() != "{}" {
                entry.push_str(&format!(" -> {}", r.result_json.trim()));
            }
            lines.push(entry);
        }
        lines.join("\n")
    }

    /// Record a child agent's completion/exit result (`plexi agent done` or exit without report),
    /// wake any blocking `plexi agent wait` callers, or queue the result on the parent pane for
    /// orderly batched delivery when the parent next reports `Idle`.
    pub(crate) fn record_child_agent_result(
        &mut self,
        parent_pane_id: Option<u64>,
        item: crate::workspace::QueuedChildResult,
    ) {
        let child_id = item.child_pane_id;
        crate::host::flight_recorder::record(
            "agent_done_event",
            None,
            Some(child_id),
            None,
            serde_json::json!({
                "pane_id": child_id,
                "parent_pane_id": parent_pane_id,
                "agent": item.child_agent,
                "status": item.status,
                "summary": item.summary,
                "result_json": item.result_json,
                "finished_at_epoch_secs": item.finished_at_epoch_secs,
            }),
        );
        self.child_result_events.insert(child_id, item.clone());

        let has_blocking_waiter = self
            .pending_agent_waits
            .iter()
            .any(|w| w.pane_ids.contains(&child_id));
        if has_blocking_waiter {
            self.pulled_child_results.insert(child_id);
            self.complete_agent_waits();
            return;
        }
        if self.pulled_child_results.contains(&child_id)
            || self.agent_tool_children.contains_key(&child_id)
        {
            return;
        }
        if let Some(parent_id) = parent_pane_id {
            let queued = self
                .windows
                .iter_mut()
                .find_map(|w| w.panes.get_mut(&parent_id))
                .and_then(|p| p.as_terminal_mut())
                .filter(|term| term.is_agent_pane())
                .map(|term| {
                    term.pending_child_results
                        .retain(|r| r.child_pane_id != child_id);
                    term.pending_child_results.push(item);
                })
                .is_some();
            if queued {
                self.mark_workspace_dirty();
                let _ = self.flush_pending_child_results_if_idle(parent_id);
            }
        }
    }

    /// If `parent_pane_id` has queued child results and its agent now reports `Idle` (with no
    /// submit in flight), drain all queued results and deliver them as ONE batched turn.
    pub(crate) fn flush_pending_child_results_if_idle(&mut self, parent_pane_id: u64) -> bool {
        if !self.agent_reports_idle(parent_pane_id) || self.submit_in_flight(parent_pane_id) {
            return false;
        }
        let waited_ids: std::collections::HashSet<u64> = self
            .pending_agent_waits
            .iter()
            .flat_map(|w| w.pane_ids.iter().copied())
            .chain(self.pulled_child_results.iter().copied())
            .collect();
        let drained: Vec<crate::workspace::QueuedChildResult> = {
            let Some(term) = self
                .windows
                .iter_mut()
                .find_map(|w| w.panes.get_mut(&parent_pane_id))
                .and_then(|p| p.as_terminal_mut())
            else {
                return false;
            };
            if term.pending_child_results.is_empty() {
                return false;
            }
            let mut keep = Vec::new();
            let mut deliver = Vec::new();
            for item in std::mem::take(&mut term.pending_child_results) {
                if waited_ids.contains(&item.child_pane_id) {
                    keep.push(item);
                } else {
                    deliver.push(item);
                }
            }
            term.pending_child_results = keep;
            deliver
        };
        if drained.is_empty() {
            return false;
        }
        let batch_text = Self::format_batched_child_results(&drained);
        if let Some(term) = self
            .windows
            .iter_mut()
            .find_map(|w| w.panes.get_mut(&parent_pane_id))
            .and_then(|p| p.as_terminal_mut())
        {
            term.backend.process_command(egui_term::BackendCommand::Write(
                batch_text.clone().into_bytes(),
            ));
        }
        self.delivered_child_batches
            .entry(parent_pane_id)
            .or_default()
            .push(batch_text.clone());
        self.register_pending_submit(parent_pane_id, None, batch_text.chars().count());
        crate::host::flight_recorder::record(
            "child_results_delivered",
            None,
            Some(parent_pane_id),
            None,
            serde_json::json!({
                "parent_pane_id": parent_pane_id,
                "count": drained.len(),
                "children": drained.iter().map(|r| r.child_pane_id).collect::<Vec<_>>(),
                "batch_text": batch_text,
            }),
        );
        self.mark_workspace_dirty();
        true
    }

    fn format_child_result_json(r: &crate::workspace::QueuedChildResult) -> serde_json::Value {
        let parsed_result = serde_json::from_str::<serde_json::Value>(&r.result_json)
            .unwrap_or_else(|_| serde_json::Value::String(r.result_json.clone()));
        serde_json::json!({
            "pane_id": r.child_pane_id,
            "agent": r.child_agent,
            "status": r.status,
            "summary": r.summary,
            "result": parsed_result,
            "finished_at_epoch_secs": r.finished_at_epoch_secs,
        })
    }

    /// Handle `AppRequest::AgentWait`: consume any matching queued results from the parent pane
    /// so they are never re-delivered via idle batching, and either reply immediately if all
    /// children have finished or park until they finish or `timeout_secs` expires.
    pub(crate) fn handle_agent_wait(
        &mut self,
        pane_ids: Vec<u64>,
        timeout_secs: f64,
        response_file: String,
    ) {
        if pane_ids.is_empty() {
            crate::rpc::write_json_response(
                &response_file,
                serde_json::json!({
                    "ok": false,
                    "error": "agent wait requires at least one pane_id",
                }),
            );
            return;
        }
        if !timeout_secs.is_finite() || timeout_secs <= 0.0 || timeout_secs > MAX_WAIT_TIMEOUT_SECS
        {
            crate::rpc::write_json_response(
                &response_file,
                serde_json::json!({
                    "ok": false,
                    "error": format!("timeout must be a finite number in (0, {MAX_WAIT_TIMEOUT_SECS}]"),
                }),
            );
            return;
        }
        // Mark all requested child pane_ids as pulled so they are removed from any parent's
        // pending_child_results queue and never delivered as a duplicate idle-batch turn.
        for &cid in &pane_ids {
            self.pulled_child_results.insert(cid);
        }
        for win in &mut self.windows {
            for pane in win.panes.values_mut() {
                if let Some(term) = pane.as_terminal_mut() {
                    term.pending_child_results
                        .retain(|r| !pane_ids.contains(&r.child_pane_id));
                }
            }
        }
        let expires_at = Instant::now() + Duration::from_secs_f64(timeout_secs);
        self.pending_agent_waits.push(PendingAgentWait {
            pane_ids,
            response_file,
            expires_at,
        });
        self.complete_agent_waits();
    }

    /// Answer any `PendingAgentWait` whose watched child panes have all recorded a completion/exit event.
    pub(crate) fn complete_agent_waits(&mut self) {
        if self.pending_agent_waits.is_empty() {
            return;
        }
        let (ready, pending): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_agent_waits)
            .into_iter()
            .partition(|w| {
                w.pane_ids
                    .iter()
                    .all(|cid| self.child_result_events.contains_key(cid))
            });
        self.pending_agent_waits = pending;
        for wait in ready {
            for &cid in &wait.pane_ids {
                self.pulled_child_results.insert(cid);
                for win in &mut self.windows {
                    for pane in win.panes.values_mut() {
                        if let Some(term) = pane.as_terminal_mut() {
                            term.pending_child_results
                                .retain(|r| r.child_pane_id != cid);
                        }
                    }
                }
            }
            let results: Vec<serde_json::Value> = wait
                .pane_ids
                .iter()
                .filter_map(|cid| self.child_result_events.get(cid))
                .map(Self::format_child_result_json)
                .collect();
            crate::rpc::write_json_response(
                &wait.response_file,
                serde_json::json!({
                    "ok": true,
                    "results": results,
                }),
            );
        }
    }

    /// Service `PendingAgentWait` deadlines and closed panes from `App::logic`.
    pub(crate) fn service_pending_agent_waits(&mut self, ctx: &egui::Context) {
        if self.pending_agent_waits.is_empty() {
            return;
        }
        // If any watched child pane closed without recording an event, synthesize an exited_unreported result.
        let watched: Vec<u64> = self
            .pending_agent_waits
            .iter()
            .flat_map(|w| w.pane_ids.iter().copied())
            .collect();
        for cid in watched {
            if !self.child_result_events.contains_key(&cid) && !self.pane_exists(cid) {
                let now_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                self.child_result_events.insert(
                    cid,
                    crate::workspace::QueuedChildResult {
                        child_pane_id: cid,
                        child_agent: "agent".to_string(),
                        status: "exited_unreported".to_string(),
                        summary: Some("pane closed without agent done".to_string()),
                        result_json: "{}".to_string(),
                        finished_at_epoch_secs: now_secs,
                    },
                );
            }
        }
        self.complete_agent_waits();
        if self.pending_agent_waits.is_empty() {
            return;
        }
        let now = Instant::now();
        if self
            .pending_agent_waits
            .iter()
            .any(|wait| now >= wait.expires_at)
        {
            let (expired, live): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_agent_waits)
                .into_iter()
                .partition(|wait| now >= wait.expires_at);
            self.pending_agent_waits = live;
            for wait in expired {
                let results: Vec<serde_json::Value> = wait
                    .pane_ids
                    .iter()
                    .filter_map(|cid| self.child_result_events.get(cid))
                    .map(Self::format_child_result_json)
                    .collect();
                crate::rpc::write_json_response(
                    &wait.response_file,
                    serde_json::json!({
                        "ok": false,
                        "timeout": true,
                        "error": format!("timed out waiting for agent panes {:?}", wait.pane_ids),
                        "results": results,
                    }),
                );
            }
        }
        let Some(next_expiry) = self.pending_agent_waits.iter().map(|w| w.expires_at).min() else {
            return;
        };
        let predicted_frame =
            std::time::Duration::from_secs_f32(ctx.input(|input| input.predicted_dt));
        ctx.request_repaint_after(crate::host::wasm_frame::repaint_delay_until(
            next_expiry,
            Instant::now(),
            predicted_frame,
        ));
    }
}
