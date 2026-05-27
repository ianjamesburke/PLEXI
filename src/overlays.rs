use crate::app_trait::AppCommand;
use crate::style;
use crate::theme::Colors;
use crate::tiling::PaneId;
use egui::{Align, Align2, Color32, CornerRadius, Layout, RichText, Stroke, Vec2};

/// Timeout (ms) between presses in a multi-tap confirmation sequence.
const CONFIRM_TIMEOUT_MS: u64 = 1500;

/// Consume the first digit key (0–9) pressed this frame; return its value.
fn consume_digit_key(ctx: &egui::Context) -> Option<u8> {
    ctx.input_mut(|i| {
        let keys = [
            (egui::Key::Num0, 0u8),
            (egui::Key::Num1, 1),
            (egui::Key::Num2, 2),
            (egui::Key::Num3, 3),
            (egui::Key::Num4, 4),
            (egui::Key::Num5, 5),
            (egui::Key::Num6, 6),
            (egui::Key::Num7, 7),
            (egui::Key::Num8, 8),
            (egui::Key::Num9, 9),
        ];
        for (key, n) in keys {
            if i.consume_key(egui::Modifiers::NONE, key) {
                return Some(n);
            }
        }
        None
    })
}

use crate::app::PlexiApp;

pub(crate) const MODAL_WIDTH: f32 = 400.0;
const R6: CornerRadius = CornerRadius::same(6);

/// Show a native macOS folder picker using rfd's async API.
/// Blocks the calling (background) thread; must NOT be called on the main thread.
fn pick_folder() -> Option<std::path::PathBuf> {
    use std::sync::{Arc, Condvar, Mutex};
    use std::task::{Context, Poll, Wake, Waker};
    use std::pin::pin;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        let signal = Arc::new((Mutex::new(false), Condvar::new()));
        struct Signal(Arc<(Mutex<bool>, Condvar)>);
        impl Wake for Signal {
            fn wake(self: Arc<Self>) { self.signal(); }
            fn wake_by_ref(self: &Arc<Self>) { self.signal(); }
        }
        impl Signal {
            fn signal(&self) {
                let (lock, cvar) = &*self.0;
                *lock.lock().unwrap() = true;
                cvar.notify_one();
            }
        }
        let waker: Waker = Arc::new(Signal(Arc::clone(&signal))).into();
        let mut cx = Context::from_waker(&waker);
        let mut f = pin!(f);
        loop {
            match f.as_mut().poll(&mut cx) {
                Poll::Ready(val) => return val,
                Poll::Pending => {
                    let (lock, cvar) = &*signal;
                    let mut ready = lock.lock().unwrap();
                    while !*ready { ready = cvar.wait(ready).unwrap(); }
                    *ready = false;
                }
            }
        }
    }

    let dialog = rfd::AsyncFileDialog::new();
    let handle = block_on(dialog.pick_folder())?;
    Some(handle.path().to_path_buf())
}

fn draw_contact_footer(ui: &mut egui::Ui, colors: &Colors) {
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(
                "If you have any ideas, want to help, or just want to say what's up...",
            )
            .size(style::TEXT_CAPTION)
            .color(colors.text_dim),
        );
        ui.add_space(style::SPACE_SM / 2.0);
        {
            let email = "ADHDISNTREAL@GMAIL.COM";
            let mailto = "mailto:ADHDisntreal@gmail.com";
            let font_id = egui::FontId::proportional(style::TEXT_CAPTION);
            let email_w = ui.fonts(|f| {
                f.layout_no_wrap(email.to_string(), font_id, colors.text_dim)
                    .size()
                    .x
            });
            let btn_w = 24.0;
            let gap = ui.spacing().item_spacing.x;
            let pad = ((ui.available_width() - email_w - gap - btn_w) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.hyperlink_to(
                    RichText::new(email)
                        .size(style::TEXT_CAPTION)
                        .color(colors.text_dim),
                    mailto,
                );
                crate::widgets::copy_button(
                    ui,
                    egui::Id::new("shortcuts_email_copy"),
                    "ADHDisntreal@gmail.com",
                );
            });
        }
        ui.add_space(style::SPACE_SM / 2.0);
        ui.hyperlink_to(
            RichText::new("❤️  Support the project")
                .size(style::TEXT_CAPTION)
                .color(colors.text_dim),
            "https://buymeacoffee.com/ianjamesbu8",
        );
    });
}

fn section_label(ui: &mut egui::Ui, title: &str, colors: &Colors) {
    ui.label(
        RichText::new(title)
            .size(style::TEXT_HINT)
            .color(colors.text_dim)
            .strong(),
    );
    ui.add_space(2.0);
}

struct PaneRow {
    id: PaneId,
    kind: &'static str,
    name: String,
    detail: String,
    status: &'static str,
    /// Supplementary label shown next to status (e.g. agent name from OSC title when busy).
    osc_badge: Option<String>,
}

fn render_inspector_pane_row(
    ui: &mut egui::Ui,
    row: &PaneRow,
    is_selected: bool,
    colors: &Colors,
) -> egui::Response {
    let row_id = row.id;
    let (row_resp, _) = crate::widgets::selectable_row(ui, is_selected, colors, |ui| {
        ui.horizontal_centered(|ui| {
            ui.label(
                RichText::new(format!("#{}", row_id))
                    .size(style::TEXT_CAPTION)
                    .color(colors.text_dim),
            );
            crate::widgets::pane_type_badge(ui, row.kind, colors);
            let display_name: &str = if row.name.is_empty() { row.kind } else { &row.name };
            ui.label(
                RichText::new(display_name)
                    .size(style::TEXT_BODY)
                    .color(colors.text_primary),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                crate::widgets::status_chip(ui, row.status, colors);
                if let Some(badge) = &row.osc_badge {
                    ui.label(
                        RichText::new(format!("· {badge}"))
                            .size(style::TEXT_HINT)
                            .color(colors.text_dim),
                    );
                }
                if !row.detail.is_empty() {
                    ui.scope(|ui| {
                        ui.set_max_width(120.0);
                        crate::widgets::description_label(ui, row.detail.as_str(), colors);
                    });
                }
            });
        });
    });
    row_resp
}

/// Returns `(want_root_overlay, want_desc_overlay, start_rename, want_close, use_pane_dir)`.
/// `use_pane_dir` fires when the user clicks "From pane" to set the context root from the
/// focused pane's cwd without opening the text-input overlay.
fn render_inspector_header(
    ui: &mut egui::Ui,
    ctx_name: &str,
    ctx_root: &Option<std::path::PathBuf>,
    ctx_description: &Option<String>,
    focused_cwd: Option<&std::path::Path>,
    colors: &Colors,
    renaming: bool,
    rename_buffer: &mut String,
) -> (bool, bool, bool, bool, bool) {
    let mut open_root_overlay = false;
    let mut open_description_overlay = false;
    let mut start_rename = false;
    let mut want_close = false;
    let mut use_pane_dir = false;

    // Render the RTL close button BEFORE the greedy title/input widget so egui's
    // left-to-right layout doesn't let the greedy widget consume all available width
    // and push the close button off-screen.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("\u{2715}")
                            .size(style::TEXT_BODY)
                            .color(colors.text_dim),
                    )
                    .frame(false),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Close inspector")
                .clicked()
            {
                want_close = true;
            }
            if renaming {
                let te_id = egui::Id::new("inspector_rename_input");
                // Do NOT call request_focus() here — focus is managed via the one-shot
                // inspector_rename_focus_requested flag in draw_context_inspector, called
                // AFTER all UI renders so it wins egui's last-caller-wins focus contest.
                crate::widgets::styled_text_input(ui, rename_buffer, "Context name...", te_id, colors);
            } else {
                let name_resp = ui.add(
                    egui::Label::new(
                        RichText::new(ctx_name)
                            .size(style::TEXT_TITLE_XL)
                            .color(colors.text_primary)
                            .strong(),
                    )
                    .sense(egui::Sense::click()),
                );
                if name_resp.clicked() {
                    start_rename = true;
                }
                name_resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            }
        });
    });

    if let Some(root) = ctx_root {
        ui.add_space(style::SPACE_SM);
        let root_str = root.display().to_string();
        ui.horizontal(|ui| {
            ui.label(RichText::new(&root_str).size(style::TEXT_CAPTION).color(colors.text_dim));
            crate::widgets::copy_button(ui, egui::Id::new("inspector_copy_root"), &root_str);
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("\u{270e}")
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_dim),
                    )
                    .frame(false),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Edit root path")
                .clicked()
            {
                open_root_overlay = true;
            }
        });
    } else {
        ui.add_space(style::SPACE_SM);
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("Set root\u{2026}")
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_primary),
                    )
                    .frame(false),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Set a project root directory for this context")
                .clicked()
            {
                open_root_overlay = true;
            }
            if let Some(cwd) = focused_cwd {
                let cwd_str = cwd.display().to_string();
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("From pane")
                                .size(style::TEXT_CAPTION)
                                .color(colors.accent),
                        )
                        .frame(false),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(format!("Set root to {cwd_str}"))
                    .clicked()
                {
                    use_pane_dir = true;
                }
            }
        });
    }
    // Description row
    ui.add_space(style::SPACE_SM);
    if let Some(desc) = ctx_description {
        ui.horizontal(|ui| {
            ui.label(RichText::new(desc.as_str()).size(style::TEXT_CAPTION).color(colors.text_dim));
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("\u{270e}")
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_dim),
                    )
                    .frame(false),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Edit description")
                .clicked()
            {
                open_description_overlay = true;
            }
        });
    } else if ui
        .add(
            egui::Button::new(
                RichText::new("Add description\u{2026}")
                    .size(style::TEXT_CAPTION)
                    .color(colors.text_primary),
            )
            .frame(false),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Add a description for this context")
        .clicked()
    {
        open_description_overlay = true;
    }
    ui.add_space(style::SPACE_XL);
    crate::widgets::section_header(ui, "Panes", false, colors);
    ui.add_space(style::SPACE_SM);
    (open_root_overlay, open_description_overlay, start_rename, want_close, use_pane_dir)
}

fn render_inspector_hints(
    ui: &mut egui::Ui,
    pane_count: usize,
    num_contexts: usize,
    colors: &Colors,
    renaming: bool,
) {
    ui.horizontal(|ui| {
        if renaming {
            crate::widgets::key_combo_list(ui, &[&["Enter"]], Some("save"), colors);
            ui.add_space(style::SPACE_MD);
            crate::widgets::key_combo_list(ui, &[&["Esc"]], Some("cancel"), colors);
        } else {
            if num_contexts > 1 {
                crate::widgets::key_combo_list(ui, &[&["⌫"]], Some("delete"), colors);
                ui.add_space(style::SPACE_XL);
            }
            crate::widgets::key_combo_list(ui, &[&["⌘", "R"]], Some("rename"), colors);
            ui.add_space(style::SPACE_MD);
            crate::widgets::key_combo_list(ui, &[&["Esc"]], Some("close"), colors);
            ui.add_space(style::SPACE_MD);
            crate::widgets::key_combo_list(ui, &[&["j"], &["k"]], Some("navigate"), colors);
            if pane_count > 0 {
                ui.add_space(style::SPACE_MD);
                crate::widgets::key_combo_list(ui, &[&["Enter"]], Some("focus pane"), colors);
                ui.add_space(style::SPACE_MD);
                crate::widgets::key_combo_list(ui, &[&["⌘", "W"]], Some("close pane"), colors);
            }
        }
    });
}

