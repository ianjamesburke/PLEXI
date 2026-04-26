//! Agent Workspace pane render path (#348 substrate, #349 UX).
//!
//! Layout (top to bottom, left to right):
//!
//!   ┌──────────────────────────────────────────────────────┐
//!   │ <CLI> · <branch> · <task>     [status]   [merged?]  │  ← header
//!   ├──────────────────────────────┬───────────────────────┤
//!   │                              │ M src/foo.rs         │
//!   │     PTY (TerminalView)       │ A src/bar.rs         │  ← body
//!   │                              │                       │
//!   ├──────────────────────────────┴───────────────────────┤
//!   │              [Ready to merge]  (when status == Done) │  ← footer
//!   └──────────────────────────────────────────────────────┘
//!
//! The header now carries a coloured status badge in addition to the substrate's
//! plain text label. The right sidebar shows changed files (refreshed on a 2s
//! timer driven by `tick_agent_workspaces`). The footer surfaces the
//! "Ready to merge" button only when `status == Done`.

use crate::agent_workspace::{AgentStatus, AgentWorkspacePane, MergeOutcome};
use crate::theme::{self, Colors};
use egui::{Color32, RichText, Stroke, Vec2};
use egui_term::{TerminalTheme, TerminalView};

const HEADER_HEIGHT: f32 = 22.0;
const FOOTER_HEIGHT: f32 = 28.0;
const SIDEBAR_WIDTH: f32 = 160.0;

pub fn render(
    ui: &mut egui::Ui,
    pane: &mut AgentWorkspacePane,
    is_focused: bool,
    theme: &TerminalTheme,
    colors: &Colors,
) {
    let outer = ui.available_rect_before_wrap();
    let footer_visible = pane.cached_status == AgentStatus::Done
        || matches!(pane.merge_outcome, Some(MergeOutcome::Merged));

    // ── Header ──────────────────────────────────────────────────────────────
    let header_rect = egui::Rect::from_min_size(
        outer.min,
        egui::vec2(outer.width(), HEADER_HEIGHT),
    );
    render_header(ui, header_rect, pane, colors);

    // ── Footer reserve ──────────────────────────────────────────────────────
    let footer_rect = if footer_visible {
        egui::Rect::from_min_size(
            egui::pos2(outer.min.x, outer.max.y - FOOTER_HEIGHT),
            egui::vec2(outer.width(), FOOTER_HEIGHT),
        )
    } else {
        egui::Rect::NOTHING
    };

    // ── Body split: terminal | sidebar ──────────────────────────────────────
    let body_top = header_rect.max.y;
    let body_bottom = if footer_visible {
        footer_rect.min.y
    } else {
        outer.max.y
    };
    let body_rect = egui::Rect::from_min_max(
        egui::pos2(outer.min.x, body_top),
        egui::pos2(outer.max.x, body_bottom),
    );

    let sidebar_rect = egui::Rect::from_min_size(
        egui::pos2(body_rect.max.x - SIDEBAR_WIDTH, body_rect.min.y),
        egui::vec2(SIDEBAR_WIDTH, body_rect.height()),
    );
    let term_rect = egui::Rect::from_min_max(
        body_rect.min,
        egui::pos2(sidebar_rect.min.x - 4.0, body_rect.max.y),
    );

    // Reserve cursor for the entire outer area so layout downstream is sane.
    ui.advance_cursor_after_rect(outer);

    render_terminal(ui, term_rect, pane, is_focused, theme, colors);
    render_sidebar(ui, sidebar_rect, pane, colors);

    // ── Footer ──────────────────────────────────────────────────────────────
    if footer_visible {
        render_footer(ui, footer_rect, pane, colors);
    }
}

// ── Header ──────────────────────────────────────────────────────────────────

fn render_header(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    pane: &AgentWorkspacePane,
    colors: &Colors,
) {
    ui.painter().rect_filled(rect, 0.0, colors.bg_sidebar);

    // Label on the left.
    ui.painter().text(
        rect.left_center() + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        pane.header_label(),
        egui::FontId::proportional(11.0),
        colors.text_dim,
    );

    // Status badge on the right.
    let (badge_text, badge_color) = status_badge(pane.cached_status, colors);
    let merged_flash = matches!(pane.merge_outcome, Some(MergeOutcome::Merged));
    let (display_text, display_color) = if merged_flash {
        ("merged", colors.accent)
    } else {
        (badge_text, badge_color)
    };

    let badge_rect = egui::Rect::from_min_size(
        egui::pos2(rect.max.x - 80.0, rect.min.y + 4.0),
        egui::vec2(72.0, rect.height() - 8.0),
    );
    ui.painter().rect(
        badge_rect,
        4.0,
        Color32::from_rgba_unmultiplied(display_color.r(), display_color.g(), display_color.b(), 32),
        Stroke::new(1.0, display_color),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        badge_rect.center(),
        egui::Align2::CENTER_CENTER,
        display_text,
        egui::FontId::proportional(10.0),
        display_color,
    );
}

