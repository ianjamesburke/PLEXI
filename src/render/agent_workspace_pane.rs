//! Agent Workspace pane render path (#348).
//!
//! Reuses the terminal pane's `TerminalView` for the PTY surface. The only
//! chrome the substrate adds is a single-line header above the terminal:
//!
//!   `<CLI display> · <branch> · <task or "(no task)">`
//!
//! No buttons, no diff sidebar, no status pill — those land in #349. The
//! header bar height matches the terminal pane's name-bar so the split
//! geometry stays predictable.

use crate::agent_workspace::AgentWorkspacePane;
use crate::theme::{self, Colors};
use egui::Vec2;
use egui_term::{TerminalTheme, TerminalView};

const HEADER_HEIGHT: f32 = 20.0;

pub fn render(
    ui: &mut egui::Ui,
    pane: &mut AgentWorkspacePane,
    is_focused: bool,
    theme: &TerminalTheme,
    colors: &Colors,
) {
    // Header bar — plain text, no chrome.
    let bar_rect = egui::Rect::from_min_size(
        ui.cursor().min,
        egui::vec2(ui.available_width(), HEADER_HEIGHT),
    );
    ui.advance_cursor_after_rect(bar_rect);
    ui.painter().text(
        bar_rect.left_center() + egui::vec2(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        pane.header_label(),
        egui::FontId::proportional(11.0),
        colors.text_dim,
    );

    // PTY surface.
    if pane.terminal.exited {
        let rect = ui.available_rect_before_wrap();
        ui.painter().rect_filled(rect, 0.0, colors.terminal_bg);
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                ui.colored_label(colors.text_dim, "[CLI exited — close pane to remove worktree]");
            });
        });
        return;
    }

    let font_size = pane.terminal.font_size;
    let view = TerminalView::new(ui, &mut pane.terminal.backend)
        .set_focus(is_focused)
        .set_theme(theme.clone())
        .set_font(theme::terminal_font(font_size))
        .set_size(Vec2::new(ui.available_width(), ui.available_height()));
    ui.add(view);
}