impl PlexiApp {
    pub(crate) fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Workspace dots
            let sidebar_contexts: Vec<usize> = (0..self.router.len()).collect();
            if sidebar_contexts.len() > 1 {
                let dot_radius = 4.0;
                let dot_spacing = 12.0;
                let total_width = (sidebar_contexts.len() as f32) * dot_spacing;
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(total_width, ui.available_height()),
                    egui::Sense::hover(),
                );
                let y = rect.center().y;
                let start_x = rect.left() + dot_radius;
                for (dot_i, ctx_i) in sidebar_contexts.iter().enumerate() {
                    let cx = start_x + (dot_i as f32) * dot_spacing;
                    let color = if *ctx_i == self.router.active_idx() {
                        self.colors.accent
                    } else {
                        self.colors.bg_active
                    };
                    ui.painter()
                        .circle_filled(egui::pos2(cx, y), dot_radius, color);
                }
                ui.add_space(4.0);
            }

            // Right side — help button + notification badge
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("?").size(12.0).color(self.colors.text_dim),
                        )
                        .frame(false)
                        .min_size(egui::vec2(24.0, 0.0)),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Keyboard shortcuts (\u{2318}/)")
                    .clicked()
                {
                    self.show_shortcuts = !self.show_shortcuts;
                }

                let version_label = if self.update_available.is_some() {
                    RichText::new(format!("\u{2191} v{}", env!("CARGO_PKG_VERSION")))
                        .size(10.0)
                        .color(self.colors.accent)
                } else {
                    RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .size(10.0)
                        .color(self.colors.text_dim)
                };
                let hover_text = if self.update_available.is_some() {
                    "Update available — click to open changelog"
                } else {
                    "Changelog"
                };
                if ui
                    .add(egui::Button::new(version_label).frame(false))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(hover_text)
                    .clicked()
                {
                    self.show_changelog = !self.show_changelog;
                }

                let notif_count = self.visible_notification_count();
                if notif_count > 0 {
                    let badge_text = if notif_count > 9 {
                        "9+".to_string()
                    } else {
                        notif_count.to_string()
                    };
                    let btn = egui::Button::new(
                        RichText::new(format!("\u{1F514} {badge_text}"))
                            .size(12.0)
                            .color(self.colors.accent),
                    )
                    .frame(false);
                    if ui
                        .add(btn)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text("Notifications (\u{2318}\u{21E7}A)")
                        .clicked()
                    {
                        self.show_notification_modal = !self.show_notification_modal;
                        if self.show_notification_modal && self.current_notify_id.is_none() {
                            self.current_notify_id = self.select_highest_priority();
                        }
                    }
                }
            });
        });
    }

    pub(crate) fn draw_shortcuts_overlay(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let dismissed = crate::widgets::dismissable_modal(ctx, "shortcuts", |ui| {
            egui::Frame::new()
                .fill(self.colors.bg_sidebar.gamma_multiply(0.95))
                .stroke(Stroke::new(1.0, self.colors.border))
                .corner_radius(R6)
                .inner_margin(egui::Margin::symmetric(24, 20))
                .show(ui, |ui| {
                    ui.set_width(style::MODAL_WIDTH_MD);

                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("Keyboard Shortcuts")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                    });
                    ui.add_space(style::SPACE_MD);

                    let colors = &self.colors;

                    ui.horizontal(|ui| {
                        // ── Left column ──────────────────────────────
                        ui.vertical(|ui| {
                            ui.set_width(288.0);

                            section_label(ui, "LAYOUT", colors);
                            egui::Grid::new("shortcuts_layout")
                                .num_columns(2)
                                .min_col_width(80.0)
                                .spacing([style::SPACE_SM, 4.0])
                                .show(ui, |ui| {
                                    let rows: &[(&[&str], &str)] = &[
                                        (&["\u{2318}", "N"], "New window right"),
                                        (&["\u{2318}", "\u{21E7}", "N"], "New context"),
                                        (&["\u{2318}", "D"], "Split right"),
                                        (&["\u{2318}", "\u{21E7}", "D"], "Split down"),
                                        (&["\u{2318}", "\\"], "Split right (mirror)"),
                                        (&["\u{2318}", "\u{21E7}", "\\"], "Split below (mirror)"),
                                        (&["\u{2318}", "W"], "Close pane"),
                                        (&["\u{2318}", "T"], "New tab"),
                                    ];
                                    for (keys, desc) in rows {
                                        crate::widgets::key_combo(ui, keys, colors);
                                        ui.label(
                                            RichText::new(*desc)
                                                .size(style::TEXT_HINT)
                                                .color(colors.text_dim),
                                        );
                                        ui.end_row();
                                    }
                                });

                            ui.add_space(style::SPACE_SM);

                            section_label(ui, "NAVIGATION", colors);
                            egui::Grid::new("shortcuts_nav")
                                .num_columns(2)
                                .min_col_width(80.0)
                                .spacing([style::SPACE_SM, 4.0])
                                .show(ui, |ui| {
                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}"], &["H", "J", "K", "L"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Move focus (panes & windows)")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "\u{21E7}", "L"], &["\u{2318}", "\u{21E7}", "H"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Next / prev tab")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "\u{21E7}", "K"], &["\u{2318}", "\u{21E7}", "J"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("First / last tab")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "["], &["\u{2318}", "]"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Focus history back / forward")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "1"], &["\u{2318}", "9"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Switch context 1–9")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo(
                                        ui,
                                        &["\u{2318}", "\u{21A9}"],
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Zoom pane")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();
                                });
                        });

                        // ── Vertical divider ──────────────────────────
                        ui.add_space(style::SPACE_MD);
                        ui.separator();
                        ui.add_space(style::SPACE_MD);

                        // ── Right column ─────────────────────────────
                        ui.vertical(|ui| {
                            section_label(ui, "APPS & TOOLS", colors);
                            egui::Grid::new("shortcuts_apps")
                                .num_columns(2)
                                .min_col_width(80.0)
                                .spacing([style::SPACE_SM, 4.0])
                                .show(ui, |ui| {
                                    let rows: &[(&[&str], &str)] = &[
                                        (&["\u{2318}", "P"], "Command palette"),
                                        (&["\u{2318}", "E"], "File browser"),
                                        (&["\u{2318}", "0"], "Quick note"),
                                        (&["\u{2318}", "B"], "Toggle sidebar"),
                                        (&["\u{2318}", "\u{21E7}", "A"], "Notifications"),
                                        (&["\u{2318}", "\u{21E7}", "M"], "Toggle minimap"),
                                    ];
                                    for (keys, desc) in rows {
                                        crate::widgets::key_combo(ui, keys, colors);
                                        ui.label(
                                            RichText::new(*desc)
                                                .size(style::TEXT_HINT)
                                                .color(colors.text_dim),
                                        );
                                        ui.end_row();
                                    }
                                });

                            ui.add_space(style::SPACE_SM);

                            section_label(ui, "TERMINAL", colors);
                            egui::Grid::new("shortcuts_terminal")
                                .num_columns(2)
                                .min_col_width(80.0)
                                .spacing([style::SPACE_SM, 4.0])
                                .show(ui, |ui| {
                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "\u{2191}"], &["\u{2318}", "\u{2193}"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Scroll terminal")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();

                                    crate::widgets::key_combo_list(
                                        ui,
                                        &[&["\u{2318}", "="], &["\u{2318}", "-"]],
                                        None,
                                        colors,
                                    );
                                    ui.label(
                                        RichText::new("Font size +/−")
                                            .size(style::TEXT_HINT)
                                            .color(colors.text_dim),
                                    );
                                    ui.end_row();
                                });

                            ui.add_space(style::SPACE_SM);

                            section_label(ui, "SETTINGS", colors);
                            egui::Grid::new("shortcuts_settings")
                                .num_columns(2)
                                .min_col_width(80.0)
                                .spacing([style::SPACE_SM, 4.0])
                                .show(ui, |ui| {
                                    let rows: &[(&[&str], &str)] = &[
                                        (&["\u{2318}", ","], "Open config"),
                                        (&["\u{2318}", "\u{21E7}", ","], "Reload config"),
                                        (&["\u{2318}", "\u{21E7}", "S"], "Secrets manager"),
                                        (&["\u{2318}", "\u{21E7}", "R"], "Rename pane"),
                                        (&["\u{2318}", "/"], "This help"),
                                        (&["\u{2318}", "Q"], "Quit"),
                                    ];
                                    for (keys, desc) in rows {
                                        crate::widgets::key_combo(ui, keys, colors);
                                        ui.label(
                                            RichText::new(*desc)
                                                .size(style::TEXT_HINT)
                                                .color(colors.text_dim),
                                        );
                                        ui.end_row();
                                    }
                                });
                        });
                    });

                    ui.add_space(style::SPACE_MD);
                    ui.separator();
                    ui.add_space(style::SPACE_SM);
                    draw_contact_footer(ui, colors);
                });
        });
        if dismissed {
            self.show_shortcuts = false;
        }
    }

    pub(crate) fn draw_changelog_overlay(&mut self, ctx: &egui::Context) {
        if !self.show_changelog {
            return;
        }
        const CHANGELOG: &str = include_str!("../CHANGELOG.md");

        let dismissed = crate::widgets::dismissable_modal(ctx, "changelog", |ui| {
            egui::Frame::new()
                .fill(self.colors.bg_sidebar.gamma_multiply(0.95))
                .stroke(Stroke::new(1.0, self.colors.border))
                .corner_radius(R6)
                .inner_margin(egui::Margin::symmetric(24, 20))
                .show(ui, |ui| {
                    ui.set_width(560.0);
                    ui.set_max_height(480.0);

                    ui.label(
                        RichText::new("Changelog")
                            .size(13.0)
                            .color(self.colors.text_primary)
                            .strong(),
                    );

                    if let Some(ref latest) = self.update_available.clone() {
                        ui.add_space(8.0);
                        egui::Frame::new()
                            .fill(self.colors.accent.gamma_multiply(0.15))
                            .stroke(Stroke::new(1.0, self.colors.accent.gamma_multiply(0.4)))
                            .corner_radius(R6)
                            .inner_margin(egui::Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("\u{2191} v{latest} available"))
                                            .size(style::TEXT_HINT)
                                            .color(self.colors.accent),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            crate::widgets::copy_button(
                                                ui,
                                                egui::Id::new("update_banner_copy"),
                                                "plexi update",
                                            );
                                        },
                                    );
                                });
                            });
                    }

                    ui.add_space(8.0);

                    egui::ScrollArea::vertical()
                        .max_height(420.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.set_width(512.0);
                            for line in CHANGELOG.lines() {
                                let clean = |s: &str| s.replace("**", "");
                                if line.starts_with("## ") {
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new(clean(&line[3..]))
                                            .size(style::TEXT_BODY)
                                            .color(self.colors.text_primary)
                                            .strong(),
                                    );
                                    ui.add_space(2.0);
                                } else if line.starts_with("### ") {
                                    ui.label(
                                        RichText::new(clean(&line[4..]))
                                            .size(style::TEXT_HINT)
                                            .color(self.colors.text_dim)
                                            .strong(),
                                    );
                                } else if line.starts_with("- ") || line.starts_with("* ") {
                                    ui.label(
                                        RichText::new(clean(line))
                                            .size(style::TEXT_HINT)
                                            .color(self.colors.text_primary),
                                    );
                                } else if line.starts_with('#') || line.trim().is_empty() {
                                    // skip top-level # heading and blank lines
                                } else {
                                    ui.label(
                                        RichText::new(clean(line))
                                            .size(style::TEXT_HINT)
                                            .color(self.colors.text_dim),
                                    );
                                }
                            }
                        });
                });
        });
        if dismissed {
            self.show_changelog = false;
        }
    }

    /// Shared text-input overlay. Renders a centered modal with a text field,
    /// optional "Browse..." button (for folder targets), and Enter/Escape
    /// commit/cancel. Dispatches commit to `OverlayTarget`.
    pub(crate) fn draw_text_input_overlay(&mut self, ctx: &egui::Context) {
        use crate::app::OverlayTarget;

        if self.text_overlay.is_none() {
            return;
        }

        // Consume Enter/Escape before the Area so they don't bleed through.
        let (commit, cancel) = ctx.input_mut(|i| {
            let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            (enter, esc)
        });

        // Poll async folder-picker result.
        if let Some(rx) = &self.text_overlay_browse_rx {
            if let Ok(result) = rx.try_recv() {
                if let Some(path) = result {
                    let path_str = path.display().to_string();
                    log::info!("TextInputOverlay: browse selected path={path_str}");
                    if let Some((ref mut ov, _)) = self.text_overlay {
                        ov.buffer = path_str;
                    }
                }
                self.text_overlay_browse_rx = None;
            }
        }

        // Re-borrow after potential mutation above.
        let (overlay, target) = match self.text_overlay.as_mut() {
            Some(pair) => pair,
            None => return,
        };
        let label = overlay.label.clone();
        let hint = overlay.hint.clone();
        let show_browse = matches!(target, OverlayTarget::ContextRoot(_));
        let mut browse_clicked = false;

        // Scrim
        let screen_rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("text_input_overlay_scrim"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(
                    screen_rect,
                    0.0,
                    Color32::from_black_alpha(style::SCRIM_ALPHA),
                );
            });

        egui::Area::new(egui::Id::new("text_input_overlay"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(style::RADIUS_MD)
                    .inner_margin(egui::Margin::symmetric(
                        style::MODAL_PADDING_H,
                        style::MODAL_PADDING_V,
                    ))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);
                        ui.label(
                            RichText::new(&label)
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);

                        let te_id = egui::Id::new("text_input_overlay_field");
                        let (overlay, _target) = self.text_overlay.as_mut().unwrap();
                        let te = crate::widgets::styled_text_input(
                            ui,
                            &mut overlay.buffer,
                            hint.as_str(),
                            te_id,
                            &self.colors,
                        );

                        // One-shot focus guard.
                        if !overlay.focus_requested {
                            te.request_focus();
                            if let Some(mut state) =
                                egui::TextEdit::load_state(ui.ctx(), te_id)
                            {
                                state
                                    .cursor
                                    .set_char_range(Some(egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(0),
                                        egui::text::CCursor::new(overlay.buffer.len()),
                                    )));
                                state.store(ui.ctx(), te_id);
                            }
                            overlay.focus_requested = true;
                            log::info!("TextInputOverlay: focus requested");
                        }

                        if show_browse {
                            ui.add_space(style::SPACE_SM);
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Browse\u{2026}")
                                                .size(style::TEXT_CAPTION)
                                                .color(self.colors.text_primary),
                                        )
                                        .fill(self.colors.bg_active),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    browse_clicked = true;
                                }
                                ui.add_space(style::SPACE_SM);
                                ui.label(
                                    RichText::new("Enter to confirm, Esc to cancel")
                                        .size(style::TEXT_CAPTION)
                                        .color(self.colors.text_dim),
                                );
                            });
                        }
                    });
            });

        if cancel {
            log::info!("TextInputOverlay: cancelled");
            self.text_overlay = None;
            self.text_overlay_browse_rx = None;
            return;
        }

        if browse_clicked && self.text_overlay_browse_rx.is_none() {
            log::info!("TextInputOverlay: browse folder picker opened");
            let (tx, rx) = std::sync::mpsc::channel();
            self.text_overlay_browse_rx = Some(rx);
            std::thread::spawn(move || {
                let result = pick_folder();
                let _ = tx.send(result);
            });
            return;
        }

        if commit {
            // Take overlay state for dispatch.
            let (overlay, target) = match self.text_overlay.take() {
                Some(pair) => pair,
                None => return,
            };
            self.text_overlay_browse_rx = None;
            let raw = overlay.buffer.trim().to_string();

            match target {
                OverlayTarget::ContextRoot(idx) => {
                    if idx >= self.router.len() {
                        log::warn!("TextInputOverlay: context idx={idx} no longer exists");
                        return;
                    }
                    if raw.is_empty() {
                        log::info!("TextInputOverlay: clear context root idx={idx}");
                        self.router.get_mut(idx).root = None;
                    } else {
                        // Tilde expansion.
                        let expanded = if raw.starts_with("~/") {
                            if let Some(home) = dirs::home_dir() {
                                home.join(&raw[2..])
                            } else {
                                std::path::PathBuf::from(&raw)
                            }
                        } else if raw.starts_with('~') && raw.len() == 1 {
                            dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(&raw))
                        } else {
                            std::path::PathBuf::from(&raw)
                        };

                        // Resolve relative paths against context path.
                        // Skip resolution if the raw input started with ~ (failed
                        // tilde expansion should not be treated as relative).
                        let resolved = if expanded.is_relative() && !raw.starts_with('~') {
                            self.router.get(idx).path.join(&expanded)
                        } else {
                            expanded
                        };

                        log::info!(
                            "TextInputOverlay: set context root idx={idx} path={}",
                            resolved.display()
                        );
                        self.router.get_mut(idx).root = Some(resolved);
                    }
                    self.save_workspace();
                }
            }
        }
    }

    pub(crate) fn draw_rename_pane_overlay(&mut self, ctx: &egui::Context) {
        let pane_id: PaneId = match self.renaming_pane {
            Some(id) => id,
            None => return,
        };

        // Consume Enter/Escape at the context level so they cannot bleed into
        // the focused pane this frame. Pairs with `FocusLayer::RenamePane` —
        // the overlay owns its own commit/cancel keys.
        let (commit, cancel) = ctx.input_mut(|i| {
            let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            (enter, esc)
        });

        egui::Area::new(egui::Id::new("rename_pane_overlay"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);
                        ui.label(
                            RichText::new("Rename Pane")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);

                        let te_id = egui::Id::new("rename_pane_input");
                        let te = crate::widgets::styled_text_input(
                            ui,
                            &mut self.rename_buffer,
                            "Pane name...",
                            te_id,
                            &self.colors,
                        );

                        // One-shot focus: only request on the first render frame.
                        // Re-requesting every frame (guarded by `!te.has_focus()`) lets
                        // any widget rendered later in the same frame steal focus
                        // permanently — egui resolves `request_focus()` conflicts
                        // last-caller-wins within a frame.
                        if !self.rename_pane_focus_requested {
                            te.request_focus();
                            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), te_id) {
                                state
                                    .cursor
                                    .set_char_range(Some(egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(0),
                                        egui::text::CCursor::new(self.rename_buffer.len()),
                                    )));
                                state.store(ui.ctx(), te_id);
                            }
                            self.rename_pane_focus_requested = true;
                            log::info!("rename_pane: focus requested for TextEdit");
                        }
                    });
            });

        if cancel {
            self.renaming_pane = None;
            self.rename_pane_focus_requested = false;
        } else if commit {
            let new_name = self.rename_buffer.trim().to_string();
            if let Some(pane) =
                self.windows[self.active_window].panes.get_mut(&pane_id)
            {
                if let Some(t) = pane.as_terminal_mut() {
                    t.name_locked = !new_name.is_empty();
                    t.name = if new_name.is_empty() {
                        None
                    } else {
                        log::info!("rename_pane: pane {pane_id} name locked to {:?}", new_name);
                        Some(new_name)
                    };
                }
            }
            self.renaming_pane = None;
            self.rename_pane_focus_requested = false;
        }
    }


    /// Modal for naming a newly-created context when the sidebar is hidden.
    /// Mirrors the inline sidebar rename but as a centred overlay so the
    /// terminal becomes immediately usable after the user hits Enter or Escape.
    pub(crate) fn draw_rename_context_overlay(&mut self, ctx: &egui::Context) {
        let ctx_idx = match self.renaming_window {
            Some(idx) => idx,
            None => return,
        };

        let (commit, cancel) = ctx.input_mut(|i| {
            let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            (enter, esc)
        });

        egui::Area::new(egui::Id::new("rename_context_overlay"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);
                        ui.label(
                            RichText::new("Name this context")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);

                        let te_id = egui::Id::new("rename_context_input");
                        let te = crate::widgets::styled_text_input(
                            ui,
                            &mut self.rename_buffer,
                            "Context name...",
                            te_id,
                            &self.colors,
                        );

                        if !te.has_focus() {
                            te.request_focus();
                            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), te_id) {
                                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(self.rename_buffer.len()),
                                )));
                                state.store(ui.ctx(), te_id);
                            }
                        }
                    });
            });

        if cancel {
            self.renaming_window = None;
        } else if commit {
            let new_name = self.rename_buffer.trim().to_string();
            if !new_name.is_empty() {
                self.router.get_mut(ctx_idx).name = new_name;
            }
            self.renaming_window = None;
        }
    }

    /// Modal for editing a context's description. Multiline text input with
    /// Cmd+Enter to save, Esc to cancel.
    pub(crate) fn draw_edit_description_overlay(&mut self, ctx: &egui::Context) {
        let ctx_idx = match self.editing_description {
            Some(idx) if idx < self.router.len() => idx,
            _ => {
                self.editing_description = None;
                self.pop_focus_layer(&crate::app::FocusLayer::ContextDescription);
                return;
            }
        };

        let (commit, cancel) = ctx.input_mut(|i| {
            let cmd_enter = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter);
            let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            (cmd_enter, esc)
        });

        egui::Area::new(egui::Id::new("edit_description_overlay"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 80.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);
                        let ctx_name = self.router.get(ctx_idx).name.clone();
                        ui.label(
                            RichText::new(format!("Description for \"{}\"", ctx_name))
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);

                        let te_id = egui::Id::new("edit_description_input");
                        let te = ui.scope(|ui| {
                            ui.visuals_mut().text_cursor.stroke.width = 1.5;
                            ui.visuals_mut().text_cursor.stroke.color = self.colors.accent;
                            ui.visuals_mut().extreme_bg_color = self.colors.bg_active;
                            ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(1.0, self.colors.accent);
                            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0, self.colors.border);
                            ui.add(
                                egui::TextEdit::multiline(&mut self.description_buffer)
                                    .id(te_id)
                                    .desired_width(MODAL_WIDTH)
                                    .desired_rows(3)
                                    .hint_text("What are you working on in this context?")
                                    .font(egui::TextStyle::Body)
                                    .margin(egui::Margin::symmetric(8, 5)),
                            )
                        }).inner;

                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("Cmd+Enter to save  ·  Esc to cancel")
                                .size(11.0)
                                .color(self.colors.text_dim),
                        );

                        if !self.description_focus_requested {
                            te.request_focus();
                            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), te_id) {
                                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(self.description_buffer.len()),
                                )));
                                state.store(ui.ctx(), te_id);
                            }
                            self.description_focus_requested = true;
                        }
                    });
            });

        if cancel {
            log::info!("context_description: edit cancelled ctx_idx={ctx_idx}");
            self.editing_description = None;
            self.description_focus_requested = false;
            self.pop_focus_layer(&crate::app::FocusLayer::ContextDescription);
        } else if commit {
            let new_desc = self.description_buffer.trim().to_string();
            self.router.get_mut(ctx_idx).description = if new_desc.is_empty() { None } else { Some(new_desc) };
            self.editing_description = None;
            self.description_focus_requested = false;
            self.pop_focus_layer(&crate::app::FocusLayer::ContextDescription);
            self.save_workspace();
            log::info!("context_description: updated ctx_idx={ctx_idx}");
        }
    }

    /// Quick note compose phase: full-screen scrim + centered text input.
    pub(crate) fn draw_quick_note_modal(&mut self, ctx: &egui::Context) {
        use crate::style;
        use egui::{Align2, RichText, Vec2};

        // Consume Esc to close.
        let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if esc {
            self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
            self.quick_note_text.clear();
            log::info!("QuickNote: modal dismissed");
            return;
        }

        // Explicitly consume paste events before the TextEdit renders.
        // egui::TextEdit processes Paste via filtered_events(), which requires
        // settled egui focus. When the zoom overlay is active it calls
        // set_focus(true) unconditionally, stealing focus from the TextEdit and
        // leaving paste unhandled — the event then falls through to the terminal
        // backend. Consuming here is consistent with how Esc and Enter are
        // handled above and guarantees paste lands in QuickNote every frame.
        let pasted: String = ctx.input_mut(|i| {
            let mut text = String::new();
            i.events.retain(|e| {
                if let egui::Event::Paste(t) = e {
                    text.push_str(t);
                    false
                } else {
                    true
                }
            });
            text
        });
        if !pasted.is_empty() {
            let te_id = egui::Id::new("quick_note_text");
            let inserted = if let Some(mut state) = egui::TextEdit::load_state(ctx, te_id) {
                if let Some(range) = state.cursor.char_range() {
                    let lo = range.primary.index.min(range.secondary.index);
                    let hi = range.primary.index.max(range.secondary.index);
                    let chars: Vec<char> = self.quick_note_text.chars().collect();
                    let mut new_chars: Vec<char> = chars[..lo].to_vec();
                    new_chars.extend(pasted.chars());
                    new_chars.extend(chars[hi..].iter().copied());
                    self.quick_note_text = new_chars.into_iter().collect();
                    let new_pos = lo + pasted.chars().count();
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(
                        egui::text::CCursor::new(new_pos),
                    )));
                    state.store(ctx, te_id);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !inserted {
                self.quick_note_text.push_str(&pasted);
            }
            log::info!("QuickNote: pasted {} chars", pasted.len());
        }

        // Plain Enter (no shift) → advance to destination picker.
        let advance = ctx.input_mut(|i| {
            if !i.modifiers.shift && !i.modifiers.command {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            } else {
                false
            }
        });
        if advance && !self.quick_note_text.trim().is_empty() {
            self.quick_note_dest_cursor = 0;
            self.push_focus_layer(crate::app::FocusLayer::QuickNoteDestination);
            log::info!("QuickNote: advancing to destination picker");
            // Draw destination in the same frame to avoid a one-frame scrim flash.
            self.draw_quick_note_destination(ctx);
            return;
        }

        let screen_rect = ctx.screen_rect();

        // Scrim
        egui::Area::new(egui::Id::new("quick_note_scrim"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(
                    screen_rect,
                    0.0,
                    egui::Color32::from_black_alpha(230),
                );
            });

        // Modal — grows from ~25% to ~80% of screen height as the user types.
        let modal_w = (screen_rect.width() * 0.72).min(864.0).max(480.0);
        let max_text_h = (screen_rect.height() * 0.8).max(80.0);
        let line_h = style::TEXT_BODY + 4.0;
        let initial_rows = ((screen_rect.height() * 0.25) / line_h).round() as usize;
        let initial_rows = initial_rows.max(3);
        egui::Area::new(egui::Id::new("quick_note_modal"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, self.colors.border))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(24, 20))
                    .show(ui, |ui| {
                        ui.set_width(modal_w);

                        // Hint
                        ui.label(
                            RichText::new("Enter to send  ·  Shift+Enter for new line  ·  Esc to discard")
                                .color(self.colors.text_dim.linear_multiply(0.5))
                                .size(style::TEXT_HINT)
                                .family(egui::FontFamily::Monospace),
                        );
                        ui.add_space(style::SPACE_SM);

                        // Text input — starts at ~25% screen height, grows with content, caps at ~80%.
                        egui::ScrollArea::vertical()
                            .max_height(max_text_h)
                            .show(ui, |ui| {
                                ui.scope(|ui| {
                                    ui.visuals_mut().text_cursor.stroke.width = 1.5;
                                    ui.visuals_mut().text_cursor.stroke.color = self.colors.accent;
                                    ui.add(
                                        egui::TextEdit::multiline(&mut self.quick_note_text)
                                            .id(egui::Id::new("quick_note_text"))
                                            .font(egui::FontId::monospace(style::TEXT_BODY))
                                            .text_color(self.colors.text_primary)
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(initial_rows)
                                            .frame(false)
                                            .hint_text(
                                                RichText::new("What's on your mind?")
                                                    .color(self.colors.text_dim.linear_multiply(0.3))
                                                    .size(style::TEXT_BODY),
                                            ),
                                    );
                                });
                            });
                    });
            });
    }

    /// Quick note destination picker: digit keys route instantly; arrows/jk navigate.
    pub(crate) fn draw_quick_note_destination(&mut self, ctx: &egui::Context) {
        use crate::style;
        use crate::widgets;
        use egui::{Align2, RichText, Vec2};

        let dest_count = self.config.quick_note.as_ref()
            .map(|qn| qn.destinations.len())
            .unwrap_or(0);
        let total = 1 + dest_count;

        // H or Esc → back to compose.
        let back = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::H)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
        });
        if back {
            self.pop_focus_layer(&crate::app::FocusLayer::QuickNoteDestination);
            log::info!("QuickNote: destination picker dismissed, back to composing");
            self.draw_quick_note_modal(ctx); // same-frame draw to avoid flash
            return;
        }

        // Arrow / vim navigation.
        let up = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::K)
        });
        let down = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::J)
        });
        if up {
            self.quick_note_dest_cursor = if self.quick_note_dest_cursor == 0 {
                total.saturating_sub(1)
            } else {
                self.quick_note_dest_cursor - 1
            };
            log::info!("QuickNote: dest cursor → {}", self.quick_note_dest_cursor);
        }
        if down {
            self.quick_note_dest_cursor = (self.quick_note_dest_cursor + 1) % total.max(1);
            log::info!("QuickNote: dest cursor → {}", self.quick_note_dest_cursor);
        }

        // Enter → dispatch selected item.
        let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        if enter {
            let cursor = self.quick_note_dest_cursor;
            if cursor == 0 {
                let text = self.quick_note_text.clone();
                log::info!("QuickNote: destination selected via Enter: cursor=0 (global backlog)");
                if self.commit_quick_note_global_backlog(&text) {
                    self.pop_focus_layer(&crate::app::FocusLayer::QuickNoteDestination);
                    self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
                    self.quick_note_text.clear();
                }
                return;
            }
            let dest = self.config.quick_note.as_ref()
                .and_then(|qn| qn.destinations.get(cursor - 1).cloned());
            if let Some(dest) = dest {
                log::info!("QuickNote: destination selected via Enter: cursor={cursor} key={}", dest.key);
                if dest.options.is_some() || dest.children_cmd.is_some() {
                    let key = dest.key;
                    self.quick_note_sub_cursor = 0;
                    self.quick_note_children_cache.clear();
                    self.quick_note_children_rx = None;
                    self.push_focus_layer(crate::app::FocusLayer::QuickNoteSubDestination(vec![key]));
                    log::info!("QuickNote: submenu opened: path=[{key}]");
                    self.draw_quick_note_menu(ctx, &[key]);
                } else {
                    let text = self.quick_note_text.clone();
                    if self.commit_quick_note(&text, &dest) {
                        self.pop_focus_layer(&crate::app::FocusLayer::QuickNoteDestination);
                        self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
                        self.quick_note_text.clear();
                        self.quick_note_children_cache.clear();
                        self.quick_note_children_rx = None;
                    }
                }
            }
            return;
        }

        // L → enter submenu for selected item (zero-flash).
        let enter_sub = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::L)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
        });
        if enter_sub {
            let cursor = self.quick_note_dest_cursor;
            if cursor > 0 {
                let dest = self.config.quick_note.as_ref()
                    .and_then(|qn| qn.destinations.get(cursor - 1).cloned());
                if let Some(dest) = dest {
                    if dest.options.is_some() || dest.children_cmd.is_some() {
                        let key = dest.key;
                        self.quick_note_sub_cursor = 0;
                        self.quick_note_children_cache.clear();
                        self.quick_note_children_rx = None;
                        self.push_focus_layer(crate::app::FocusLayer::QuickNoteSubDestination(vec![key]));
                        log::info!("QuickNote: submenu opened via L: path=[{key}]");
                        self.draw_quick_note_menu(ctx, &[key]);
                        return;
                    }
                }
            }
        }

        // N → open config file to add a new destination.
        let add_dest = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::N)
        });
        if add_dest {
            log::info!("QuickNote: opening config to add destination");
            crate::config::open_config_file();
            return;
        }

        // Digit keys → fast-dispatch by number.
        let pressed = consume_digit_key(ctx);

        if let Some(key) = pressed {
            if key == 0 {
                let text = self.quick_note_text.clone();
                log::info!("QuickNote: destination selected: key=0 (global backlog)");
                if self.commit_quick_note_global_backlog(&text) {
                    self.pop_focus_layer(&crate::app::FocusLayer::QuickNoteDestination);
                    self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
                    self.quick_note_text.clear();
                }
                return;
            }
            let dest = self.config.quick_note.as_ref()
                .and_then(|qn| qn.destinations.iter().find(|d| d.key == key).cloned());
            if let Some(dest) = dest {
                log::info!("QuickNote: destination selected: key={key}");
                if dest.options.is_some() || dest.children_cmd.is_some() {
                    self.quick_note_sub_cursor = 0;
                    self.quick_note_children_cache.clear();
                    self.quick_note_children_rx = None;
                    self.push_focus_layer(crate::app::FocusLayer::QuickNoteSubDestination(vec![key]));
                    log::info!("QuickNote: submenu opened: path=[{key}]");
                    self.draw_quick_note_menu(ctx, &[key]);
                } else {
                    let text = self.quick_note_text.clone();
                    if self.commit_quick_note(&text, &dest) {
                        self.pop_focus_layer(&crate::app::FocusLayer::QuickNoteDestination);
                        self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
                        self.quick_note_text.clear();
                        self.quick_note_children_cache.clear();
                        self.quick_note_children_rx = None;
                    }
                }
            }
            return;
        }

        // Render
        let screen_rect = ctx.screen_rect();

        egui::Area::new(egui::Id::new("quick_note_scrim"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(
                    screen_rect,
                    0.0,
                    egui::Color32::from_black_alpha(230),
                );
            });

        let modal_w = (screen_rect.width() * 0.6).min(672.0).max(408.0);
        let cursor = self.quick_note_dest_cursor;
        let destinations = self.config.quick_note.as_ref()
            .map(|qn| qn.destinations.as_slice())
            .unwrap_or(&[]);
        egui::Area::new(egui::Id::new("quick_note_dest_modal"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, self.colors.border))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(24, 20))
                    .show(ui, |ui| {
                        ui.set_width(modal_w);

                        // Note preview — up to 4 lines, each truncated at 80 chars.
                        let preview = {
                            let t = self.quick_note_text.trim();
                            let mut lines: Vec<String> = t.lines()
                                .take(4)
                                .map(|line| {
                                    if let Some((idx, _)) = line.char_indices().nth(80) {
                                        format!("{}…", &line[..idx])
                                    } else {
                                        line.to_string()
                                    }
                                })
                                .collect();
                            if t.lines().count() > 4 {
                                lines.push("…".to_string());
                            }
                            lines.join("\n")
                        };
                        ui.label(
                            RichText::new(&preview)
                                .color(self.colors.text_dim.linear_multiply(0.5))
                                .size(style::TEXT_BODY)
                                .family(egui::FontFamily::Monospace),
                        );
                        ui.add_space(style::SPACE_XL);

                        ui.label(
                            RichText::new("Send to…")
                                .color(self.colors.text_primary)
                                .size(style::TEXT_BODY),
                        );
                        ui.add_space(style::SPACE_MD);

                        // Row 0: global backlog
                        let row0_frame = if cursor == 0 {
                            egui::Frame::new()
                                .fill(self.colors.bg_active)
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(egui::Margin::symmetric(4, 2))
                        } else {
                            egui::Frame::new()
                                .inner_margin(egui::Margin::symmetric(4, 2))
                        };
                        row0_frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = style::SPACE_SM;
                                widgets::key_chip(ui, "0", &self.colors);
                                ui.label(
                                    RichText::new("Backlog (global)")
                                        .color(self.colors.text_dim)
                                        .size(style::TEXT_BODY),
                                );
                            });
                        });
                        ui.add_space(style::SPACE_SM);

                        // Config destinations
                        for (idx, dest) in destinations.iter().enumerate() {
                            let row_idx = idx + 1;
                            let label = if dest.options.is_some() || dest.children_cmd.is_some() {
                                format!("{} ›", dest.label)
                            } else {
                                dest.label.clone()
                            };
                            let row_frame = if cursor == row_idx {
                                egui::Frame::new()
                                    .fill(self.colors.bg_active)
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::symmetric(4, 2))
                            } else {
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(4, 2))
                            };
                            row_frame.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = style::SPACE_SM;
                                    widgets::key_chip(ui, &dest.key.to_string(), &self.colors);
                                    ui.label(
                                        RichText::new(&label)
                                            .color(self.colors.text_dim)
                                            .size(style::TEXT_BODY),
                                    );
                                });
                            });
                            ui.add_space(style::SPACE_SM);
                        }

                        ui.add_space(style::SPACE_XL);
                        ui.label(
                            RichText::new("↑↓/jk navigate  ·  Enter select  ·  n add dest  ·  H/Esc back")
                                .color(self.colors.text_dim.linear_multiply(0.4))
                                .size(style::TEXT_HINT)
                                .family(egui::FontFamily::Monospace),
                        );
                    });
            });
    }

    /// Recursive quick-note menu renderer. `key_path` is the sequence of keys from the
    /// root destinations list to the current node. E.g. `&[3]` = children of destination 3.
    pub(crate) fn draw_quick_note_menu(&mut self, ctx: &egui::Context, key_path: &[u8]) {
        use crate::style;
        use crate::widgets;
        use egui::{Align2, RichText, Vec2};

        // Resolve the current node by walking key_path through the config tree.
        let node: Option<crate::config::QuickNoteNode> = {
            let mut current: Option<&Vec<crate::config::QuickNoteNode>> =
                self.config.quick_note.as_ref().map(|qn| &qn.destinations);
            let mut found = None;
            for &key in key_path {
                if let Some(nodes) = current {
                    if let Some(n) = nodes.iter().find(|n| n.key == key) {
                        found = Some(n.clone());
                        current = n.options.as_ref();
                    } else {
                        found = None;
                        break;
                    }
                } else {
                    found = None;
                    break;
                }
            }
            found
        };

        let current_layer = crate::app::FocusLayer::QuickNoteSubDestination(key_path.to_vec());

        // ── Drain children_cmd receiver if pending ────────────────────────────────
        // Cache key is the full key_path to avoid collisions when different submenus
        // share child keys (e.g. both have a child with key=1).
        let node_path = key_path.to_vec();
        {
            let mut received = None;
            if let Some((pending_path, rx)) = &self.quick_note_children_rx {
                if pending_path == &node_path {
                    match rx.try_recv() {
                        Ok(Ok(children)) => {
                            log::info!(
                                "QuickNote: children_cmd result received for path={:?} count={}",
                                node_path, children.len()
                            );
                            received = Some((node_path.clone(), Ok(children)));
                        }
                        Ok(Err(e)) => {
                            log::error!("QuickNote: children_cmd failed for path={:?}: {e}", node_path);
                            received = Some((node_path.clone(), Err(e)));
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {}
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            log::error!("QuickNote: children_cmd sender dropped for path={:?}", node_path);
                            received = Some((node_path.clone(), Err("sender dropped".to_string())));
                        }
                    }
                }
            }
            if let Some((path, result)) = received {
                self.quick_note_children_rx = None;
                match result {
                    Ok(children) => { self.quick_note_children_cache.insert(path, children); }
                    Err(_) => { self.quick_note_children_cache.insert(path, vec![]); }
                }
                self.quick_note_sub_cursor = 0;
            }
        }

        // Determine the list of children to display.
        // Priority: static options > dynamic children_cmd results.
        let static_options = node.as_ref().and_then(|n| n.options.as_ref());
        let dynamic_children = if static_options.is_none() {
            self.quick_note_children_cache.get(&node_path).cloned()
        } else {
            None
        };
        let loading = static_options.is_none()
            && dynamic_children.is_none()
            && self.quick_note_children_rx.as_ref().map(|(p, _)| p == &node_path).unwrap_or(false);

        // Kick off children_cmd loading if needed and not already in flight.
        if static_options.is_none() && dynamic_children.is_none() && !loading {
            if let Some(n) = &node {
                if let Some(cmd_template) = &n.children_cmd {
                    let cmd = Self::substitute_note_tokens_static(
                        cmd_template,
                        &self.quick_note_text,
                        &self.quick_note_ctx,
                    );
                    let (tx, rx) = std::sync::mpsc::channel();
                    let path_for_log = node_path.clone();
                    std::thread::spawn(move || {
                        let result = std::process::Command::new("sh")
                            .args(["-c", &cmd])
                            .output()
                            .map_err(|e| e.to_string())
                            .and_then(|out| {
                                if out.status.success() {
                                    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                                    let children = parse_children_cmd_output(&stdout);
                                    Ok(children)
                                } else {
                                    Err(String::from_utf8_lossy(&out.stderr).to_string())
                                }
                            });
                        let _ = tx.send(result);
                    });
                    log::info!("QuickNote: children_cmd spawned for path={:?}", path_for_log);
                    self.quick_note_children_rx = Some((node_path, rx));
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
            }
        }

        // Resolve effective children list (empty if loading or no children).
        let children: Vec<crate::config::QuickNoteNode> = if let Some(opts) = static_options {
            opts.clone()
        } else if let Some(dyn_children) = dynamic_children {
            dyn_children
        } else {
            vec![]
        };

        let total = children.len();

        // ── Input handling ────────────────────────────────────────────────────────

        // H or Esc → navigate up.
        let back = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::H)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
        });
        if back {
            self.pop_focus_layer(&current_layer);
            log::info!("QuickNote: menu back: path={key_path:?}");
            if key_path.len() > 1 {
                let parent_path = key_path[..key_path.len() - 1].to_vec();
                self.draw_quick_note_menu(ctx, &parent_path);
            } else {
                self.draw_quick_note_destination(ctx);
            }
            return;
        }

        // Arrow / vim navigation (only when children are available).
        if !children.is_empty() {
            let up = ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::K)
            });
            let down = ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::J)
            });
            if up {
                self.quick_note_sub_cursor = if self.quick_note_sub_cursor == 0 {
                    total.saturating_sub(1)
                } else {
                    self.quick_note_sub_cursor - 1
                };
                log::info!("QuickNote: menu cursor → {}", self.quick_note_sub_cursor);
            }
            if down {
                self.quick_note_sub_cursor = (self.quick_note_sub_cursor + 1) % total.max(1);
                log::info!("QuickNote: menu cursor → {}", self.quick_note_sub_cursor);
            }
        }

        // Enter → dispatch selected child.
        let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        if enter && !children.is_empty() {
            if let Some(child) = children.get(self.quick_note_sub_cursor).cloned() {
                let key_path_owned = key_path.to_vec();
                self.dispatch_menu_child(ctx, &key_path_owned, &child);
            }
            return;
        }

        // L → enter submenu for selected child.
        let enter_sub = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::L)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
        });
        if enter_sub && !children.is_empty() {
            if let Some(child) = children.get(self.quick_note_sub_cursor).cloned() {
                if child.options.is_some() || child.children_cmd.is_some() {
                    let mut new_path = key_path.to_vec();
                    new_path.push(child.key);
                    self.push_focus_layer(crate::app::FocusLayer::QuickNoteSubDestination(new_path.clone()));
                    self.quick_note_sub_cursor = 0;
                    log::info!("QuickNote: menu enter via L: path={new_path:?}");
                    self.draw_quick_note_menu(ctx, &new_path);
                    return;
                }
            }
        }

        // N → open config file to add a new destination.
        let add_dest = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, egui::Key::N)
        });
        if add_dest {
            log::info!("QuickNote: opening config to add destination (from submenu)");
            crate::config::open_config_file();
            return;
        }

        // Digit keys → fast-dispatch by number.
        let pressed = consume_digit_key(ctx);
        if let Some(key) = pressed {
            if let Some(child) = children.iter().find(|c| c.key == key).cloned() {
                let key_path_owned = key_path.to_vec();
                self.dispatch_menu_child(ctx, &key_path_owned, &child);
            }
            return;
        }

        // ── Render ────────────────────────────────────────────────────────────────
        let screen_rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("quick_note_scrim"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen_rect, 0.0, egui::Color32::from_black_alpha(230));
            });

        let modal_w = (screen_rect.width() * 0.6).min(672.0).max(408.0);
        let sub_cursor = self.quick_note_sub_cursor;
        let node_label = node.as_ref().map(|n| n.label.as_str()).unwrap_or("Menu");

        egui::Area::new(egui::Id::new("quick_note_sub_modal"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, self.colors.border))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(egui::Margin::symmetric(24, 20))
                    .show(ui, |ui| {
                        ui.set_width(modal_w);

                        ui.label(
                            RichText::new(node_label)
                                .color(self.colors.text_primary)
                                .size(style::TEXT_BODY)
                                .strong(),
                        );
                        ui.add_space(style::SPACE_MD);

                        if loading {
                            ui.label(
                                RichText::new("Loading…")
                                    .color(self.colors.text_dim)
                                    .size(style::TEXT_BODY),
                            );
                        } else if children.is_empty() {
                            ui.label(
                                RichText::new("No items")
                                    .color(self.colors.text_dim.linear_multiply(0.5))
                                    .size(style::TEXT_BODY),
                            );
                        } else {
                            for (idx, child) in children.iter().enumerate() {
                                let child_label = if child.options.is_some() || child.children_cmd.is_some() {
                                    format!("{} ›", child.label)
                                } else {
                                    child.label.clone()
                                };
                                let row_frame = if sub_cursor == idx {
                                    egui::Frame::new()
                                        .fill(self.colors.bg_active)
                                        .corner_radius(egui::CornerRadius::same(4))
                                        .inner_margin(egui::Margin::symmetric(4, 2))
                                } else {
                                    egui::Frame::new()
                                        .inner_margin(egui::Margin::symmetric(4, 2))
                                };
                                row_frame.show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = style::SPACE_SM;
                                        widgets::key_chip(ui, &child.key.to_string(), &self.colors);
                                        ui.label(
                                            RichText::new(&child_label)
                                                .color(self.colors.text_dim)
                                                .size(style::TEXT_BODY),
                                        );
                                    });
                                });
                                ui.add_space(style::SPACE_SM);
                            }
                        }

                        ui.add_space(style::SPACE_XL);
                        ui.label(
                            RichText::new("↑↓/jk navigate  ·  Enter select  ·  L enter  ·  n add dest  ·  H/Esc back")
                                .color(self.colors.text_dim.linear_multiply(0.4))
                                .size(style::TEXT_HINT)
                                .family(egui::FontFamily::Monospace),
                        );
                    });
            });
    }

    /// Dispatch a child node selected from within a menu at `parent_path`.
    /// Handles: entering deeper submenus, committing leaf commands.
    fn dispatch_menu_child(
        &mut self,
        ctx: &egui::Context,
        parent_path: &[u8],
        child: &crate::config::QuickNoteNode,
    ) {
        if child.options.is_some() || child.children_cmd.is_some() {
            // Submenu — navigate deeper.
            let mut new_path = parent_path.to_vec();
            new_path.push(child.key);
            self.push_focus_layer(crate::app::FocusLayer::QuickNoteSubDestination(new_path.clone()));
            self.quick_note_sub_cursor = 0;
            log::info!("QuickNote: menu enter: path={new_path:?}");
            self.draw_quick_note_menu(ctx, &new_path);
        } else {
            // Leaf — commit.
            let text = self.quick_note_text.clone();
            log::info!(
                "QuickNote: menu commit: path={:?} key={} label='{}'",
                parent_path, child.key, child.label
            );
            if self.commit_quick_note(&text, child) {
                // Pop all submenu layers + destination + compose layers.
                while matches!(
                    self.focus_stack.last(),
                    Some(crate::app::FocusLayer::QuickNoteSubDestination(_))
                ) {
                    self.focus_stack.pop();
                }
                self.pop_focus_layer(&crate::app::FocusLayer::QuickNoteDestination);
                self.pop_focus_layer(&crate::app::FocusLayer::QuickNote);
                self.quick_note_text.clear();
                self.quick_note_children_cache.clear();
                self.quick_note_children_rx = None;
            }
        }
    }

    pub(crate) fn inspector_pane_order(&self) -> Vec<PaneId> {
        self.collect_inspector_rows().1
    }

    fn collect_inspector_rows(&self) -> (Vec<(String, u64, Vec<PaneRow>)>, Vec<PaneId>, Vec<u64>) {
        let mut groups: Vec<(String, u64, Vec<PaneRow>)> = Vec::new();
        let mut all_pane_ids: Vec<PaneId> = Vec::new();
        let mut all_context_ids: Vec<u64> = Vec::new();
        for ctx_entry in self.router.iter() {
            let cname = ctx_entry.name.clone();
            let cid = ctx_entry.context_id;
            let mut rows: Vec<PaneRow> = Vec::new();
            for win in self.windows.iter() {
                if win.context_id != cid {
                    continue;
                }
                for (_, pane) in &win.panes {
                    match pane {
                        crate::pane::Pane::Terminal(t) => {
                            let pane_name = t.name.clone().or_else(|| t.pty_title.clone()).unwrap_or_default();
                            // Show pty_title in detail only when it adds context beyond the displayed name.
                            let pane_detail = t.pty_title.as_deref()
                                .filter(|pt| !pt.is_empty() && *pt != pane_name.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_default();
                            let (status, osc_badge) = if t.exited {
                                ("exited", None)
                            } else {
                                let busy = crate::shell::has_foreground_child(t.backend.child_pid());
                                // Show the OSC title as the badge so the user can see what's running.
                                let badge = if busy {
                                    t.pty_title.as_deref()
                                        .filter(|t| !t.is_empty())
                                        .map(|t| t.to_string())
                                } else {
                                    None
                                };
                                let status = if busy { "busy" } else { "idle" };
                                (status, badge)
                            };
                            rows.push(PaneRow {
                                id: t.id,
                                kind: "Terminal",
                                name: pane_name,
                                detail: pane_detail,
                                status,
                                osc_badge,
                            });
                        }
                        crate::pane::Pane::App(a) => {
                            let status = if let crate::pane::AppRuntime::Process(ref proc) = a.runtime {
                                match proc.lifecycle.state() {
                                    crate::process_app::LifecycleState::Running => "running",
                                    crate::process_app::LifecycleState::Booting => "booting",
                                    crate::process_app::LifecycleState::Crashed => "crashed",
                                    crate::process_app::LifecycleState::Hung => "hung",
                                    crate::process_app::LifecycleState::ProtocolError => "error",
                                }
                            } else {
                                "active"
                            };
                            rows.push(PaneRow {
                                id: a.id,
                                kind: "App",
                                name: a.name.clone(),
                                detail: a.manifest_id.clone(),
                                status,
                                osc_badge: None,
                            });
                        }
                        crate::pane::Pane::Portal(p) => {
                            rows.push(PaneRow {
                                id: p.pane_id,
                                kind: "Portal",
                                name: format!("portal:{}", p.target_context_id),
                                detail: String::new(),
                                status: "active",
                                osc_badge: None,
                            });
                        }
                    }
                }
            }
            if !rows.is_empty() {
                rows.sort_by_key(|r| r.id);
                for r in &rows {
                    all_pane_ids.push(r.id);
                    all_context_ids.push(cid);
                }
                groups.push((cname, cid, rows));
            }
        }
        (groups, all_pane_ids, all_context_ids)
    }

    pub(crate) fn draw_context_inspector(&mut self, ctx: &egui::Context) {
        let mut dismissed = false;
        let mut close_pane: Option<PaneId> = None;
        let mut focus_pane: Option<PaneId> = None;
        let mut delete_context = false;
        let mut open_root_overlay = false;
        let mut open_description_overlay = false;
        let mut start_rename = false;
        let mut commit_rename = false;
        let mut cancel_rename = false;
        let mut set_root_from_pane = false;

        let renaming = self.inspector_renaming;
        let num_contexts = self.router.len();
        let (nav_down, nav_up, enter_pressed, backspace_pressed, cmd_w_pressed) = ctx.input_mut(|i| {
            if renaming {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                    commit_rename = true;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    cancel_rename = true;
                }
                (false, false, false, false, false)
            } else {
                let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                let down = i.consume_key(egui::Modifiers::NONE, egui::Key::J)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
                let up = i.consume_key(egui::Modifiers::NONE, egui::Key::K)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
                let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                let r = i.consume_key(egui::Modifiers::COMMAND, egui::Key::R);
                let backspace = num_contexts > 1 && (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
                        || i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
                );
                let cmd_w = i.consume_key(egui::Modifiers::COMMAND, egui::Key::W);
                if esc { dismissed = true; }
                if r { start_rename = true; }
                (down, up, enter, backspace, cmd_w)
            }
        });

        let focused_cwd = self.windows.get(self.active_window)
            .and_then(|w| w.focused_pane.map(|tile| (w, tile)))
            .and_then(|(w, tile)| w.get_focused_pane_cwd(tile));

        let (groups, all_pane_ids, all_context_ids) = self.collect_inspector_rows();
        let pane_count = all_pane_ids.len();
        let selected_ctx_idx = all_context_ids
            .get(self.inspector_selected_pane)
            .and_then(|&cid| self.router.position(|c| c.context_id == cid))
            .unwrap_or_else(|| self.router.active_idx());
        let ctx_name = self.router.get(selected_ctx_idx).name.clone();
        let ctx_root = self.router.get(selected_ctx_idx).root.clone();
        let ctx_description = self.router.get(selected_ctx_idx).description.clone();
        let active_cid = self.router.active().context_id;

        if nav_down && pane_count > 0 {
            self.inspector_selected_pane = (self.inspector_selected_pane + 1) % pane_count;
        }
        if nav_up && pane_count > 0 {
            self.inspector_selected_pane =
                (self.inspector_selected_pane + pane_count - 1) % pane_count;
        }
        if self.inspector_selected_pane >= pane_count && pane_count > 0 {
            self.inspector_selected_pane = pane_count - 1;
        }
        if enter_pressed && pane_count > 0 {
            focus_pane = Some(all_pane_ids[self.inspector_selected_pane]);
        }
        if cmd_w_pressed && pane_count > 0 {
            log::info!("ContextInspector: ⌘W close pane {}", all_pane_ids[self.inspector_selected_pane]);
            close_pane = Some(all_pane_ids[self.inspector_selected_pane]);
        }
        if backspace_pressed {
            let now = std::time::Instant::now();
            let elapsed = self
                .inspector_delete_last_press
                .map(|t| now.duration_since(t))
                .unwrap_or(std::time::Duration::MAX);
            if elapsed > std::time::Duration::from_millis(CONFIRM_TIMEOUT_MS) {
                self.inspector_delete_press_count = 0;
            }
            self.inspector_delete_press_count += 1;
            self.inspector_delete_last_press = Some(now);
            log::info!(
                "ContextInspector: backspace press {} of 3 for context {:?}",
                self.inspector_delete_press_count,
                self.router.active().name
            );
            if self.inspector_delete_press_count >= 3 {
                self.inspector_delete_press_count = 0;
                self.inspector_delete_last_press = None;
                delete_context = true;
            }
        }

        let colors = self.colors;
        let selected = self.inspector_selected_pane;
        let screen_rect = ctx.screen_rect();

        egui::Area::new(egui::Id::new("context_inspector_scrim"))
            .fixed_pos(screen_rect.min).order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen_rect, 0.0, Color32::from_black_alpha(style::SCRIM_ALPHA));
                if ui.allocate_rect(screen_rect, egui::Sense::click()).clicked() { dismissed = true; }
            });

        egui::Area::new(egui::Id::new("context_inspector_overlay"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, colors.border))
                    .corner_radius(style::RADIUS_MD)
                    .inner_margin(egui::Margin::symmetric(
                        style::MODAL_PADDING_H,
                        style::MODAL_PADDING_V,
                    ))
                    .show(ui, |ui| {
                        ui.set_width(style::MODAL_WIDTH_MD);
                        let (want_root, want_desc, rename_clicked, close_clicked, pane_dir_clicked) =
                            render_inspector_header(
                                ui,
                                ctx_name.as_str(),
                                &ctx_root,
                                &ctx_description,
                                focused_cwd.as_deref(),
                                &colors,
                                renaming,
                                &mut self.rename_buffer,
                            );
                        if want_root { open_root_overlay = true; }
                        if want_desc { open_description_overlay = true; }
                        if rename_clicked { start_rename = true; }
                        if close_clicked { dismissed = true; }
                        if pane_dir_clicked { set_root_from_pane = true; }
                        // While renaming, hide the pane list — interactive widgets
                        // rendered after the TextEdit can steal egui focus (last-caller-wins).
                        // See GOTCHAS.md "egui TextEdit focus in complex frames".
                        if !renaming {
                            egui::ScrollArea::vertical()
                                .max_height(ctx.available_rect().height() * 0.6)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    if pane_count == 0 {
                                        ui.label(RichText::new("No panes").size(style::TEXT_BODY).color(colors.text_dim));
                                    } else {
                                        let mut global_idx: usize = 0;
                                        for (group_name, group_cid, rows) in &groups {
                                            let is_active = *group_cid == active_cid;
                                            ui.add_space(style::SPACE_SM);
                                            let group_label = RichText::new(group_name.as_str()).size(style::TEXT_CAPTION);
                                            ui.label(if is_active {
                                                group_label.color(colors.text_primary).strong()
                                            } else {
                                                group_label.color(colors.text_dim)
                                            });
                                            ui.add_space(style::SPACE_SM);
                                            for row in rows {
                                                let resp = render_inspector_pane_row(
                                                    ui, row, global_idx == selected, &colors,
                                                );
                                                global_idx += 1;
                                                if resp.clicked() { focus_pane = Some(row.id); }
                                            }
                                        }
                                    }
                                });
                            ui.add_space(style::SPACE_XL);
                            ui.separator();
                            ui.add_space(style::SPACE_MD);
                        }
                        render_inspector_hints(ui, pane_count, num_contexts, &colors, renaming);
                    });
            });

        // One-shot focus: request AFTER all UI renders so we win egui's
        // last-caller-wins focus contest. See GOTCHAS.md for the pattern.
        if renaming && !self.inspector_rename_focus_requested {
            let te_id = egui::Id::new("inspector_rename_input");
            ctx.memory_mut(|m| m.request_focus(te_id));
            if let Some(mut state) = egui::TextEdit::load_state(ctx, te_id) {
                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(self.rename_buffer.chars().count()),
                )));
                state.store(ctx, te_id);
            }
            self.inspector_rename_focus_requested = true;
            log::info!("ContextInspector: rename TextEdit focus requested (one-shot)");
        }

        if start_rename && !renaming {
            self.rename_buffer = ctx_name.to_string();
            self.inspector_renaming = true;
            self.inspector_rename_focus_requested = false;
            log::info!("ContextInspector: rename mode entered for context {:?}", ctx_name);
        }
        if commit_rename {
            let new_name = self.rename_buffer.trim().to_string();
            if !new_name.is_empty() {
                log::info!(
                    "ContextInspector: renamed context {:?} → {:?}",
                    self.router.get(selected_ctx_idx).name,
                    new_name
                );
                self.router.get_mut(selected_ctx_idx).name = new_name;
                self.save_workspace();
            }
            self.inspector_renaming = false;
            self.inspector_rename_focus_requested = false;
        }
        if cancel_rename {
            self.inspector_renaming = false;
            self.inspector_rename_focus_requested = false;
            log::info!("ContextInspector: rename cancelled");
        }
        if dismissed {
            self.show_context_inspector = false;
            self.inspector_renaming = false;
            self.inspector_rename_focus_requested = false;
            self.inspector_delete_press_count = 0;
            self.inspector_delete_last_press = None;
            log::info!("ContextInspector: closed");
        }
        if let Some(pid) = focus_pane {
            log::info!("ContextInspector: focusing pane {pid}");
            self.pane_navigate(pid);
            self.show_context_inspector = false;
            self.inspector_renaming = false;
            self.inspector_rename_focus_requested = false;
            self.inspector_delete_press_count = 0;
            self.inspector_delete_last_press = None;
        }
        if let Some(pid) = close_pane {
            log::info!("ContextInspector: closing pane {pid}");
            self.close_pane_by_id(pid);
        }
        if delete_context {
            let ctx_idx = if pane_count > 0 {
                let target_cid = all_context_ids[self.inspector_selected_pane];
                self.router
                    .position(|c| c.context_id == target_cid)
                    .unwrap_or_else(|| self.router.active_idx())
            } else {
                self.router.active_idx()
            };
            log::info!(
                "ContextInspector: deleting context idx={ctx_idx} name={:?} (via backspace or button)",
                self.router.get(ctx_idx).name
            );
            self.inspector_delete_press_count = 0;
            self.inspector_delete_last_press = None;
            self.delete_context(ctx_idx);
            self.save_workspace();
        }
        if open_root_overlay {
            let idx = self.router.active_idx();
            let existing = self.router.get(idx).root.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            log::info!("TextInputOverlay: opened target=ContextRoot({idx})");
            self.text_overlay_browse_rx = None;
            self.text_overlay = Some((
                crate::app::TextInputOverlay {
                    label: "Set context root".to_string(),
                    hint: "/path/to/project or ~/...".to_string(),
                    buffer: existing,
                    focus_requested: false,
                },
                crate::app::OverlayTarget::ContextRoot(idx),
            ));
            // Close the inspector so the text overlay takes focus.
            self.show_context_inspector = false;
        }
        if open_description_overlay {
            let idx = selected_ctx_idx;
            let existing = self.router.get(idx).description.clone().unwrap_or_default();
            log::info!("ContextInspector: opening description overlay for ctx_idx={idx}");
            self.editing_description = Some(idx);
            self.description_buffer = existing;
            self.description_focus_requested = false;
            self.push_focus_layer(crate::app::FocusLayer::ContextDescription);
            self.show_context_inspector = false;
        }
        if set_root_from_pane {
            if let Some(cwd) = focused_cwd {
                log::info!(
                    "ContextInspector: set root from pane cwd={} ctx_idx={selected_ctx_idx}",
                    cwd.display()
                );
                self.router.get_mut(selected_ctx_idx).root = Some(cwd);
                self.save_workspace();
            }
        }

        if self.inspector_delete_press_count > 0 {
            let timed_out = self
                .inspector_delete_last_press
                .map(|t| t.elapsed() > std::time::Duration::from_millis(CONFIRM_TIMEOUT_MS))
                .unwrap_or(false);
            if timed_out {
                self.inspector_delete_press_count = 0;
                self.inspector_delete_last_press = None;
            } else {
                self.draw_inspector_delete_overlay(ctx);
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
    }

    fn draw_triple_tap_overlay(&self, ctx: &egui::Context, id: &str, count: u8, label: &str) {
        egui::Area::new(egui::Id::new(id))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -40.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{label} {} of 3 — press again to delete context",
                                    count
                                ))
                                .size(12.0)
                                .color(self.colors.text_dim),
                            );
                            ui.add_space(8.0);
                            for i in 1u8..=3 {
                                let color = if i <= count {
                                    self.colors.accent
                                } else {
                                    self.colors.bg_active
                                };
                                let (rect, _) = ui.allocate_exact_size(
                                    Vec2::new(8.0, 8.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(rect.center(), 4.0, color);
                            }
                        });
                    });
            });
    }

    pub(crate) fn draw_inspector_delete_overlay(&self, ctx: &egui::Context) {
        self.draw_triple_tap_overlay(
            ctx,
            "inspector_delete_overlay",
            self.inspector_delete_press_count,
            "⌫ pressed",
        );
    }

    pub(crate) fn draw_welcome_delete_overlay(&self, ctx: &egui::Context) {
        self.draw_triple_tap_overlay(
            ctx,
            "welcome_delete_overlay",
            self.welcome_delete_press_count,
            "⌫ pressed",
        );
    }

    pub(crate) fn draw_quit_confirm_overlay(&self, ctx: &egui::Context) {
        let count = self.quit_press_count;
        egui::Area::new(egui::Id::new("quit_confirm_overlay"))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -40.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "\u{2318}Q pressed {} of 3 — press again to quit",
                                    count
                                ))
                                .size(12.0)
                                .color(self.colors.text_dim),
                            );
                            ui.add_space(8.0);
                            for i in 1u8..=3 {
                                let color = if i <= count {
                                    self.colors.accent
                                } else {
                                    self.colors.bg_active
                                };
                                let (rect, _) = ui.allocate_exact_size(
                                    Vec2::new(8.0, 8.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter()
                                    .circle_filled(rect.center(), 4.0, color);
                            }
                        });
                    });
            });
    }

    pub(crate) fn draw_confirm_close(&mut self, ctx: &egui::Context) {
        let mut confirmed = false;
        let mut cancelled = false;

        // Consume Enter/Escape at the context level so they cannot bleed
        // through to the focused pane this frame. `ui.input(|i| key_pressed)`
        // is a *read-only* check — it does not remove the event — which is
        // how this modal used to leak a confirming Enter into the pane behind
        // it (e.g. the backlog app opened the selected note on Cmd+W → Enter).
        // Combined with the `FocusLayer::ConfirmClose` capture, this is the
        // systemic fix: overlay owns focus, overlay drains its own keys.
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                confirmed = true;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                cancelled = true;
            }
        });

        // Scrim — visually reinforces modal capture and mirrors the command
        // palette / notification modal chrome.
        let screen_rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("confirm_close_scrim"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(
                    screen_rect,
                    0.0,
                    egui::Color32::from_black_alpha(120),
                );
                let scrim_response = ui.allocate_rect(screen_rect, egui::Sense::click());
                if scrim_response.clicked() {
                    cancelled = true;
                }
            });

        egui::Area::new(egui::Id::new("confirm_close_overlay"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(20, 16))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);
                        ui.label(
                            RichText::new("Close pane?")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("The running process will be terminated.")
                                .size(12.0)
                                .color(self.colors.text_dim),
                        );
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Close")
                                            .size(12.0)
                                            .color(self.colors.text_primary),
                                    )
                                    .fill(self.colors.bg_active),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                confirmed = true;
                            }
                            ui.add_space(8.0);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Cancel")
                                            .size(12.0)
                                            .color(self.colors.text_dim),
                                    )
                                    .frame(false),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                cancelled = true;
                            }
                            ui.add_space(12.0);
                            crate::widgets::key_chip(ui, "Enter", &self.colors);
                            ui.label(
                                RichText::new("confirm")
                                    .size(style::TEXT_HINT)
                                    .color(self.colors.text_dim),
                            );
                            ui.add_space(style::SPACE_SM);
                            crate::widgets::key_chip(ui, "Esc", &self.colors);
                            ui.label(
                                RichText::new("cancel")
                                    .size(style::TEXT_HINT)
                                    .color(self.colors.text_dim),
                            );
                        });
                    });
            });

        if confirmed {
            self.pending_close = false;
            if self.execute_close_pane() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                self.save_workspace();
            }
        } else if cancelled {
            self.pending_close = false;
        }
    }

    /// Capability / secret consent modal for the focused ProcessApp pane.
    /// Called from step 2 of `update()` so it holds exclusive keyboard
    /// ownership before `dispatch_app_key_events` runs.
    pub(crate) fn draw_capability_modal(&mut self, ctx: &egui::Context) {
        let active = self.active_window;
        let colors = self.colors;

        // Resolve focused pane id — bail if it's not a ProcessApp.
        // Use find_pane_in_tile so we traverse through any Container wrapper that
        // egui_tiles may have inserted around a bare-pane root after first render.
        let pane_id = {
            let win = &self.windows[active];
            let focused_tile = match win.focused_pane {
                Some(t) => t,
                None => return,
            };
            match crate::app::PlexiApp::find_pane_in_tile(&win.tree, focused_tile) {
                Some(id) => id,
                None => return,
            }
        };

        // Take fields out of the ProcessApp, call the modal, put them back.
        // Two separate borrows so Rust doesn't see a conflict.
        let (mut pending_prompts, mut outbound_events, mut permissions,
             mut secret_input_buf, mut permission_store, type_id, workspace_root) = {
            let pane = match self.windows[active].panes.get_mut(&pane_id) {
                Some(crate::pane::Pane::App(a)) => a,
                _ => return,
            };
            let crate::pane::AppRuntime::Process(ref mut proc) = pane.runtime else { return };
            if proc.pending_prompts.is_empty() {
                return;
            }
            (
                std::mem::take(&mut proc.pending_prompts),
                std::mem::take(&mut proc.outbound_events),
                std::mem::take(&mut proc.permissions),
                std::mem::take(&mut proc.secret_input_buf),
                std::mem::take(&mut proc.permission_store),
                proc.type_id.clone(),
                proc.workspace_root.clone(),
            )
        };

        let config_dir = crate::config::config_dir();
        crate::process_app::prompts::show_prompt_modal(
            ctx,
            &mut pending_prompts,
            &mut outbound_events,
            &mut permissions,
            &type_id,
            &workspace_root,
            &mut secret_input_buf,
            &config_dir,
            &mut permission_store,
            &colors,
        );

        // Put the data back.
        let pane = match self.windows[active].panes.get_mut(&pane_id) {
            Some(crate::pane::Pane::App(a)) => a,
            _ => return,
        };
        let crate::pane::AppRuntime::Process(ref mut proc) = pane.runtime else { return };
        proc.pending_prompts = pending_prompts;
        proc.outbound_events = outbound_events;
        proc.permissions = permissions;
        proc.secret_input_buf = secret_input_buf;
        proc.permission_store = permission_store;
    }

    /// Context-close confirmation dialog. Shows pane inventory with three choices:
    /// Close All (Enter), Dissolve (D), Cancel (Escape).
    pub(crate) fn draw_context_close_confirm(&mut self, ctx: &egui::Context) {
        let state = match self.pending_context_close.take() {
            Some(s) => s,
            None => return,
        };

        let mut close_all = false;
        let mut dissolve = false;
        let mut cancelled = false;

        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                close_all = true;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::D) {
                dissolve = true;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                cancelled = true;
            }
        });

        let screen_rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("ctx_close_confirm_scrim"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen_rect, 0.0, egui::Color32::from_black_alpha(120));
                let scrim_resp = ui.allocate_rect(screen_rect, egui::Sense::click());
                if scrim_resp.clicked() {
                    cancelled = true;
                }
            });

        let colors = self.colors;
        egui::Area::new(egui::Id::new("ctx_close_confirm_modal"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(20, 16))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);

                        let title = if state.context_name.is_empty() {
                            "Close context?".to_string()
                        } else {
                            format!("Close \"{}\"?", state.context_name)
                        };
                        ui.label(
                            RichText::new(&title)
                                .size(13.0)
                                .color(colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(8.0);

                        for item in &state.items {
                            let label = format!("{} — {}", item.kind, item.name);
                            ui.label(
                                RichText::new(&label)
                                    .size(style::TEXT_HINT)
                                    .color(colors.text_dim),
                            );
                        }

                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Close all")
                                            .size(12.0)
                                            .color(colors.text_primary),
                                    )
                                    .fill(colors.bg_active),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                close_all = true;
                            }
                            ui.add_space(6.0);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Dissolve")
                                            .size(12.0)
                                            .color(colors.text_primary),
                                    )
                                    .fill(colors.bg_active),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                dissolve = true;
                            }
                            ui.add_space(6.0);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Cancel")
                                            .size(12.0)
                                            .color(colors.text_dim),
                                    )
                                    .frame(false),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                cancelled = true;
                            }
                        });

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            crate::widgets::key_chip(ui, "Enter", &colors);
                            ui.label(
                                RichText::new("close all")
                                    .size(style::TEXT_HINT)
                                    .color(colors.text_dim),
                            );
                            ui.add_space(style::SPACE_SM);
                            crate::widgets::key_chip(ui, "D", &colors);
                            ui.label(
                                RichText::new("dissolve")
                                    .size(style::TEXT_HINT)
                                    .color(colors.text_dim),
                            );
                            ui.add_space(style::SPACE_SM);
                            crate::widgets::key_chip(ui, "Esc", &colors);
                            ui.label(
                                RichText::new("cancel")
                                    .size(style::TEXT_HINT)
                                    .color(colors.text_dim),
                            );
                        });
                    });
            });

        if close_all {
            let idx = self.router.iter().position(|c| c.context_id == state.context_id);
            if let Some(i) = idx {
                log::info!("context_close: close_all ctx={} name={:?}", state.context_id, state.context_name);
                self.delete_context(i);
                self.save_workspace();
            } else {
                log::warn!("context_close: close_all ctx={} not found in router", state.context_id);
            }
        } else if dissolve {
            log::info!("context_close: dissolve ctx={} name={:?}", state.context_id, state.context_name);
            self.dissolve_portal(state.context_id);
            self.save_workspace();
        } else if cancelled {
            log::info!("context_close: cancelled ctx={}", state.context_id);
        } else {
            // No input yet — put state back for the next frame.
            self.pending_context_close = Some(state);
        }
    }

    /// Primary notification surface: a keyboard-first centered modal over the
    /// work area. Renders the front of the queue.
    ///
    /// Enter / option-select / input-submit dispatches exactly one
    /// `DeliverNotifyAction` and removes the notification from the queue.
    /// Esc defers: the modal closes but the notification stays in the
    /// queue (reopen with Cmd+Shift+A to come back to it). No
    /// NotifyAction is dispatched on Esc — the app hasn't been answered.
    /// Required notifications reject Esc.
    ///
    /// Input map (all kinds):
    ///   Esc        — defer (only when `required == false`); keeps in queue
    ///
    /// `kind = message`:
    ///   Enter | Space — Acknowledge
    ///
    /// `kind = choice`:
    ///   ↑/↓ or j/k   — move focus
    ///   Enter | Space — confirm focused option
    ///   1-9          — direct-select the Nth option
    ///   per-option `shortcut` — direct-select that option
    ///
    /// `kind = input`:
    ///   text typing  — edits buffer
    ///   Enter        — submit (only if non-empty OR `required == false`)
    pub(crate) fn draw_notification_modal(&mut self, ctx: &egui::Context) -> Vec<AppCommand> {
        use crate::app_protocol::NotifyKind;
        use crate::app::notification_image;

        let mut cmds: Vec<AppCommand> = Vec::new();

        // Resolve the currently-displayed notification by id. If the pinned
        // id was removed under us (dismissed on another code path) or was
        // never set, pick the highest-priority remaining — never arbitrarily
        // index into the Vec.
        if self.pending_notifications.is_empty() {
            // Show an empty-state card so Cmd+Shift+A always gives feedback.
            let screen_rect = ctx.screen_rect();
            egui::Area::new(egui::Id::new("notification_modal_scrim"))
                .order(egui::Order::Foreground)
                .fixed_pos(screen_rect.min)
                .interactable(true)
                .show(ctx, |ui| {
                    let (rect, _) = ui.allocate_exact_size(
                        screen_rect.size(),
                        egui::Sense::click(),
                    );
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::ZERO,
                        Color32::from_black_alpha(style::SCRIM_ALPHA),
                    );
                });
            egui::Area::new(egui::Id::new("notification_modal"))
                .order(egui::Order::Tooltip)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(self.colors.bg_sidebar)
                        .stroke(Stroke::new(1.0, self.colors.border))
                        .corner_radius(style::RADIUS_LG)
                        .inner_margin(egui::Margin::symmetric(
                            style::MODAL_PADDING_H,
                            style::MODAL_PADDING_V,
                        ))
                        .show(ui, |ui| {
                            ui.set_width(style::MODAL_WIDTH_NOTIFY);
                            ui.vertical_centered(|ui| {
                                ui.add_space(style::SPACE_MD);
                                ui.label(
                                    RichText::new("No notifications")
                                        .size(style::TEXT_BODY)
                                        .color(self.colors.text_dim),
                                );
                                ui.add_space(style::SPACE_MD);
                            });
                        });
                });
            let esc_pressed = ctx.input(|i| i.key_pressed(egui::Key::Escape));
            if esc_pressed {
                self.show_notification_modal = false;
            }
            return cmds;
        }
        if self
            .current_notify_id
            .as_ref()
            .map(|id| !self.pending_notifications.iter().any(|n| &n.notify_id == id))
            .unwrap_or(true)
        {
            self.current_notify_id = self.select_highest_priority();
        }
        let Some(current_id) = self.current_notify_id.clone() else {
            self.show_notification_modal = false;
            return cmds;
        };
        let Some(notif) = self
            .pending_notifications
            .iter()
            .find(|n| n.notify_id == current_id)
            .cloned()
        else {
            self.show_notification_modal = false;
            self.current_notify_id = None;
            return cmds;
        };

        // Reset per-notification transient state when the front of the queue
        // changes. notify_id is the stable identifier; fall back on title if
        // the app didn't set one (rare — still gives reasonable reset behavior).
        let front_key = if notif.notify_id.is_empty() {
            format!("__no_id__:{}", notif.title)
        } else {
            notif.notify_id.clone()
        };
        if self.modal_state_notify_id != front_key {
            self.modal_state_notify_id = front_key;
            self.modal_focused_option = 0;
            self.modal_input_buffer.clear();
        }

        // Resolve the image attachment (#74) once per frame, before borrowing
        // any other `self` field into the egui closure. `notification_image::resolve`
        // is idempotent — it caches into `self.notification_images` keyed by
        // `notify_id`, so subsequent frames reuse the same TextureHandle.
        let image_state = notification_image::resolve(self, ctx, &notif);

        let screen_rect = ctx.screen_rect();

        // Dim the whole work area. The scrim swallows clicks so UI behind can't
        // accidentally eat them.
        egui::Area::new(egui::Id::new("notification_modal_scrim"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen_rect.min)
            .interactable(true)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(
                    screen_rect.size(),
                    egui::Sense::click(),
                );
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::ZERO,
                    Color32::from_black_alpha(style::SCRIM_ALPHA),
                );
            });

        let level_color = match notif.level.as_str() {
            "error" => Color32::from_rgb(0xff, 0x55, 0x55),
            "warn" => Color32::from_rgb(0xf1, 0xfa, 0x8c),
            _ => self.colors.accent,
        };

        // Live position + total, recomputed every frame. Total reflects the
        // current queue size so if a new notification arrives while this
        // modal is open, the count updates on the next render without
        // displacing the pinned view.
        let (position_idx, queue_len) = self
            .position_of_current()
            .unwrap_or((1, self.pending_notifications.len()));
        let mut action_cmd: Option<AppCommand> = None;

        // ── Keyboard handling ──────────────────────────────────────────────
        // Done before rendering so focus index is current when we draw the
        // highlighted option. Typing into the input field is handled by the
        // TextEdit widget below (it reads pressed keys from egui directly).
        // Cmd+Enter is consumed (not just read) so the multiline `TextEdit`
        // can't see it and misinterpret it as a newline or some other
        // widget-local action. Bare Enter deliberately flows through to
        // TextEdit for the input kind (it inserts a newline) and to our own
        // read for message/choice kinds (it confirms).
        let cmd_enter_pressed = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
        });

        // For NotifyKind::Input, bare Enter must reach the TextEdit (newline).
        // For all other kinds, consume it so it cannot bleed into panes behind.
        let consume_bare_enter = !matches!(notif.kind, NotifyKind::Input);

        let (
            enter_pressed,
            space_pressed,
            esc_pressed,
            up_pressed,
            down_pressed,
            digit_pressed,
            shortcut_pressed,
        ) = ctx.input_mut(|i| {
            // Bare Enter (no modifiers) — used for message/choice submit.
            let enter = if consume_bare_enter {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            } else {
                i.key_pressed(egui::Key::Enter)
                    && !i.modifiers.command
                    && !i.modifiers.shift
                    && !i.modifiers.alt
                    && !i.modifiers.ctrl
            };
            // Space and arrows are also gated — they must reach TextEdit for
            // cursor navigation and space insertion in NotifyKind::Input.
            let space = if consume_bare_enter {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Space)
            } else {
                false
            };
            let esc = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            let (up, down) = if consume_bare_enter {
                let up = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::K);
                let down = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::J);
                (up, down)
            } else {
                (false, false)
            };
            let mut digit: Option<usize> = None;
            for (n, key) in [
                (1, egui::Key::Num1), (2, egui::Key::Num2), (3, egui::Key::Num3),
                (4, egui::Key::Num4), (5, egui::Key::Num5), (6, egui::Key::Num6),
                (7, egui::Key::Num7), (8, egui::Key::Num8), (9, egui::Key::Num9),
            ] {
                if i.consume_key(egui::Modifiers::NONE, key) {
                    digit = Some(n - 1);
                    break;
                }
            }
            // Collect typed characters for per-option shortcut matching.
            let mut shortcut: Option<char> = None;
            for ev in &i.events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev {
                    if modifiers.is_none() {
                        if let Some(name) = key.name().chars().next() {
                            let c = name.to_ascii_lowercase();
                            if c.is_ascii_alphabetic() {
                                shortcut = Some(c);
                                break;
                            }
                        }
                    }
                }
            }
            (enter, space, esc, up, down, digit, shortcut)
        });

        // Apply keyboard to modal state BEFORE rendering (so focus ring is fresh).
        match notif.kind {
            NotifyKind::Choice if !notif.options.is_empty() => {
                let n = notif.options.len();
                if up_pressed && self.modal_focused_option > 0 {
                    self.modal_focused_option -= 1;
                }
                if down_pressed && self.modal_focused_option + 1 < n {
                    self.modal_focused_option += 1;
                }
            }
            _ => {}
        }

        egui::Area::new(egui::Id::new("notification_modal"))
            .order(egui::Order::Tooltip)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(style::RADIUS_LG)
                    .inner_margin(egui::Margin::symmetric(
                        style::MODAL_PADDING_H,
                        style::MODAL_PADDING_V,
                    ))
                    .show(ui, |ui| {
                        ui.set_width(style::MODAL_WIDTH_NOTIFY);

                        // Header: level dot + kind label · queue indicator.
                        ui.horizontal(|ui| {
                            let (dot_rect, _) = ui.allocate_exact_size(
                                Vec2::new(10.0, 10.0),
                                egui::Sense::hover(),
                            );
                            ui.painter()
                                .circle_filled(dot_rect.center(), 5.0, level_color);
                            ui.add_space(style::SPACE_SM);
                            let kind_label = match notif.kind {
                                NotifyKind::Message => "MESSAGE",
                                NotifyKind::Choice => "CHOICE",
                                NotifyKind::Input => "INPUT",
                            };
                            ui.label(
                                RichText::new(format!(
                                    "{}  ·  {}",
                                    notif.level.to_uppercase(),
                                    kind_label
                                ))
                                .size(style::TEXT_HINT)
                                .color(self.colors.text_dim)
                                .strong(),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if queue_len > 1 {
                                    // RTL layout: L first (rightmost), then H.
                                    crate::widgets::key_chip(
                                        ui, "L", &self.colors,
                                    );
                                    ui.add_space(4.0);
                                    crate::widgets::key_chip(
                                        ui, "H", &self.colors,
                                    );
                                    ui.add_space(8.0);
                                    ui.label(
                                        RichText::new(format!(
                                            "{position_idx} of {queue_len}  ·  cycle"
                                        ))
                                        .size(style::TEXT_HINT)
                                        .color(self.colors.text_dim),
                                    );
                                }
                            });
                        });

                        ui.add_space(style::SPACE_XL);

                        // Title — centered, large.
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(&notif.title)
                                    .size(style::TEXT_TITLE_XL)
                                    .color(self.colors.text_primary)
                                    .strong(),
                            );
                        });

                        // Body — centered under the title.
                        if !notif.body.is_empty() {
                            ui.add_space(style::SPACE_MD);
                            ui.vertical_centered(|ui| {
                                ui.set_max_width(style::MODAL_WIDTH_NOTIFY - 120.0);
                                ui.label(
                                    RichText::new(&notif.body)
                                        .size(style::TEXT_BODY)
                                        .color(self.colors.text_primary),
                                );
                            });
                        }

                        // Image attachment (#74) — renders above the
                        // kind-specific body / action buttons. Sized to fit
                        // the modal width with aspect ratio preserved, max
                        // height 200 px. Placeholder badges render the
                        // user-visible reason text.
                        if let Some(state) = &image_state {
                            ui.add_space(style::SPACE_MD);
                            ui.vertical_centered(|ui| {
                                draw_notification_image(ui, state, &self.colors);
                            });
                        }

                        ui.add_space(style::SPACE_XL);

                        if notif.tombstoned {
                            // Source ended — show a dim label and a plain Dismiss button.
                            // Action buttons are hidden since the app can no longer respond.
                            ui.add_space(style::SPACE_SM);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("Source ended")
                                        .size(style::TEXT_BODY)
                                        .color(self.colors.text_dim)
                                        .italics(),
                                );
                            });
                            ui.add_space(style::SPACE_SM);
                            ui.add_space(style::SPACE_MD);
                            ui.vertical_centered(|ui| {
                                let resp = primary_button(ui, "Dismiss", &self.colors, 180.0);
                                if resp.clicked() {
                                    if let Some(n) = self.pending_notifications
                                        .iter()
                                        .find(|n| n.notify_id == current_id)
                                        .cloned()
                                    {
                                        self.pending_notifications
                                            .retain(|x| x.notify_id != current_id);
                                        self.save_notifications();
                                        self.current_notify_id = None;
                                        if !n.notify_id.is_empty()
                                            && !n.notify_id.starts_with("__host__:")
                                        {
                                            action_cmd =
                                                Some(AppCommand::DeliverNotifyAction {
                                                    pane_id: n.sender_pane_id,
                                                    notify_id: n.notify_id.clone(),
                                                    action_label: "tombstone_dismiss"
                                                        .to_string(),
                                                    value: Some(
                                                        "tombstone_dismiss".to_string(),
                                                    ),
                                                    response_file: n.response_file.clone(),
                                                    host_action: None,
                                                });
                                        }
                                    }
                                }
                            });
                            ui.add_space(style::SPACE_MD);
                        } else {
                        // Kind-specific body.
                        match notif.kind {
                            NotifyKind::Message => {}
                            NotifyKind::Choice => {
                                for (idx, opt) in notif.options.iter().enumerate() {
                                    let focused = idx == self.modal_focused_option;
                                    let shortcut_hint = opt
                                        .shortcut
                                        .as_ref()
                                        .map(|s| format!("[{}]", s.to_uppercase()))
                                        .unwrap_or_else(|| format!("[{}]", idx + 1));
                                    let resp = option_button(
                                        ui,
                                        &opt.label,
                                        &shortcut_hint,
                                        focused,
                                        &self.colors,
                                    );
                                    if resp.clicked() {
                                        let value = if opt.value.is_empty() {
                                            opt.label.clone()
                                        } else {
                                            opt.value.clone()
                                        };
                                        action_cmd = Some(AppCommand::DeliverNotifyAction {
                                            pane_id: notif.sender_pane_id,
                                            notify_id: notif.notify_id.clone(),
                                            action_label: opt.label.clone(),
                                            value: Some(value),
                                            response_file: notif.response_file.clone(),
                                            host_action: opt.host_action.clone(),
                                        });
                                    }
                                    ui.add_space(style::SPACE_SM);
                                }
                            }
                            NotifyKind::Input => {
                                if let Some(prompt) = &notif.input_prompt {
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new(prompt)
                                                .size(style::TEXT_CAPTION)
                                                .color(self.colors.text_dim),
                                        );
                                    });
                                    ui.add_space(style::SPACE_SM);
                                }
                                // Multiline editor: Enter inserts a newline,
                                // Cmd+Enter submits (handled in the keyboard
                                // pre-pass below). Scrolls vertically once it
                                // exceeds the visible row count.
                                let resp = ui.scope(|ui| {
                                    ui.visuals_mut().text_cursor.stroke.width = 1.5;
                                    ui.visuals_mut().text_cursor.stroke.color = self.colors.accent;
                                    ui.visuals_mut().extreme_bg_color = self.colors.bg_active;
                                    ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(1.0, self.colors.accent);
                                    ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::new(1.0, self.colors.border);
                                    ui.add(egui::TextEdit::multiline(&mut self.modal_input_buffer)
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(6)
                                        .font(egui::FontId::proportional(16.0))
                                        .margin(egui::Margin::symmetric(12, 10))
                                        .hint_text("Type. Enter for newline, \u{2318}\u{21B5} to submit."))
                                }).inner;
                                resp.request_focus();
                            }
                        }

                        ui.add_space(style::SPACE_MD);

                        // Footer: primary action for message kind, plus a
                        // centered hint row describing keyboard shortcuts.
                        if matches!(notif.kind, NotifyKind::Message) {
                            ui.vertical_centered(|ui| {
                                let resp = primary_button(
                                    ui,
                                    "Acknowledge",
                                    &self.colors,
                                    180.0,
                                );
                                if resp.clicked() {
                                    action_cmd = Some(AppCommand::DeliverNotifyAction {
                                        pane_id: notif.sender_pane_id,
                                        notify_id: notif.notify_id.clone(),
                                        action_label: "acknowledge".to_string(),
                                        value: None,
                                        response_file: notif.response_file.clone(),
                                        host_action: None,
                                    });
                                }
                            });
                            ui.add_space(style::SPACE_MD);
                        }
                        } // end !tombstoned

                        // Footer keyboard hints — separator + centered hint row per kind.
                        // Not shown for tombstoned notifications (only a Dismiss button).
                        //
                        // Centering strategy: egui's vertical_centered expands all child
                        // rects to full available width (Layout::top_down(Center) sets
                        // frame_size.x = max(desired, available)), so placing a ui.horizontal
                        // inside vertical_centered does NOT center it — content paints at x=0.
                        // Fix: pre-measure the row width with ui.fonts, then use
                        // allocate_ui_with_layout(exact_size) — justify_and_align then places
                        // the child rect at center_x − hint_w/2, which IS centered.
                        if !notif.tombstoned {
                            ui.add_space(style::SPACE_SM);
                            ui.separator();
                            ui.add_space(style::SPACE_SM);

                            // Measure hint row width to match what key_combo_list renders.
                            // key_combo_list: item_spacing=0, INTER_COMBO_GAP=10 between
                            // combos, TRAILING_GAP=10 before label. INTRA_COMBO_GAP=2
                            // between chips within a combo (e.g. ⌘+↵).
                            let mono12 = egui::FontId::monospace(style::TEXT_CAPTION);
                            let prop11 = egui::FontId::proportional(style::TEXT_HINT);
                            let (hint_w, hint_h) = ui.fonts(|f| {
                                let chip_w = |s: &str| -> f32 {
                                    let g = f.layout_no_wrap(
                                        s.to_string(), mono12.clone(), egui::Color32::WHITE,
                                    );
                                    let h = g.size().y + 6.0; // KEYCAP_PAD_V*2
                                    (g.size().x + 12.0_f32).max(h) // KEYCAP_PAD_H*2
                                };
                                let lbl_w = |s: &str| -> f32 {
                                    f.layout_no_wrap(
                                        s.to_string(), prop11.clone(), egui::Color32::WHITE,
                                    ).size().x
                                };
                                let kcl = |combos: &[&[&str]], trailing: Option<&str>| -> f32 {
                                    let mut w = 0.0_f32;
                                    for (i, keys) in combos.iter().enumerate() {
                                        if i > 0 { w += 10.0; } // INTER_COMBO_GAP
                                        for (j, key) in keys.iter().enumerate() {
                                            if j > 0 { w += 2.0; } // INTRA_COMBO_GAP
                                            w += chip_w(key);
                                        }
                                    }
                                    if let Some(t) = trailing { w += 10.0 + lbl_w(t); }
                                    w
                                };
                                // Between groups: add_space(8) + "·" label + add_space(8)
                                let dot = || 8.0 + lbl_w("·") + 8.0;
                                let w = match notif.kind {
                                    NotifyKind::Message => {
                                        kcl(&[&["Enter"], &["Space"]], Some("acknowledge"))
                                        + if !notif.required {
                                            dot() + kcl(&[&["Esc"]], Some("dismiss"))
                                        } else { 0.0 }
                                    }
                                    NotifyKind::Choice => {
                                        kcl(&[&["↑↓"], &["j/k"]], Some("navigate"))
                                        + dot() + kcl(&[&["Enter"], &["1-9"]], Some("select"))
                                        + if !notif.required {
                                            dot() + kcl(&[&["Esc"]], Some("dismiss"))
                                        } else { 0.0 }
                                    }
                                    NotifyKind::Input => {
                                        kcl(&[&["Enter"]], Some("newline"))
                                        + dot() + kcl(&[&["\u{2318}", "\u{21B5}"]], Some("submit"))
                                        + if !notif.required {
                                            dot() + kcl(&[&["Esc"]], Some("dismiss"))
                                        } else { 0.0 }
                                    }
                                };
                                let h = {
                                    let g = f.layout_no_wrap(
                                        "A".to_string(), mono12.clone(), egui::Color32::WHITE,
                                    );
                                    g.size().y + 6.0 // chip_h
                                };
                                (w, h)
                            });

                            // Allocate the exact width centered in the parent top_down(Center).
                            // justify_and_align places child_rect at center_x − hint_w/2.
                            ui.vertical_centered(|ui| {
                                ui.allocate_ui_with_layout(
                                    egui::Vec2::new(hint_w, hint_h),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.spacing_mut().item_spacing.x = 0.0;
                                        let dim = |s: &str| {
                                            RichText::new(s)
                                                .size(style::TEXT_HINT)
                                                .color(self.colors.text_dim)
                                        };
                                        match notif.kind {
                                            NotifyKind::Message => {
                                                crate::widgets::key_combo_list(
                                                    ui,
                                                    &[&["Enter"], &["Space"]],
                                                    Some("acknowledge"),
                                                    &self.colors,
                                                );
                                                if !notif.required {
                                                    ui.add_space(style::SPACE_SM);
                                                    ui.label(dim("·"));
                                                    ui.add_space(style::SPACE_SM);
                                                    crate::widgets::key_combo_list(
                                                        ui,
                                                        &[&["Esc"]],
                                                        Some("dismiss"),
                                                        &self.colors,
                                                    );
                                                }
                                            }
                                            NotifyKind::Choice => {
                                                crate::widgets::key_combo_list(
                                                    ui,
                                                    &[&["↑↓"], &["j/k"]],
                                                    Some("navigate"),
                                                    &self.colors,
                                                );
                                                ui.add_space(style::SPACE_SM);
                                                ui.label(dim("·"));
                                                ui.add_space(style::SPACE_SM);
                                                crate::widgets::key_combo_list(
                                                    ui,
                                                    &[&["Enter"], &["1-9"]],
                                                    Some("select"),
                                                    &self.colors,
                                                );
                                                if !notif.required {
                                                    ui.add_space(style::SPACE_SM);
                                                    ui.label(dim("·"));
                                                    ui.add_space(style::SPACE_SM);
                                                    crate::widgets::key_combo_list(
                                                        ui,
                                                        &[&["Esc"]],
                                                        Some("dismiss"),
                                                        &self.colors,
                                                    );
                                                }
                                            }
                                            NotifyKind::Input => {
                                                crate::widgets::key_combo_list(
                                                    ui,
                                                    &[&["Enter"]],
                                                    Some("newline"),
                                                    &self.colors,
                                                );
                                                ui.add_space(style::SPACE_SM);
                                                ui.label(dim("·"));
                                                ui.add_space(style::SPACE_SM);
                                                crate::widgets::key_combo_list(
                                                    ui,
                                                    &[&["\u{2318}", "\u{21B5}"]],
                                                    Some("submit"),
                                                    &self.colors,
                                                );
                                                if !notif.required {
                                                    ui.add_space(style::SPACE_SM);
                                                    ui.label(dim("·"));
                                                    ui.add_space(style::SPACE_SM);
                                                    crate::widgets::key_combo_list(
                                                        ui,
                                                        &[&["Esc"]],
                                                        Some("dismiss"),
                                                        &self.colors,
                                                    );
                                                }
                                            }
                                        }
                                    },
                                );
                            });
                        } // end !tombstoned keyboard hint
                    });
            });

        // ── Resolve keyboard input into an action_cmd (only if not already
        //    produced by a mouse click). Mouse wins; keyboard is a fallback.
        // Tombstoned notifications have no keyboard actions — only the mouse Dismiss.
        if action_cmd.is_none() && !notif.tombstoned {
            match notif.kind {
                NotifyKind::Message => {
                    if enter_pressed || space_pressed {
                        action_cmd = Some(AppCommand::DeliverNotifyAction {
                            pane_id: notif.sender_pane_id,
                            notify_id: notif.notify_id.clone(),
                            action_label: "acknowledge".to_string(),
                            value: None,
                            response_file: notif.response_file.clone(),
                            host_action: None,
                        });
                    }
                }
                NotifyKind::Choice if !notif.options.is_empty() => {
                    // Direct-select by digit, then per-option shortcut, then Enter.
                    let mut picked: Option<usize> = None;
                    if let Some(d) = digit_pressed {
                        if d < notif.options.len() {
                            picked = Some(d);
                        }
                    }
                    if picked.is_none() {
                        if let Some(c) = shortcut_pressed {
                            for (i, opt) in notif.options.iter().enumerate() {
                                if let Some(sc) = &opt.shortcut {
                                    if sc.to_ascii_lowercase().chars().next() == Some(c) {
                                        picked = Some(i);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    if picked.is_none() && (enter_pressed || space_pressed) {
                        picked = Some(self.modal_focused_option);
                    }
                    if let Some(idx) = picked {
                        let opt = &notif.options[idx];
                        let value = if opt.value.is_empty() {
                            opt.label.clone()
                        } else {
                            opt.value.clone()
                        };
                        action_cmd = Some(AppCommand::DeliverNotifyAction {
                            pane_id: notif.sender_pane_id,
                            notify_id: notif.notify_id.clone(),
                            action_label: opt.label.clone(),
                            value: Some(value),
                            response_file: notif.response_file.clone(),
                            host_action: opt.host_action.clone(),
                        });
                    }
                }
                NotifyKind::Input => {
                    // Bare Enter inserts a newline into the multiline field;
                    // Cmd+Enter is the commit chord.
                    if cmd_enter_pressed {
                        let buf = self.modal_input_buffer.trim().to_string();
                        if !buf.is_empty() || !notif.required {
                            action_cmd = Some(AppCommand::DeliverNotifyAction {
                                pane_id: notif.sender_pane_id,
                                notify_id: notif.notify_id.clone(),
                                action_label: "submit".to_string(),
                                value: Some(buf),
                                response_file: notif.response_file.clone(),
                                host_action: None,
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        // Esc defers (not cancels) unless the notification is required.
        //
        // Defer = close the modal but keep the notification in the queue.
        // Cmd+Shift+A will bring it back as the front-most on reopen.
        // No NotifyAction is dispatched to the app — the app hasn't been
        // answered yet, it just got postponed.
        //
        // For required notifications, Esc does nothing: the user must
        // acknowledge or pick an option.
        if action_cmd.is_none() && esc_pressed && !notif.required {
            self.show_notification_modal = false;
            self.modal_focused_option = 0;
            self.modal_input_buffer.clear();
            self.modal_state_notify_id.clear();
            // `current_notify_id` intentionally stays set so the same
            // notification is front-most on next reopen. No queue mutation,
            // no NotifyAction — that's what makes it defer, not cancel.
            return cmds;
        }

        if let Some(cmd) = action_cmd {
            // Check for snooze before removing from queue. Snooze sets
            // deliver_after in place — the notification stays but becomes
            // invisible and no DeliverNotifyAction is sent to the app (the
            // CLI stays blocked until the user picks a non-snooze choice).
            let is_snooze = if let AppCommand::DeliverNotifyAction { ref host_action, .. } = cmd {
                if let Some(delay_secs) = host_action.as_deref()
                    .and_then(|a| a.strip_prefix("snooze:"))
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    let wake = std::time::Instant::now() + std::time::Duration::from_secs(delay_secs);
                    if let Some(n) = self.pending_notifications.iter_mut()
                        .find(|n| n.notify_id == current_id)
                    {
                        n.deliver_after = Some(wake);
                        log::info!("notify:snooze: notify_id={} delay={}s", current_id, delay_secs);
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            };

            self.modal_focused_option = 0;
            self.modal_input_buffer.clear();
            self.modal_state_notify_id.clear();

            if !is_snooze {
                // Real answer: remove from queue and deliver NotifyAction to app.
                self.pending_notifications.retain(|n| n.notify_id != current_id);
                self.save_notifications();
                cmds.push(cmd);
            }

            match self.select_highest_priority() {
                Some(next) => self.current_notify_id = Some(next),
                None => {
                    self.show_notification_modal = false;
                    self.current_notify_id = None;
                }
            }
        }

        cmds
    }
}

/// Full-width option button for the notification modal's `choice` kind.
///
/// Egui's built-in `Button` left-aligns its label and gives you no hook to
/// center it inside a wider fixed-width rect. We paint the rect, label, and
/// shortcut-hint manually so:
///   • The label sits center-horizontal and center-vertical.
///   • The shortcut hint (e.g. `[Y]`) sits right-aligned in the gutter.
///   • The focused option gets an accent fill + darker text for contrast.
fn option_button(
    ui: &mut egui::Ui,
    label: &str,
    shortcut_hint: &str,
    focused: bool,
    colors: &Colors,
) -> egui::Response {
    let width = ui.available_width();
    let height = style::BUTTON_H_LG;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());

    let (bg, fg, hint_color) = if focused {
        (
            colors.accent,
            Color32::BLACK,
            Color32::from_black_alpha(140),
        )
    } else {
        (colors.bg_hover, colors.text_primary, colors.text_dim)
    };

    // Hover lifts the bg slightly for non-focused options (focused is already
    // the accent and looks clearly active — no hover lift needed).
    let actual_bg = if resp.hovered() && !focused {
        colors.bg_active
    } else {
        bg
    };

    let painter = ui.painter();
    painter.rect_filled(rect, style::RADIUS_MD, actual_bg);

    // Centered label.
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(style::TEXT_BODY + 1.0),
        fg,
    );

    // Right-gutter shortcut hint.
    if !shortcut_hint.is_empty() {
        let hint_pos = egui::pos2(
            rect.right() - 18.0,
            rect.center().y,
        );
        painter.text(
            hint_pos,
            Align2::RIGHT_CENTER,
            shortcut_hint,
            egui::FontId::proportional(style::TEXT_CAPTION),
            hint_color,
        );
    }

    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    resp
}

/// Primary action button — used for the message-kind Acknowledge. Fixed width,
/// center-aligned label, accent fill. Keyboard dispatch is handled separately.
fn primary_button(
    ui: &mut egui::Ui,
    label: &str,
    colors: &Colors,
    width: f32,
) -> egui::Response {
    let height = style::BUTTON_H_MD;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());

    let bg = if resp.hovered() {
        colors.accent.gamma_multiply(1.15)
    } else {
        colors.accent
    };
    let painter = ui.painter();
    painter.rect_filled(rect, style::RADIUS_MD, bg);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(style::TEXT_BODY + 1.0),
        Color32::BLACK,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
}

/// Maximum displayed image height inside the notification card. Width is
/// constrained by the card's available width; aspect ratio is preserved.
const NOTIFICATION_IMAGE_MAX_H: f32 = 200.0;

/// Render an attached notification image — either the loaded texture (with
/// aspect-preserving scaling) or a placeholder badge with the failure
/// reason. Called from the modal renderer above the kind-specific body.
fn draw_notification_image(
    ui: &mut egui::Ui,
    state: &crate::app::NotificationImageState,
    colors: &Colors,
) {
    use crate::app::NotificationImageState as S;
    match state {
        S::Ready(texture, w, h) => {
            // Fit within the card's available width and the global
            // 200 px height cap, preserving aspect ratio.
            let avail_w = ui.available_width();
            let (orig_w, orig_h) = (*w as f32, *h as f32);
            let scale = (avail_w / orig_w).min(NOTIFICATION_IMAGE_MAX_H / orig_h);
            // Only ever shrink — never upscale a small image and pixelate it.
            let scale = scale.min(1.0).max(0.0);
            let display = Vec2::new(orig_w * scale, orig_h * scale);
            ui.add(egui::Image::new((texture.id(), display)).corner_radius(style::RADIUS_MD));
        }
        S::Placeholder { reason } => {
            // Draw a small badge so the user sees that an image was attached
            // but couldn't be rendered. Keeps the modal layout stable.
            let badge_w = 220.0;
            let badge_h = 36.0;
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(badge_w, badge_h),
                egui::Sense::hover(),
            );
            ui.painter()
                .rect_filled(rect, style::RADIUS_MD, colors.bg_hover);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!("[image: {reason}]"),
                egui::FontId::proportional(style::TEXT_CAPTION),
                colors.text_dim,
            );
        }
        S::Pending => {
            // Show a placeholder so the layout doesn't jump on the next frame
            // when the texture arrives. "loading" is the user-visible label.
            let badge_w = 160.0;
            let badge_h = 36.0;
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(badge_w, badge_h),
                egui::Sense::hover(),
            );
            ui.painter()
                .rect_filled(rect, style::RADIUS_MD, colors.bg_hover);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "[image: loading…]",
                egui::FontId::proportional(style::TEXT_CAPTION),
                colors.text_dim,
            );
        }
    }
}

impl PlexiApp {
    /// Render the spatial-grid minimap in the top-right corner of the work
    /// area. Uses an `egui::Area` so it floats above all content.
    pub(crate) fn draw_minimap_overlay(&mut self, ctx: &egui::Context) {
        if !self.minimap.visible {
            return;
        }

        let content_rect = ctx.screen_rect();
        let active_context = self.active_window;
        let colors = self.colors;
        let ws_id = self.router.active().context_id;
        let ws_name = self.router.active().name.clone();

        // Anchor the Area at the actual panel position so its bounding rect
        // doesn't overlap the sidebar. Previously anchored at content_rect.min
        // (0,0), which made layer_id_at return Foreground for the top sidebar
        // rows, suppressing their hover state entirely (#852).
        let Some(panel_rect) = crate::minimap::minimap_panel_rect(
            ctx,
            content_rect,
            &self.windows,
            ws_id,
            &ws_name,
        ) else {
            return;
        };
        log::debug!("minimap area anchored at {:?}", panel_rect.min);

        egui::Area::new(egui::Id::new("minimap_overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(panel_rect.min)
            .interactable(true)
            .show(ctx, |ui| {
                if let Some(clicked_idx) = crate::minimap::render_minimap(
                    ui,
                    content_rect,
                    &self.windows,
                    active_context,
                    &self.last_page_x_per_row,
                    &colors,
                    ws_id,
                    &ws_name,
                ) {
                    let old = &self.windows[self.active_window];
                    self.last_page_x_per_row.insert(old.grid_y, old.grid_x);
                    self.active_window = clicked_idx;
                    let new = &self.windows[clicked_idx];
                    self.last_page_x_per_row.insert(new.grid_y, new.grid_x);
                    self.context_active_window.insert(ws_id, new.window_id);
                }
            });
    }

    pub(crate) fn draw_welcome_screen(&self, ui: &mut egui::Ui) {
        let colors = self.colors;
        let center = ui.max_rect().center();
        let box_rect = egui::Rect::from_center_size(center, egui::vec2(480.0, 560.0));

        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(box_rect), |ui| {
            egui::Frame::new()
                .fill(colors.bg_sidebar)
                .stroke(Stroke::new(1.0, colors.border))
                .corner_radius(style::RADIUS_MD)
                .inner_margin(egui::Margin::symmetric(
                    style::MODAL_PADDING_H,
                    style::MODAL_PADDING_V,
                ))
                .show(ui, |ui| {
                    ui.add_space(style::SPACE_XL);
                    {
                        let logo_size = style::TEXT_TITLE_XL + 4.0; // 32px — matches SVG native size
                        let gap = style::SPACE_SM;
                        let font_id = egui::FontId::proportional(style::TEXT_TITLE_XL);
                        let text_w = ui.fonts(|f| {
                            f.layout_no_wrap(
                                "PLEXI".to_string(),
                                font_id,
                                colors.text_primary,
                            )
                            .size()
                            .x
                        });
                        let total_w = logo_size + gap + text_w;
                        let pad = ((ui.available_width() - total_w) / 2.0).max(0.0);

                        ui.horizontal(|ui| {
                            ui.add_space(pad);
                            let (logo_rect, _) = ui.allocate_exact_size(
                                egui::vec2(logo_size, logo_size),
                                egui::Sense::hover(),
                            );
                            let painter = ui.painter().clone();
                            let scale = logo_size / 32.0;
                            let cell = egui::vec2(10.0 * scale, 10.0 * scale);
                            let s1 = 5.0 * scale;
                            let s2 = 17.0 * scale;
                            let rx =
                                egui::CornerRadius::same((1.5 * scale).round() as u8);
                            let outline = Stroke::new(
                                0.9 * scale,
                                egui::Color32::from_rgb(0xe4, 0xe4, 0xe7),
                            );
                            let purple = egui::Color32::from_rgb(0x3b, 0x07, 0x64);
                            for (dx, dy, filled) in [
                                (s1, s1, false),
                                (s2, s1, false),
                                (s1, s2, false),
                                (s2, s2, true),
                            ] {
                                let r = egui::Rect::from_min_size(
                                    logo_rect.min + egui::vec2(dx, dy),
                                    cell,
                                );
                                if filled {
                                    painter.rect_filled(r, rx, purple);
                                } else {
                                    painter.rect_stroke(r, rx, outline, egui::StrokeKind::Inside);
                                }
                            }
                            ui.add_space(gap);
                            ui.label(
                                RichText::new("PLEXI")
                                    .size(style::TEXT_TITLE_XL)
                                    .color(colors.text_primary)
                                    .strong(),
                            );
                        });
                    }
                    ui.add_space(style::SPACE_MD);
                    ui.label(
                        RichText::new(
                            "Caution: early-stage passion project. If you encounter \
                             any issues, don't hesitate to reach out.",
                        )
                        .size(style::TEXT_HINT)
                        .color(colors.text_dim)
                        .italics(),
                    );
                    ui.add_space(style::SPACE_XL);

                    // Each entry: (chip groups for one combo, description).
                    // Every modifier/key is a separate chip — no combined strings.
                    let shortcuts: &[(&[&str], &str)] = &[
                        (&["⌘", "N"], "new terminal"),
                        (&["⌘", "E"], "file browser"),
                        (&["⌘", "P"], "command palette"),
                        (&["⌘", "⇧", "N"], "new context"),
                        (&["⌘", "/"], "keyboard shortcuts"),
                    ];

                    for (keys, desc) in shortcuts {
                        ui.horizontal(|ui| {
                            crate::widgets::key_combo(ui, keys, &colors);
                            ui.add_space(style::SPACE_SM);
                            ui.label(
                                RichText::new(*desc)
                                    .size(style::TEXT_BODY)
                                    .color(colors.text_dim),
                            );
                        });
                        ui.add_space(style::SPACE_SM);
                    }

                    ui.add_space(style::SPACE_XL);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("New to Plexi? Run ")
                                .size(style::TEXT_CAPTION)
                                .color(colors.text_dim),
                        );
                        ui.label(
                            RichText::new("plexi demo")
                                .size(style::TEXT_CAPTION)
                                .color(colors.text_dim)
                                .monospace()
                                .background_color(colors.bg_active),
                        );
                        ui.label(
                            RichText::new(" in any terminal to get started.")
                                .size(style::TEXT_CAPTION)
                                .color(colors.text_dim),
                        );
                    });
                    ui.add_space(style::SPACE_SM);
                    ui.hyperlink_to(
                        RichText::new("Read the docs at plexiapp.com/docs")
                            .size(style::TEXT_CAPTION)
                            .color(colors.text_dim),
                        "https://plexiapp.com/docs",
                    );

                    ui.add_space(style::SPACE_MD);
                    ui.separator();
                    ui.add_space(style::SPACE_MD);
                    draw_contact_footer(ui, &colors);
                });
        });
    }

    pub(crate) fn draw_cli_setup_modal(&mut self, ctx: &egui::Context) {
        let cli_name = crate::cli_setup::cli_name();
        let colors = self.colors;
        let cmd = crate::cli_setup::INSTALL_COMMAND;

        egui::Area::new(egui::Id::new("cli_setup_modal"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(24, 20))
                    .show(ui, |ui| {
                        ui.set_width(400.0);

                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(format!("Install `{cli_name}`"))
                                    .size(style::TEXT_BODY)
                                    .color(colors.text_primary)
                                    .strong(),
                            );
                            ui.add_space(style::SPACE_SM);
                            ui.label(
                                RichText::new("Lets shell scripts, agents, and hooks:")
                                    .size(style::TEXT_CAPTION)
                                    .color(colors.text_dim),
                            );
                            ui.add_space(2.0);
                            for item in &["send notifications", "trigger actions", "pipe data into Plexi"] {
                                ui.horizontal(|ui| {
                                    ui.add_space(style::SPACE_SM);
                                    ui.label(
                                        RichText::new(format!("· {item}"))
                                            .size(style::TEXT_CAPTION)
                                            .color(colors.text_dim),
                                    );
                                });
                            }
                            ui.add_space(style::SPACE_MD);
                            ui.label(
                                RichText::new(
                                    "Open Terminal, paste this command, and press Enter.\n\
                                     You'll be asked for your password."
                                )
                                .size(style::TEXT_CAPTION)
                                .color(colors.text_dim),
                            );
                            ui.add_space(style::SPACE_SM);

                            // Copyable install command with code-block styling.
                            egui::Frame::new()
                                .fill(colors.bg_darkest)
                                .corner_radius(R6)
                                .inner_margin(egui::Margin::symmetric(12, 8))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(cmd)
                                                .size(style::TEXT_CAPTION)
                                                .color(colors.text_primary)
                                                .monospace(),
                                        );
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            crate::widgets::copy_button(
                                                ui,
                                                egui::Id::new("cli_setup_copy"),
                                                cmd,
                                            );
                                        });
                                    });
                                });

                            ui.add_space(style::SPACE_MD);

                            // Show result feedback from a previous check.
                            if let Some(false) = self.cli_setup_check_result {
                                ui.label(
                                    RichText::new("CLI not found. Run the command above and try again.")
                                        .size(style::TEXT_CAPTION)
                                        .color(colors.accent),
                                );
                                ui.add_space(style::SPACE_SM);
                            }

                            ui.horizontal(|ui| {
                                let check_btn = ui.add(
                                    egui::Button::new(
                                        RichText::new("Check for success")
                                            .size(style::TEXT_BODY)
                                            .color(colors.text_primary),
                                    )
                                    .fill(colors.bg_active)
                                    .min_size(egui::vec2(150.0, 28.0)),
                                );

                                if check_btn.clicked() {
                                    log::info!("cli_setup: user clicked Check for success");
                                    if crate::cli_setup::is_installed() {
                                        log::info!("cli_setup: CLI found - closing modal");
                                        crate::cli_setup::mark_prompted();
                                        self.cli_setup_check_result = None;
                                        self.show_cli_setup_prompt = false;
                                    } else {
                                        log::info!("cli_setup: CLI not found - showing retry");
                                        self.cli_setup_check_result = Some(false);
                                    }
                                }

                                let skip_btn = ui.add(
                                    egui::Button::new(
                                        RichText::new("Not now")
                                            .size(style::TEXT_BODY)
                                            .color(colors.text_dim),
                                    )
                                    .min_size(egui::vec2(100.0, 28.0)),
                                );

                                if skip_btn.clicked() {
                                    log::info!("cli_setup: user chose Not now - will ask again next launch");
                                    self.show_cli_setup_prompt = false;
                                }
                            });
                        });
                    });
            });

        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            log::info!("cli_setup: dismissed via Escape — will ask again next launch");
            self.show_cli_setup_prompt = false;
        }
    }

    pub(crate) fn draw_completions_banner(&mut self, ctx: &egui::Context) {
        if !self.show_completions_banner {
            return;
        }
        let colors = self.colors;
        let cmd = crate::cli_setup::INSTALL_COMMAND;

        egui::Area::new(egui::Id::new("completions_banner"))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -20.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Shell completions aren't set up.")
                                    .size(style::TEXT_CAPTION)
                                    .color(colors.text_dim),
                            );
                            ui.add_space(style::SPACE_SM);
                            egui::Frame::new()
                                .fill(colors.bg_darkest)
                                .corner_radius(R6)
                                .inner_margin(egui::Margin::symmetric(8, 4))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(cmd)
                                                .size(style::TEXT_HINT)
                                                .color(colors.text_primary)
                                                .monospace(),
                                        );
                                        ui.add_space(4.0);
                                        crate::widgets::copy_button(
                                            ui,
                                            egui::Id::new("completions_banner_copy"),
                                            cmd,
                                        );
                                    });
                                });
                            ui.add_space(style::SPACE_SM);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Done")
                                            .size(style::TEXT_CAPTION)
                                            .color(colors.text_primary),
                                    )
                                    .fill(colors.bg_active)
                                    .min_size(egui::vec2(50.0, 22.0)),
                                )
                                .clicked()
                            {
                                log::info!("cli_setup: completions banner — user clicked Done, marking sentinel");
                                crate::cli_setup::completions_mark_prompted();
                                self.show_completions_banner = false;
                            }
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Not now")
                                            .size(style::TEXT_CAPTION)
                                            .color(colors.text_dim),
                                    )
                                    .min_size(egui::vec2(60.0, 22.0)),
                                )
                                .clicked()
                            {
                                log::info!("cli_setup: completions banner — user clicked Not now (session-only dismiss)");
                                self.show_completions_banner = false;
                            }
                        });
                    });
            });
    }
}

