//! Needs attention: the one host surface for recoverable issues.
//!
//! # Routing rule
//!
//! Where a problem is shown depends on what the person can do about it:
//!
//! - **Recoverable host issues** (queue integrity, Keychain access, anything
//!   filed into the Needs-you queue) live in this review: a compact sidebar
//!   entry opens a themed list/detail overlay. They are never printed as raw
//!   paragraphs in navigation chrome and never stack competing overlays.
//! - **Brief ordinary notifications** keep the notification path
//!   (`src/app/notifications.rs`, `notification_modal`). Security records are
//!   never copied into that dismissible queue.
//! - **Local validation** stays beside the control that produced it.
//! - **Blocking prompts** (approval banner, Keychain recovery banner) appear
//!   only when a real decision is required, and the attention view routes to
//!   those same handlers instead of re-implementing them.
//!
//! Viewing, closing, acknowledging and resolving are distinct. Closing the
//! overlay and copying diagnostics never resolve a record, and the overlay
//! never mints a grant: approvals and permission changes are decided in their
//! existing desktop prompts. There is deliberately no "dismiss all".
//!
//! Technical failure logging stays in the normal log; this module adds
//! `attention:` info traces carrying stable issue ids only.

use std::sync::mpsc;

use egui::RichText;

use crate::app::app_trait::KeyDisposition;
use crate::app::input_router::PlexiInput;
use crate::app::{FocusKind, PlexiApp};
use crate::broker::gate::{NeedsYouKind, NeedsYouRecord, PermissionMonitor};
use crate::platform::log_export::{self, LogExcerpt};
use crate::ui::button::{chrome_button, ButtonKind};
use crate::ui::overlay::ModalShell;
use crate::ui::{style, typography};

/// Unresolved Needs-you records, oldest first. Reads the already-loaded
/// monitor: paint must never open a profile store (see `src/ui/AGENTS.md`).
pub(crate) fn open_items() -> Vec<NeedsYouRecord> {
    let mut items = PermissionMonitor::loaded(&crate::config::config_dir())
        .map(|monitor| monitor.open_needs_you())
        .unwrap_or_default();
    // Most severe first, then oldest first (stable, so ties keep queue order).
    items.sort_by_key(|item| (attention_view(item).severity, item.created_at));
    items
}

/// Label of the compact sidebar entry for `count` unresolved items.
pub(crate) fn sidebar_label(count: usize) -> String {
    format!("Needs attention · {count}")
}

/// The item the sidebar's Approve/Dismiss buttons act on: the oldest one
/// that is not an approval click. Approval clicks are decided on the approval
/// banner, which has its own Allow/Deny choices.
pub(crate) fn oldest_actionable(items: &[NeedsYouRecord]) -> Option<&NeedsYouRecord> {
    items
        .iter()
        .filter(|item| item.kind != NeedsYouKind::ApprovalClick)
        .min_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Severity {
    Critical,
    Warning,
    Info,
}

impl Severity {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Critical => "Critical",
            Self::Warning => "Needs decision",
            Self::Info => "Question",
        }
    }
}

/// Plain-language presentation of one record. Built from the record alone, so
/// a host-global failure never invents a context.
#[derive(Clone, Debug)]
pub(crate) struct AttentionView {
    pub id: String,
    pub kind_label: &'static str,
    pub severity: Severity,
    pub title: String,
    pub explanation: String,
    pub impact: String,
    pub next_action: String,
    pub source: String,
    /// Complete, untruncated technical cause.
    pub technical: String,
}

