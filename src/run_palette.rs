/// Run palette — Cmd+Shift+U.
///
/// Shows active and recently completed runs as cards. Each card displays the
/// run's status pill, head_task (current activity), initiating app, and
/// elapsed time. Modeled structurally on `notification_palette.rs`.

use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, RichText, Stroke, Vec2};

use crate::app::PlexiApp;
use crate::app_protocol::{Run, RunStatus};
use crate::overlays::MODAL_WIDTH;

impl PlexiApp {
    pub(crate) fn draw_run_palette(&mut self, ctx: &egui::Context) {
        // Snapshot run store under lock, then release before UI work.
        let runs: Vec<Run> = match self.run_store.lock() {
            Ok(store) => {
                let mut all: Vec<Run> = store.list_all().into_iter().cloned().collect();
                // Newest first — sort by created_at descending.
                all.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                all
            }
            Err(e) => {
                log::error!("run_palette: mutex poisoned: {e}");
                return;
            }
        };

        let total = runs.len();
        if self.run_palette_selected >= total.max(1) {
            self.run_palette_selected = 0;
        }

        // ── Keyboard nav ────────────────────────────────────────────────────
        let mut close = false;
        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                close = true;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                && total > 0
                && self.run_palette_selected + 1 < total
            {
                self.run_palette_selected += 1;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                && self.run_palette_selected > 0
            {
                self.run_palette_selected -= 1;
            }
        });

        // ── Render ──────────────────────────────────────────────────────────
        let screen_rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("run_palette_scrim"))
            .fixed_pos(screen_rect.min)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen_rect, 0.0, Color32::from_black_alpha(120));
                let scrim_response = ui.allocate_rect(screen_rect, egui::Sense::click());
                if scrim_response.clicked() {
                    close = true;
                }
            });

        egui::Area::new(egui::Id::new("run_palette"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);

                        // Header
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Runs")
                                    .size(13.0)
                                    .color(self.colors.text_primary)
                                    .strong(),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let active_count = runs.iter().filter(|r| is_active(r)).count();
                                if active_count > 0 {
                                    ui.label(
                                        RichText::new(format!("{active_count} active"))
                                            .size(10.0)
                                            .color(self.colors.text_dim),
                                    );
                                }
                            });
                        });
                        ui.add_space(6.0);

                        if runs.is_empty() {
                            ui.label(
                                RichText::new("No runs yet")
                                    .size(11.0)
                                    .color(self.colors.text_dim),
                            );
                            return;
                        }

                        egui::ScrollArea::vertical()
                            .max_height(400.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for (row_idx, run) in runs.iter().enumerate() {
                                    let is_selected = row_idx == self.run_palette_selected;
                                    let fill = if is_selected {
                                        self.colors.bg_active
                                    } else {
                                        Color32::TRANSPARENT
                                    };

                                    let row_rect = Rect::from_min_size(
                                        ui.cursor().min,
                                        Vec2::new(MODAL_WIDTH, 52.0),
                                    );
                                    ui.painter().rect_filled(row_rect, CornerRadius::same(4), fill);

                                    // Status dot on the left
                                    let dot_color = status_color(&run.status, &self.colors);
                                    ui.painter().circle_filled(
                                        egui::pos2(row_rect.min.x + 12.0, row_rect.center().y),
                                        4.0,
                                        dot_color,
                                    );

                                    ui.allocate_ui_with_layout(
                                        Vec2::new(MODAL_WIDTH, 52.0),
                                        Layout::left_to_right(Align::Center),
                                        |ui| {
                                            ui.add_space(24.0);
                                            ui.vertical(|ui| {
                                                ui.add_space(4.0);
                                                // Row 1: app id + status pill + elapsed
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        RichText::new(&run.initiator.app_id)
                                                            .size(10.0)
                                                            .color(self.colors.text_dim),
                                                    );
                                                    ui.label(
                                                        RichText::new("\u{203A}")
                                                            .size(10.0)
                                                            .color(self.colors.text_dim),
                                                    );
                                                    let pill = status_label(&run.status);
                                                    ui.label(
                                                        RichText::new(pill)
                                                            .size(9.0)
                                                            .color(dot_color),
                                                    );
                                                    ui.with_layout(
                                                        Layout::right_to_left(Align::Center),
                                                        |ui| {
                                                            ui.label(
                                                                RichText::new(elapsed(run.created_at))
                                                                    .size(9.0)
                                                                    .color(self.colors.text_section),
                                                            );
                                                        },
                                                    );
                                                });
                                                // Row 2: head_task
                                                ui.label(
                                                    RichText::new(&run.head_task)
                                                        .size(12.0)
                                                        .color(self.colors.text_primary),
                                                );
                                                // Row 3: blocked-on-user prompt if applicable
                                                if let RunStatus::BlockedOnUser { ref prompt, .. } = run.status {
                                                    ui.label(
                                                        RichText::new(format!("\u{26A0} {prompt}"))
                                                            .size(10.0)
                                                            .color(Color32::from_rgb(0xf9, 0xc8, 0x6a)),
                                                    );
                                                }
                                            });
                                        },
                                    );

                                    let r = ui.interact(
                                        row_rect,
                                        egui::Id::new(("run_row", row_idx)),
                                        egui::Sense::click(),
                                    );
                                    if r.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }
                                }
                            });
                    });
            });

        if close {
            self.show_run_palette = false;
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn is_active(run: &Run) -> bool {
    !matches!(
        run.status,
        RunStatus::Complete | RunStatus::Failed { .. } | RunStatus::Cancelled
    )
}

fn status_label(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Pending => "pending",
        RunStatus::Running => "running",
        RunStatus::BlockedOnUser { .. } => "blocked:user",
        RunStatus::BlockedOnChild { .. } => "blocked:child",
        RunStatus::Complete => "done",
        RunStatus::Failed { .. } => "failed",
        RunStatus::Cancelled => "cancelled",
    }
}

fn status_color(status: &RunStatus, colors: &crate::theme::Colors) -> Color32 {
    match status {
        RunStatus::Pending => colors.text_dim,
        RunStatus::Running => Color32::from_rgb(0x89, 0xdc, 0xeb),
        RunStatus::BlockedOnUser { .. } => Color32::from_rgb(0xf9, 0xc8, 0x6a),
        RunStatus::BlockedOnChild { .. } => Color32::from_rgb(0xf9, 0xc8, 0x6a),
        RunStatus::Complete => Color32::from_rgb(0xa6, 0xe3, 0xa1),
        RunStatus::Failed { .. } => Color32::from_rgb(0xdd, 0x77, 0x55),
        RunStatus::Cancelled => colors.text_section,
    }
}

/// Format seconds since epoch as a human-readable elapsed string (e.g. "2m ago").
fn elapsed(created_at: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let secs = (now - created_at).max(0);
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}