fn status_badge(status: AgentStatus, colors: &Colors) -> (&'static str, Color32) {
    let color = match status {
        AgentStatus::Idle => colors.text_dim,
        AgentStatus::Thinking => colors.accent,
        AgentStatus::Writing => Color32::from_rgb(0xa6, 0xe3, 0xa1),
        AgentStatus::Done => Color32::from_rgb(0xa6, 0xe3, 0xa1),
        AgentStatus::Error => Color32::from_rgb(0xf3, 0x8b, 0xa8),
    };
    (status.label(), color)
}

// ── Terminal ────────────────────────────────────────────────────────────────

fn render_terminal(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    pane: &mut AgentWorkspacePane,
    is_focused: bool,
    theme: &TerminalTheme,
    colors: &Colors,
) {
    if pane.terminal.exited {
        ui.painter().rect_filled(rect, 0.0, colors.terminal_bg);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                ui.colored_label(
                    colors.text_dim,
                    "[CLI exited — close pane to remove worktree]",
                );
            });
        });
        return;
    }

    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
        let font_size = pane.terminal.font_size;
        let view = TerminalView::new(ui, &mut pane.terminal.backend)
            .set_focus(is_focused)
            .set_theme(theme.clone())
            .set_font(theme::terminal_font(font_size))
            .set_size(Vec2::new(rect.width(), rect.height()));
        ui.add(view);
    });
}

// ── Sidebar ─────────────────────────────────────────────────────────────────

fn render_sidebar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    pane: &AgentWorkspacePane,
    colors: &Colors,
) {
    ui.painter().rect_filled(rect, 0.0, colors.bg_sidebar);
    ui.painter().line_segment(
        [
            egui::pos2(rect.min.x, rect.min.y),
            egui::pos2(rect.min.x, rect.max.y),
        ],
        Stroke::new(1.0, colors.border),
    );

    ui.allocate_new_ui(
        egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(8.0, 6.0))),
        |ui| {
            ui.label(
                RichText::new("CHANGED FILES")
                    .size(9.0)
                    .color(colors.text_dim),
            );
            ui.add_space(4.0);

            if pane.changed_files.is_empty() {
                ui.label(
                    RichText::new("(none)")
                        .size(10.0)
                        .color(colors.text_dim),
                );
                return;
            }
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for entry in &pane.changed_files {
                        let status_color = match entry.status {
                            'A' => Color32::from_rgb(0xa6, 0xe3, 0xa1),
                            'M' => colors.accent,
                            'D' => Color32::from_rgb(0xf3, 0x8b, 0xa8),
                            'R' | 'C' => Color32::from_rgb(0xf9, 0xe2, 0xaf),
                            _ => colors.text_dim,
                        };
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(entry.status.to_string())
                                    .size(10.0)
                                    .monospace()
                                    .color(status_color),
                            );
                            ui.label(
                                RichText::new(&entry.path)
                                    .size(10.0)
                                    .color(colors.text_primary),
                            );
                        });
                    }
                });
        },
    );
}

// ── Footer ──────────────────────────────────────────────────────────────────

fn render_footer(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    pane: &mut AgentWorkspacePane,
    colors: &Colors,
) {
    ui.painter().rect_filled(rect, 0.0, colors.bg_sidebar);
    ui.painter().line_segment(
        [
            egui::pos2(rect.min.x, rect.min.y),
            egui::pos2(rect.max.x, rect.min.y),
        ],
        Stroke::new(1.0, colors.border),
    );

    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.horizontal_centered(|ui| {
            ui.add_space(8.0);
            // Disable the button if the merge already ran (until the flash
            // clears). Reusing the button mid-flash would re-fire the merge.
            let merge_running = matches!(pane.merge_outcome, Some(MergeOutcome::Merged));
            let btn = egui::Button::new(
                RichText::new("Ready to merge")
                    .size(11.0)
                    .color(colors.text_primary),
            )
            .stroke(Stroke::new(1.0, colors.accent))
            .corner_radius(4.0);
            let r = ui.add_enabled(!merge_running, btn);
            if r.clicked() {
                pane.run_merge();
            }
            ui.add_space(8.0);
            // Inline status text on the right of the button so the user gets
            // immediate feedback even before the notification surfaces.
            match &pane.merge_outcome {
                Some(MergeOutcome::Merged) => {
                    ui.colored_label(
                        Color32::from_rgb(0xa6, 0xe3, 0xa1),
                        "merged into main",
                    );
                }
                Some(MergeOutcome::Conflict { files }) => {
                    ui.colored_label(
                        Color32::from_rgb(0xf3, 0x8b, 0xa8),
                        format!("conflict in {} file(s)", files.len()),
                    );
                }
                Some(MergeOutcome::Failed { .. }) => {
                    ui.colored_label(
                        Color32::from_rgb(0xf3, 0x8b, 0xa8),
                        "merge failed (see notification)",
                    );
                }
                None => {}
            }
        });
    });
}