pub(crate) fn attention_view(record: &NeedsYouRecord) -> AttentionView {
    let host_global = record.actor == "host";
    let source = if host_global {
        "Plexi host".to_string()
    } else {
        record.actor.clone()
    };
    let (kind_label, severity, title, explanation, impact, next_action) = match record.kind {
        NeedsYouKind::Integrity if record.resource == "needs-you" => (
            "Queue integrity",
            Severity::Critical,
            "Saved approvals could not be loaded".to_string(),
            "Plexi could not verify the saved approvals queue, so the unverified saved entries were withheld. This does not by itself mean they were changed or lost.".to_string(),
            "Pending approvals saved before the host restarted are not shown, and may need to be requested again.".to_string(),
            "Review the technical details. Acknowledge to remove this notice; that does not repair or grant anything.".to_string(),
        ),
        NeedsYouKind::Integrity => (
            "Integrity",
            Severity::Critical,
            "A saved permission file could not be verified".to_string(),
            "Plexi could not verify a saved permission file, so its unverified contents were not used.".to_string(),
            "Permissions stored in that file do not apply until they are granted again.".to_string(),
            "Review the technical details. Acknowledge to remove this notice; that does not repair or grant anything.".to_string(),
        ),
        NeedsYouKind::Keychain => (
            "Keychain",
            Severity::Critical,
            "Keychain access needed".to_string(),
            "Plexi cannot read the key that seals saved permissions without asking macOS.".to_string(),
            "Saved permissions stay unavailable until access is restored.".to_string(),
            "Choose Continue to let macOS ask for access. If the key stays unreadable, Plexi replaces it and resets only permissions it can no longer verify.".to_string(),
        ),
        NeedsYouKind::ApprovalClick => (
            "Approval",
            Severity::Warning,
            format!("{} is waiting for approval", record.actor),
            record.summary.clone(),
            "The request stays blocked until you decide.".to_string(),
            "Decide in the approval prompt. Closing this review does not decide anything.".to_string(),
        ),
        NeedsYouKind::PermissionChange => (
            "Permission change",
            Severity::Warning,
            format!("{} asked to widen a permission", record.actor),
            record.summary.clone(),
            "The permission stays as it is until you decide.".to_string(),
            "Decide in the approval prompt. Closing this review does not decide anything.".to_string(),
        ),
        NeedsYouKind::Question => (
            "Question",
            Severity::Info,
            format!("{} has a question", record.actor),
            record.summary.clone(),
            "The agent is waiting for your answer.".to_string(),
            "Answer in the agent's pane.".to_string(),
        ),
        NeedsYouKind::BlockedRun => (
            "Blocked run",
            Severity::Info,
            format!("A run by {} is blocked", record.actor),
            record.summary.clone(),
            "The run does not continue until you respond.".to_string(),
            "Open the agent's pane to respond.".to_string(),
        ),
    };
    AttentionView {
        id: record.id.clone(),
        kind_label,
        severity,
        title,
        explanation,
        impact,
        next_action,
        source,
        technical: format!(
            "{}\n\nid: {}\nkind: {}\nactor: {}\nresource: {}",
            record.summary,
            record.id,
            record.kind.as_str(),
            record.actor,
            record.resource
        ),
    }
}

fn format_time(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|utc| {
            utc.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| format!("unix {ts}"))
}

/// Pasteable plain-text report: complete message, technical cause, issue id,
/// timestamp, and host version/channel.
pub(crate) fn copy_error_text(record: &NeedsYouRecord, version: &str, channel: &str) -> String {
    let view = attention_view(record);
    format!(
        "Plexi issue report\nIssue: {id}\nTitle: {title}\nKind: {kind}\nSeverity: {severity}\nTime: {time}\nHost: Plexi {version} (channel: {channel})\nSource: {source}\n\nExplanation: {explanation}\n\nTechnical details:\n{technical}\n",
        id = view.id,
        title = view.title,
        kind = view.kind_label,
        severity = view.severity.label(),
        time = format_time(record.created_at),
        source = view.source,
        explanation = view.explanation,
        technical = view.technical,
    )
}

pub(crate) fn host_channel() -> String {
    crate::config::build_channel().unwrap_or_else(|| "main".to_string())
}

#[derive(Default)]
pub(crate) enum ExportState {
    #[default]
    Idle,
    Preparing {
        issue_id: String,
        rx: mpsc::Receiver<Result<LogExcerpt, String>>,
    },
    Ready {
        issue_id: String,
        excerpt: Box<LogExcerpt>,
    },
    Failed(String),
}

/// Why the user is seeing feedback; shown as one line under the actions.
#[derive(Clone, Debug)]
pub(crate) struct Feedback {
    pub text: String,
    pub is_error: bool,
}