/// Per-overlay keyboard handlers. Each method owns its overlay's full keyboard
/// contract and returns `Consumed` so the unified dispatch loop knows not to
/// pass through to `dispatch_app_key_events` or `poll_actions`.
impl PlexiApp {
    /// Notification modal: consume H/L queue cycling when the active notification
    /// is not Choice or Input. All other key handling stays in `draw_notification_modal`.
    pub(crate) fn notification_modal_handle_key(
        &mut self,
        ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        use crate::app_protocol::NotifyKind;
        let shortcuts_blocked = self
            .current_notify_id
            .as_ref()
            .and_then(|id| self.pending_notifications.iter().find(|n| n.notify_id == *id))
            .map(|n| matches!(n.kind, NotifyKind::Choice | NotifyKind::Input))
            .unwrap_or(true);

        let (h_pressed, l_pressed) = ctx.input_mut(|i| {
            if shortcuts_blocked || !i.modifiers.is_none() {
                return (false, false);
            }
            let h = i.consume_key(egui::Modifiers::NONE, egui::Key::H);
            let l = i.consume_key(egui::Modifiers::NONE, egui::Key::L);
            (h, l)
        });

        if h_pressed {
            log::info!("notification cycle: prev (H)");
            self.cycle_notification(-1);
        }
        if l_pressed {
            log::info!("notification cycle: next (L)");
            self.cycle_notification(1);
        }

        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn confirm_close_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn command_palette_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn rename_pane_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn context_rename_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn context_description_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn quick_note_handle_key(
        &mut self,
        ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        // Consume Cmd+0 so poll_actions doesn't fire OpenQuickNote while the modal
        // is already open — that would reset mid-session note state.
        ctx.input_mut(|i| { i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0); });
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn quick_note_destination_handle_key(
        &mut self,
        ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        ctx.input_mut(|i| { i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0); });
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn quick_note_sub_destination_handle_key(
        &mut self,
        ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        ctx.input_mut(|i| { i.consume_key(egui::Modifiers::COMMAND, egui::Key::Num0); });
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn cli_setup_prompt_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn context_inspector_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn text_input_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn context_close_confirm_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn capability_modal_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }
}

fn parse_children_cmd_output(stdout: &str) -> Vec<crate::config::QuickNoteNode> {
    stdout.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() { return None; }
            let (key_str, rest) = line.split_once('|')?;
            let (label, command) = rest.split_once('|')?;
            let key: u8 = key_str.trim().parse().ok()?;
            Some(crate::config::QuickNoteNode {
                key,
                label: label.trim().to_string(),
                command: Some(command.trim().to_string()),
                hidden: false,
                stay_alive: None,
                options: None,
                children_cmd: None,
                dest_type: None,
                position: None,
                path: None,
            })
        })
        .collect()
}
