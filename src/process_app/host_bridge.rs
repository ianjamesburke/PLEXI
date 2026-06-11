//! UI-thread bridge for PGAP control commands.

use super::{FrameDoneOutcome, ProcessApp};
use crate::app_protocol::ControlCommand;

impl ProcessApp {
    /// Dispatch a `ControlCommand` that arrived during `ui()`. Called inline
    /// from the `ui()` dispatch loop; has access to `egui::Ui` for operations
    /// that require UI-thread context (font metrics, clipboard, repaint).
    pub(crate) fn handle_control_command(&mut self, ui: &mut egui::Ui, cmd: ControlCommand) {
        match cmd {
            ControlCommand::Ready { sdk, features_used } => {
                self.sdk = Some(sdk);
                self.features_used = features_used;
                self.runtime.mark_ready();
                ui.ctx().request_repaint();
            }
            ControlCommand::FrameDone { frame_id: done_id } => {
                let frame_ok = match self.runtime.complete_frame(done_id) {
                    FrameDoneOutcome::Matched => true,
                    FrameDoneOutcome::Unexpected { expected, got } => {
                        log::error!(
                            "ProcessApp[{}]: FrameDone frame_id={got} expected={expected:?}",
                            self.type_id,
                        );
                        self.lifecycle.on_parse_error();
                        false
                    }
                };
                if !frame_ok {
                    self.pending_frame.clear();
                    return;
                }
                std::mem::swap(&mut self.frame, &mut self.pending_frame);
                self.pending_frame.clear();
                self.click_awaiting_frame = false;
                self.lifecycle.on_frame_done();
                self.render_diag.record_frame_committed(
                    &self.type_id,
                    done_id,
                    self.scheduler_mode.next_frame_delay(),
                    std::time::Instant::now(),
                );
                if self.runtime.has_pending_render() {
                    ui.ctx().request_repaint();
                }
            }
            ControlCommand::Log { level, message } => {
                let target = format!("app::{}", self.type_id);
                match level.as_str() {
                    "error" => log::error!(target: &target, "{message}"),
                    "warn" => log::warn!(target: &target, "{message}"),
                    "debug" => log::debug!(target: &target, "{message}"),
                    _ => log::info!(target: &target, "{message}"),
                }
            }
            ControlCommand::FatalError { message, traceback } => {
                self.record_fatal_error(message, traceback);
            }
            ControlCommand::SetSchedulerMode { mode, fps } => {
                self.set_scheduler_mode(&mode, fps);
            }
            ControlCommand::ScheduleRender { after_ms } => {
                crate::platform::frame_diag::note(
                    crate::platform::frame_diag::RepaintCause::AppScheduleRender,
                );
                let delay = std::time::Duration::from_millis(after_ms as u64);
                self.mark_render_needed_after("schedule_render", delay);
                ui.ctx().request_repaint_after(delay);
            }
            ControlCommand::CopyToClipboard { text } => {
                let preview = text.chars().take(80).collect::<String>();
                log::info!(target: &format!("app::{}", self.type_id), "copy_to_clipboard: {} chars: {:?}", text.len(), preview);
                ui.ctx().copy_text(text);
            }
            ControlCommand::MeasureText {
                request_id,
                text,
                font_size,
                monospace,
            } => {
                let family = if monospace {
                    egui::FontFamily::Monospace
                } else {
                    egui::FontFamily::Proportional
                };
                let font_id = egui::FontId::new(font_size, family);
                let galley = ui.fonts(|f| f.layout_no_wrap(text, font_id, egui::Color32::WHITE));
                let sz = galley.size();
                self.outbound_events
                    .push_back(crate::app_protocol::PlexiEvent::TextMeasured {
                        request_id,
                        width: sz.x,
                        height: sz.y,
                    });
            }
            ControlCommand::MeasureTextWrapped {
                request_id,
                text,
                font_size,
                max_width,
                max_lines,
            } => {
                let font_id = egui::FontId::proportional(font_size);
                let galley =
                    ui.fonts(|f| f.layout(text.clone(), font_id, egui::Color32::WHITE, max_width));
                let n_rows = max_lines
                    .map(|n| (n as usize).min(galley.rows.len()))
                    .unwrap_or(galley.rows.len());
                let height = if n_rows == 0 {
                    0.0
                } else {
                    galley.rows[n_rows - 1].rect.max.y
                };
                log::debug!(
                    "ProcessApp[{}]: MeasureTextWrapped request_id={} max_width={:.1} max_lines={:?} rows={} height={:.1}",
                    self.type_id, request_id, max_width, max_lines, n_rows, height
                );
                self.outbound_events.push_back(
                    crate::app_protocol::PlexiEvent::TextWrappedMeasured {
                        request_id: request_id.clone(),
                        height,
                    },
                );
            }
            ControlCommand::SetMinSize { width, height } => {
                self.live_min_size = Some((width, height));
                log::info!(
                    "ProcessApp[{}]: live min size set to {:.0}×{:.0}",
                    self.type_id,
                    width,
                    height
                );
            }
            ControlCommand::CloseSelf => {
                log::info!(
                    "ProcessApp[{}]: CloseSelf — pane will close on next frame",
                    self.type_id
                );
                self.wants_close_self = true;
            }
        }
    }
}