#[derive(Default)]
pub(crate) struct AttentionState {
    pub open: bool,
    pub selected: usize,
    pub show_technical: bool,
    pub feedback: Option<Feedback>,
    pub export: ExportState,
    /// Issue the per-item state above (export, feedback, details) belongs to.
    /// When the selected record changes, that state is discarded: it must
    /// never be shown, copied or saved against a different issue.
    pub item_id: Option<String>,
    /// Action requested by a key this frame; applied by `draw` (needs `ctx`).
    pub pending: Option<AttentionAction>,
}

impl AttentionState {
    /// Drop everything that belongs to the previously selected item.
    fn reset_item_state(&mut self) {
        self.show_technical = false;
        self.feedback = None;
        self.export = ExportState::Idle;
    }

    /// Bind per-item state to `current`; reset it when the item changed.
    fn bind_item(&mut self, current: Option<&str>) {
        if self.item_id.as_deref() != current {
            self.reset_item_state();
            self.item_id = current.map(str::to_string);
        }
    }

    /// True when a prepared export belongs to `issue_id`.
    fn export_ready_for(&self, issue_id: &str) -> bool {
        matches!(&self.export, ExportState::Ready { issue_id: id, .. } if id == issue_id)
    }
}

/// `L` prepares the log excerpt, then copies it once it is ready for the
/// selected issue. Pure so the keyboard path is testable.
fn log_key_action(ready_for_selected: bool) -> AttentionAction {
    if ready_for_selected {
        AttentionAction::CopyLogs
    } else {
        AttentionAction::PrepareLogs
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttentionAction {
    Select(usize),
    ToggleTechnical,
    CopyError,
    PrepareLogs,
    CopyLogs,
    SaveLogs,
    DiscardLogs,
    /// Keychain recovery (`approve = true`) or "Not now" (`false`).
    Keychain(bool),
    Acknowledge,
    Close,
}

impl PlexiApp {
    pub(crate) fn open_attention_overlay(&mut self, source: &str) {
        let items = open_items();
        log::info!(
            "attention: opened source={source} count={} ids={:?}",
            items.len(),
            items.iter().map(|item| item.id.as_str()).collect::<Vec<_>>()
        );
        self.attention.open = true;
        self.attention.selected = 0;
        self.attention.reset_item_state();
        self.attention.item_id = None;
    }

    pub(crate) fn close_attention_overlay(&mut self) {
        if self.attention.open {
            log::info!("attention: closed");
        }
        self.attention.open = false;
        self.attention.pending = None;
        self.attention.export = ExportState::Idle;
    }

    pub(crate) fn sync_attention_focus(&mut self) {
        let open = self.attention.open;
        self.reconcile_focus_layer(FocusKind::AttentionOverlay, open);
    }

    pub(crate) fn attention_handle_key(&mut self, input: &mut PlexiInput) -> KeyDisposition {
        use egui::{Key, Modifiers};
        let none = Modifiers::NONE;
        let action = if input.consume_key(none, Key::Escape) {
            Some(AttentionAction::Close)
        } else if input.consume_key(none, Key::ArrowDown) || input.consume_key(none, Key::J) {
            let count = open_items().len();
            (count > 0).then(|| AttentionAction::Select((self.attention.selected + 1).min(count - 1)))
        } else if input.consume_key(none, Key::ArrowUp) || input.consume_key(none, Key::K) {
            Some(AttentionAction::Select(self.attention.selected.saturating_sub(1)))
        } else if input.consume_key(none, Key::D) {
            Some(AttentionAction::ToggleTechnical)
        } else if input.consume_key(none, Key::C) {
            Some(AttentionAction::CopyError)
        } else if input.consume_key(none, Key::L) {
            let items = open_items();
            let ready = items
                .get(self.attention.selected)
                .is_some_and(|item| self.attention.export_ready_for(&item.id));
            Some(log_key_action(ready))
        } else if input.consume_key(none, Key::X) {
            Some(AttentionAction::DiscardLogs)
        } else if input.consume_key(none, Key::S) {
            Some(AttentionAction::SaveLogs)
        } else if input.consume_key(none, Key::N) {
            Some(AttentionAction::Keychain(false))
        } else if input.consume_key(none, Key::Enter) {
            self.primary_attention_action()
        } else {
            None
        };
        if action.is_some() {
            self.attention.pending = action;
        }
        KeyDisposition::Consumed
    }

    /// Enter: the selected item's one resolving action, if it has one.
    fn primary_attention_action(&self) -> Option<AttentionAction> {
        let items = open_items();
        match items.get(self.attention.selected)?.kind {
            NeedsYouKind::Keychain => Some(AttentionAction::Keychain(true)),
            NeedsYouKind::Integrity => Some(AttentionAction::Acknowledge),
            _ => None,
        }
    }

    pub(crate) fn draw_attention_overlay(&mut self, ctx: &egui::Context) {
        let items = open_items();
        if !items.is_empty() {
            self.attention.selected = self.attention.selected.min(items.len() - 1);
        }
        self.attention
            .bind_item(items.get(self.attention.selected).map(|item| item.id.as_str()));
        self.poll_attention_export(ctx);
        let colors = self.colors;
        let selected = self.attention.selected;
        let views: Vec<AttentionView> = items.iter().map(attention_view).collect();
        let selected_view = views.get(selected).cloned();
        let selected_record = items.get(selected).cloned();

        let mut action = self.attention.pending.take();
        let screen_h = ctx.content_rect().height();
        let body_max = (screen_h - 160.0).max(180.0);
        let list_max = (body_max * 0.3).clamp(64.0, 150.0);
        let detail_max = (body_max - list_max - 90.0).max(110.0);
        let show_technical = self.attention.show_technical;
        let feedback = self.attention.feedback.clone();
        let selected_id = selected_record.as_ref().map(|r| r.id.clone());
        let export_summary: Option<(String, String)> = match &self.attention.export {
            ExportState::Ready { excerpt, issue_id } if Some(issue_id) == selected_id.as_ref() => {
                Some((excerpt.summary(), excerpt.clipboard_text.clone()))
            }
            _ => None,
        };
        let preparing = matches!(self.attention.export, ExportState::Preparing { .. });
        let export_error = match &self.attention.export {
            ExportState::Failed(error) => Some(error.clone()),
            _ => None,
        };

        let response = ModalShell::centered("needs_attention")
            .title("Needs attention")
            .width(style::MODAL_WIDTH_PALETTE)
            .escape(false)
            .show(ctx, &colors, |ui| {
                if views.is_empty() {
                    typography::body_strong(ui, "Nothing needs your attention.", &colors);
                    typography::caption(
                        ui,
                        "Resolved items are removed from this list.",
                        &colors,
                    );
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("attention_list")
                        .max_height(list_max)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for (idx, view) in views.iter().enumerate() {
                                let tint = match view.severity {
                                    Severity::Critical => colors.danger,
                                    Severity::Warning => colors.warning,
                                    Severity::Info => colors.accent,
                                };
                                let label = format!("{}  ·  {}", view.severity.label(), view.title);
                                let button = egui::Button::new(
                                    RichText::new(label)
                                        .size(style::TEXT_CAPTION)
                                        .color(if idx == selected { tint } else { colors.text_primary }),
                                )
                                .fill(if idx == selected {
                                    colors.bg_hover
                                } else {
                                    egui::Color32::TRANSPARENT
                                })
                                .truncate()
                                .min_size(egui::vec2(ui.available_width(), style::BUTTON_H_MD));
                                if ui.add(button).clicked() {
                                    action = Some(AttentionAction::Select(idx));
                                }
                            }
                        });
                    ui.add_space(style::SPACE_SM);
                    ui.separator();
                    ui.add_space(style::SPACE_SM);
                }

                if let (Some(view), Some(record)) = (&selected_view, &selected_record) {
                    egui::ScrollArea::vertical()
                        .id_salt("attention_detail")
                        .max_height(detail_max)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            typography::body_strong(ui, view.title.clone(), &colors);
                            typography::caption(
                                ui,
                                format!(
                                    "{} · {} · {}",
                                    view.kind_label,
                                    view.source,
                                    format_time(record.created_at)
                                ),
                                &colors,
                            );
                            ui.add_space(style::SPACE_SM);
                            typography::body(ui, view.explanation.clone(), &colors);
                            ui.add_space(style::SPACE_SM);
                            typography::caption_strong(ui, "Impact", &colors);
                            typography::body(ui, view.impact.clone(), &colors);
                            ui.add_space(style::SPACE_SM);
                            typography::caption_strong(ui, "What to do", &colors);
                            typography::body(ui, view.next_action.clone(), &colors);
                            ui.add_space(style::SPACE_SM);
                            let toggle = if show_technical {
                                "▾ Technical details"
                            } else {
                                "▸ Technical details"
                            };
                            if ui
                                .add(egui::Button::new(RichText::new(toggle).size(style::TEXT_CAPTION)).frame(false))
                                .clicked()
                            {
                                action = Some(AttentionAction::ToggleTechnical);
                            }
                            if show_technical {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(view.technical.clone())
                                            .monospace()
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    )
                                    .selectable(true),
                                );
                            }
                        });
                    ui.add_space(style::SPACE_MD);
                    ui.horizontal_wrapped(|ui| {
                        if record.kind == NeedsYouKind::Keychain {
                            if chrome_button(ui, "Continue", ButtonKind::Accent, &colors, 0.0)
                                .clicked()
                            {
                                action = Some(AttentionAction::Keychain(true));
                            }
                            if chrome_button(ui, "Not now", ButtonKind::Secondary, &colors, 0.0)
                                .clicked()
                            {
                                action = Some(AttentionAction::Keychain(false));
                            }
                        } else if record.kind == NeedsYouKind::Integrity
                            && chrome_button(ui, "Acknowledge", ButtonKind::Accent, &colors, 0.0)
                                .clicked()
                        {
                            action = Some(AttentionAction::Acknowledge);
                        }
                        if chrome_button(ui, "Copy error", ButtonKind::Secondary, &colors, 0.0)
                            .clicked()
                        {
                            action = Some(AttentionAction::CopyError);
                        }
                        if chrome_button(
                            ui,
                            "Copy session logs",
                            ButtonKind::Secondary,
                            &colors,
                            0.0,
                        )
                        .clicked()
                        {
                            action = Some(AttentionAction::PrepareLogs);
                        }
                    });
                }

                if preparing {
                    typography::caption(ui, "Preparing session log excerpt…", &colors);
                }
                if let Some(error) = &export_error {
                    ui.colored_label(colors.danger, format!("Could not prepare logs: {error}"));
                }
                if let Some((summary, preview)) = &export_summary {
                    ui.add_space(style::SPACE_SM);
                    typography::caption_strong(ui, "Review before copying", &colors);
                    typography::caption(ui, summary.clone(), &colors);
                    typography::caption(ui, "L copy · S save · X discard", &colors);
                    egui::ScrollArea::vertical()
                        .id_salt("attention_log_preview")
                        .max_height(110.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(preview.clone())
                                        .monospace()
                                        .size(style::TEXT_HINT)
                                        .color(colors.text_dim),
                                )
                                .selectable(true),
                            );
                        });
                    ui.horizontal_wrapped(|ui| {
                        if chrome_button(ui, "Copy logs", ButtonKind::Accent, &colors, 0.0)
                            .clicked()
                        {
                            action = Some(AttentionAction::CopyLogs);
                        }
                        if chrome_button(ui, "Save logs", ButtonKind::Secondary, &colors, 0.0)
                            .clicked()
                        {
                            action = Some(AttentionAction::SaveLogs);
                        }
                        if chrome_button(ui, "Discard", ButtonKind::Secondary, &colors, 0.0)
                            .clicked()
                        {
                            action = Some(AttentionAction::DiscardLogs);
                        }
                    });
                }
                if let Some(feedback) = &feedback {
                    ui.add_space(style::SPACE_SM);
                    let color = if feedback.is_error { colors.danger } else { colors.success };
                    ui.colored_label(color, feedback.text.clone());
                }
                ui.add_space(style::SPACE_MD);
                ui.horizontal(|ui| {
                    typography::caption(
                        ui,
                        "↑/↓ select · C copy error · L session logs · D details · Esc close",
                        &colors,
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if chrome_button(ui, "Close", ButtonKind::Secondary, &colors, 0.0).clicked()
                        {
                            action = Some(AttentionAction::Close);
                        }
                    });
                });
            });
        if response.dismissed {
            action = Some(AttentionAction::Close);
        }
        if let Some(action) = action {
            self.apply_attention_action(ctx, action, selected_record.as_ref());
        }
    }

    fn set_attention_feedback(&mut self, text: impl Into<String>, is_error: bool) {
        self.attention.feedback = Some(Feedback {
            text: text.into(),
            is_error,
        });
    }

    fn apply_attention_action(
        &mut self,
        ctx: &egui::Context,
        action: AttentionAction,
        record: Option<&NeedsYouRecord>,
    ) {
        match action {
            AttentionAction::Close => self.close_attention_overlay(),
            AttentionAction::Select(idx) => {
                self.attention.selected = idx;
            }
            AttentionAction::ToggleTechnical => {
                self.attention.show_technical = !self.attention.show_technical;
            }
            AttentionAction::CopyError => {
                let Some(record) = record else {
                    self.set_attention_feedback("No issue selected to copy.", true);
                    return;
                };
                let text = copy_error_text(record, env!("CARGO_PKG_VERSION"), &host_channel());
                ctx.copy_text(text);
                log::info!("attention: copied error id={}", record.id);
                self.set_attention_feedback("Copied error details to the clipboard.", false);
            }
            AttentionAction::PrepareLogs => {
                let Some(record) = record else {
                    self.set_attention_feedback("No issue selected to correlate logs with.", true);
                    return;
                };
                let issue_id = record.id.clone();
                let thread_issue = issue_id.clone();
                // Resolve the path here: it depends on the calling thread's
                // profile, which the worker does not share.
                let log_path = crate::platform::logging::log_path();
                let session_start = log_export::session_start();
                let (tx, rx) = mpsc::channel();
                let spawn = std::thread::Builder::new()
                    .name("attention-log-export".to_string())
                    .spawn(move || {
                        let result = log_export::prepare_excerpt(
                            &log_path,
                            session_start,
                            &thread_issue,
                            log_export::CLIPBOARD_LIMIT_BYTES,
                            log_export::SAVE_LIMIT_BYTES,
                        );
                        let _ = tx.send(result);
                    });
                match spawn {
                    Ok(_) => {
                        log::info!("attention: preparing session log excerpt id={issue_id}");
                        self.attention.export = ExportState::Preparing { issue_id, rx };
                        self.attention.feedback = None;
                    }
                    Err(error) => {
                        log::error!("attention: could not start log export: {error}");
                        self.attention.export =
                            ExportState::Failed(format!("could not start export: {error}"));
                    }
                }
            }
            AttentionAction::CopyLogs => {
                let selected_ready = record
                    .is_some_and(|r| self.attention.export_ready_for(&r.id));
                if let (true, ExportState::Ready { excerpt, issue_id }) =
                    (selected_ready, &self.attention.export)
                {
                    ctx.copy_text(excerpt.clipboard_text.clone());
                    log::info!("attention: copied session logs id={issue_id}");
                    self.set_attention_feedback("Copied reviewed session logs to the clipboard.", false);
                } else {
                    self.set_attention_feedback("Prepare session logs first (L).", true);
                }
            }
            AttentionAction::SaveLogs => {
                let selected_ready = record
                    .is_some_and(|r| self.attention.export_ready_for(&r.id));
                let text = match &self.attention.export {
                    ExportState::Ready { excerpt, .. } if selected_ready => excerpt.save_text.clone(),
                    _ => {
                        self.set_attention_feedback("Prepare session logs first (L).", true);
                        return;
                    }
                };
                match log_export::save_export(&crate::config::config_dir(), &text) {
                    Ok(path) => {
                        log::info!("attention: saved session logs to {}", path.display());
                        self.set_attention_feedback(format!("Saved logs to {}", path.display()), false);
                    }
                    Err(error) => {
                        log::error!("attention: saving session logs failed: {error}");
                        self.set_attention_feedback(format!("Could not save logs: {error}"), true);
                    }
                }
            }
            AttentionAction::DiscardLogs => {
                self.attention.export = ExportState::Idle;
            }
            AttentionAction::Keychain(approve) => {
                let Some(record) = record.filter(|r| r.kind == NeedsYouKind::Keychain) else {
                    return;
                };
                log::info!("attention: keychain recovery invoked id={} approve={approve}", record.id);
                self.finish_attention_resolve(&record.id, approve, "keychain recovery");
            }
            AttentionAction::Acknowledge => {
                let Some(record) = record.filter(|r| r.kind == NeedsYouKind::Integrity) else {
                    return;
                };
                log::info!("attention: acknowledged integrity issue id={}", record.id);
                self.finish_attention_resolve(&record.id, false, "integrity acknowledge");
            }
        }
    }

    /// Resolve through the shared desktop handler. The resolved item's export,
    /// feedback and details never carry over to whichever item is selected next.
    fn finish_attention_resolve(&mut self, id: &str, approve: bool, label: &str) {
        match self.resolve_from_desktop(id, approve, label) {
            Ok(()) => self.attention.reset_item_state(),
            Err(error) => self.set_attention_feedback(error, true),
        }
    }

    /// Collect a finished log export without blocking the frame.
    fn poll_attention_export(&mut self, ctx: &egui::Context) {
        let ExportState::Preparing { issue_id, rx } = &self.attention.export else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(excerpt)) => {
                let issue_id = issue_id.clone();
                self.attention.export = ExportState::Ready {
                    issue_id,
                    excerpt: Box::new(excerpt),
                };
            }
            Ok(Err(error)) => {
                log::warn!("attention: log export failed: {error}");
                self.attention.export = ExportState::Failed(error);
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.attention.export =
                    ExportState::Failed("the export worker stopped unexpectedly".to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(kind: NeedsYouKind, resource: &str, summary: &str) -> NeedsYouRecord {
        NeedsYouRecord {
            id: "ny_test-1".to_string(),
            kind,
            actor: "host".to_string(),
            resource: resource.to_string(),
            summary: summary.to_string(),
            created_at: 1_760_000_000,
            expires_at: None,
            run_tag: None,
            resolution: None,
        }
    }

    #[test]
    fn integrity_view_is_plain_language_without_claiming_tampering() {
        let long = format!("Needs you queue failed its integrity check (seal mismatch). {}", "x".repeat(600));
        let view = attention_view(&record(NeedsYouKind::Integrity, "needs-you", &long));
        assert_eq!(view.title, "Saved approvals could not be loaded");
        assert!(view.explanation.contains("withheld"));
        for claim in ["tamper", "lost forever", "repaired"] {
            assert!(!view.explanation.to_lowercase().contains(claim), "{claim}");
        }
        assert!(view.technical.contains("seal mismatch"));
        assert_eq!(view.source, "Plexi host", "host-global issues name no context");
    }

    #[test]
    fn copy_error_has_the_whole_message_and_metadata() {
        let long = format!("start {} end-marker", "y".repeat(2000));
        let text = copy_error_text(
            &record(NeedsYouKind::Integrity, "needs-you", &long),
            "9.9.9",
            "alpha",
        );
        assert!(text.contains("end-marker"), "hidden tail must be copied");
        assert!(text.contains("ny_test-1"));
        assert!(text.contains("Plexi 9.9.9 (channel: alpha)"));
        assert!(text.contains("Time: "));
        assert!(text.contains("kind: integrity"));
    }

    #[test]
    fn log_key_prepares_then_copies() {
        assert_eq!(log_key_action(false), AttentionAction::PrepareLogs);
        assert_eq!(log_key_action(true), AttentionAction::CopyLogs);
    }

    #[test]
    fn per_item_state_never_follows_a_different_item() {
        let mut state = AttentionState::default();
        state.bind_item(Some("a"));
        state.feedback = Some(Feedback { text: "x".into(), is_error: false });
        state.show_technical = true;
        state.bind_item(Some("a"));
        assert!(state.feedback.is_some(), "same item keeps its state");
        state.bind_item(Some("b"));
        assert!(state.feedback.is_none() && !state.show_technical);
        assert!(!state.export_ready_for("a"));
    }

    #[test]
    fn sidebar_label_is_compact_regardless_of_diagnostics() {
        assert_eq!(sidebar_label(3), "Needs attention · 3");
    }
}
