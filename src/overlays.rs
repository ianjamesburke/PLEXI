use crate::app_trait::AppCommand;
use crate::style;
use crate::theme::Colors;
use crate::tiling::PaneId;
use egui::{Align, Align2, Color32, CornerRadius, Layout, RichText, Stroke, Vec2};

use crate::app::PlexiApp;

pub(crate) const MODAL_WIDTH: f32 = 400.0;
const R6: CornerRadius = CornerRadius::same(6);

impl PlexiApp {
    pub(crate) fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let active_ctx = &self.contexts[self.active_context];

            // Context dots (page indicators)
            if self.contexts.len() > 1 {
                let dot_radius = 3.5;
                let dot_spacing = 10.0;
                let total_width = (self.contexts.len() as f32) * dot_spacing;
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(total_width, ui.available_height()),
                    egui::Sense::hover(),
                );
                let y = rect.center().y;
                let start_x = rect.left() + dot_radius;
                for i in 0..self.contexts.len() {
                    let cx = start_x + (i as f32) * dot_spacing;
                    let color = if i == self.active_context {
                        self.colors.accent
                    } else {
                        self.colors.bg_active
                    };
                    ui.painter()
                        .circle_filled(egui::pos2(cx, y), dot_radius, color);
                }
                ui.add_space(4.0);
            }

            // Context info
            ui.label(
                RichText::new(&active_ctx.name)
                    .size(12.0)
                    .color(self.colors.text_primary)
                    .strong(),
            );
            ui.label(
                RichText::new(active_ctx.path.display().to_string())
                    .size(11.0)
                    .color(self.colors.text_dim)
                    .family(egui::FontFamily::Monospace),
            );
            let pane_count = active_ctx.panes.len();
            ui.label(
                RichText::new(format!(
                    "{} pane{}",
                    pane_count,
                    if pane_count == 1 { "" } else { "s" }
                ))
                .size(11.0)
                .color(self.colors.text_section),
            );

            // Right side — help button + notification badge
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("?").size(12.0).color(self.colors.text_dim),
                        )
                        .frame(false),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Keyboard shortcuts (\u{2318}/)")
                    .clicked()
                {
                    self.show_shortcuts = !self.show_shortcuts;
                }

                let notif_count = self.pending_notifications.len();
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
                        if self.show_notification_modal {
                            self.modal_queue_offset = 0;
                        }
                    }
                }
            });
        });
    }

    pub(crate) fn draw_shortcuts_overlay(&self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("shortcuts_overlay"))
            .anchor(Align2::RIGHT_TOP, Vec2::new(-16.0, 44.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(240.0);
                        ui.label(
                            RichText::new("Keyboard Shortcuts")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(8.0);

                        let shortcuts = [
                            ("\u{2318}P", "Command palette"),
                            ("\u{2318}\u{21E7}R", "Rename pane"),
                            ("\u{2318}T", "New tab"),
                            ("\u{2318}]/[", "Next/prev tab"),
                            ("\u{2318}D", "Split right"),
                            ("\u{2318}\u{21E7}D", "Split down"),
                            ("\u{2318}W", "Close pane"),
                            ("\u{2318}B", "Toggle sidebar"),
                            ("\u{2318}H/J/K/L", "Focus pane"),
                            ("\u{2318}\u{21A9}", "Zoom pane"),
                            ("\u{2318}N", "New context"),
                            ("\u{2318}1-9", "Switch context"),
                            ("\u{2318}/", "This help"),
                            ("\u{2318}Q", "Quit"),
                        ];

                        for (key, desc) in shortcuts {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(key)
                                        .size(11.0)
                                        .color(self.colors.accent)
                                        .family(egui::FontFamily::Monospace),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(desc).size(11.0).color(self.colors.text_dim),
                                );
                            });
                        }
                    });
            });
    }

    pub(crate) fn draw_rename_pane_overlay(&mut self, ctx: &egui::Context) {
        let pane_id: PaneId = match self.renaming_pane {
            Some(id) => id,
            None => return,
        };

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
                        let te = ui.add(
                            egui::TextEdit::singleline(&mut self.rename_buffer)
                                .id(te_id)
                                .desired_width(MODAL_WIDTH)
                                .hint_text("Pane name...")
                                .font(egui::TextStyle::Body),
                        );

                        if te.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                self.renaming_pane = None;
                            } else {
                                // Apply rename
                                let new_name = self.rename_buffer.trim().to_string();
                                if let Some(pane) =
                                    self.contexts[self.active_context].panes.get_mut(&pane_id)
                                {
                                    if let Some(t) = pane.as_terminal_mut() {
                                        t.name = if new_name.is_empty() {
                                            None
                                        } else {
                                            Some(new_name)
                                        };
                                    }
                                }
                                self.renaming_pane = None;
                            }
                            // Consume Enter/Escape
                            ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                            });
                        }

                        // Auto-focus and select all
                        if !te.has_focus() {
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
                        }
                    });
            });
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
                        });

                        // Keyboard handling
                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            confirmed = true;
                        }
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            cancelled = true;
                        }
                    });
            });

        if confirmed {
            self.pending_close = false;
            self.execute_close_pane();
        } else if cancelled {
            self.pending_close = false;
        }
    }

    /// Run palette overlay (Cmd+R). Shows active runs across all panes; BlockedOnUser
    /// runs get a [!] badge and an inline text input for unblocking.
    pub(crate) fn draw_run_palette(&mut self, ctx: &egui::Context) {
        // Collect all active runs from every app pane in every context.
        // Clone needed because we hold &self across the window render.
        let all_runs: Vec<(String, String, String, Option<String>)> = Vec::new(); // (run_id, app_id, status, blocked_prompt)
        for _context in &self.contexts {}

        let mut close = false;
        egui::Window::new("Active Runs")
            .collapsible(false)
            .resizable(true)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .default_size(egui::Vec2::new(500.0, 300.0))
            .show(ctx, |ui| {
                if all_runs.is_empty() {
                    ui.label(egui::RichText::new("No active runs.").italics());
                    ui.add_space(8.0);
                    ui.label("Apps create runs via DrawCommand::RunGet.");
                } else {
                    for (run_id, app_id, status, blocked_prompt) in &all_runs {
                        let badge = if status == "blocked_on_user" {
                            "[!] "
                        } else {
                            "    "
                        };
                        ui.label(egui::RichText::new(format!("{badge}{run_id}")).monospace());
                        ui.label(
                            egui::RichText::new(format!("    app={app_id} status={status}"))
                                .small(),
                        );
                        if let Some(prompt) = blocked_prompt {
                            ui.label(prompt);
                        }
                        ui.separator();
                    }
                }
                ui.add_space(8.0);
                if ui.button("Close  [Cmd+R]").clicked() {
                    close = true;
                }
            });

        if close {
            self.show_run_palette = false;
        }
        // Also close on Escape.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_run_palette = false;
        }
    }

    /// Primary notification surface: a keyboard-first centered modal over the
    /// work area. Renders the front of the queue. Dispatches exactly one
    /// `DeliverNotifyAction` per user action and pops the notification; if the
    /// queue still has items the modal stays open on the next one.
    ///
    /// Input map (all kinds):
    ///   Esc        — cancel (only when `required == false`)
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

        let mut cmds: Vec<AppCommand> = Vec::new();

        // Clamp the queue offset so it never points past the end. This matters
        // after an acknowledge removes an entry from under us.
        if self.modal_queue_offset >= self.pending_notifications.len() {
            self.modal_queue_offset = self
                .pending_notifications
                .len()
                .saturating_sub(1);
        }

        let offset = self.modal_queue_offset;
        let Some(notif) = self.pending_notifications.get(offset).cloned() else {
            self.show_notification_modal = false;
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

        let queue_len = self.pending_notifications.len();
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

        let (
            enter_pressed,
            space_pressed,
            esc_pressed,
            up_pressed,
            down_pressed,
            digit_pressed,
            shortcut_pressed,
        ) = ctx.input(|i| {
            // Bare Enter (no modifiers) — used for message/choice submit.
            let enter = i.key_pressed(egui::Key::Enter)
                && !i.modifiers.command
                && !i.modifiers.shift
                && !i.modifiers.alt
                && !i.modifiers.ctrl;
            let space = i.key_pressed(egui::Key::Space);
            let esc = i.key_pressed(egui::Key::Escape);
            let up = i.key_pressed(egui::Key::ArrowUp)
                || i.key_pressed(egui::Key::K);
            let down = i.key_pressed(egui::Key::ArrowDown)
                || i.key_pressed(egui::Key::J);
            let mut digit: Option<usize> = None;
            for (n, key) in [
                (1, egui::Key::Num1), (2, egui::Key::Num2), (3, egui::Key::Num3),
                (4, egui::Key::Num4), (5, egui::Key::Num5), (6, egui::Key::Num6),
                (7, egui::Key::Num7), (8, egui::Key::Num8), (9, egui::Key::Num9),
            ] {
                if i.key_pressed(key) {
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
                        ui.set_width(style::MODAL_WIDTH_MD);

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
                                    ui.label(
                                        RichText::new(format!(
                                            "{} of {queue_len}  ·  \u{2318}[/\u{2318}] to cycle",
                                            offset + 1
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
                                ui.set_max_width(style::MODAL_WIDTH_MD - 80.0);
                                ui.label(
                                    RichText::new(&notif.body)
                                        .size(style::TEXT_BODY)
                                        .color(self.colors.text_primary),
                                );
                            });
                        }

                        ui.add_space(style::SPACE_XL);

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
                                let te = egui::TextEdit::multiline(
                                    &mut self.modal_input_buffer,
                                )
                                .desired_width(f32::INFINITY)
                                .desired_rows(6)
                                .font(egui::FontId::proportional(16.0))
                                .margin(egui::Margin::symmetric(12, 10))
                                .hint_text("Type. Enter for newline, \u{2318}\u{21B5} to submit.");
                                let resp = ui.add(te);
                                resp.request_focus();
                            }
                        }

                        ui.add_space(style::SPACE_XL);

                        // Footer: primary action for message kind, plus a
                        // centered hint row describing keyboard shortcuts.
                        if matches!(notif.kind, NotifyKind::Message) {
                            ui.vertical_centered(|ui| {
                                let resp = primary_button(
                                    ui,
                                    "Acknowledge",
                                    &self.colors,
                                    220.0,
                                );
                                if resp.clicked() {
                                    action_cmd = Some(AppCommand::DeliverNotifyAction {
                                        pane_id: notif.sender_pane_id,
                                        notify_id: notif.notify_id.clone(),
                                        action_label: "acknowledge".to_string(),
                                        value: None,
                                    });
                                }
                            });
                            ui.add_space(style::SPACE_MD);
                        }

                        let hint = match notif.kind {
                            NotifyKind::Message => {
                                if notif.required {
                                    "Enter / Space to acknowledge"
                                } else {
                                    "Enter / Space  ·  Esc to dismiss"
                                }
                            }
                            NotifyKind::Choice => {
                                if notif.required {
                                    "↑↓ or j/k  ·  Enter  ·  1-9"
                                } else {
                                    "↑↓ or j/k  ·  Enter  ·  1-9  ·  Esc to dismiss"
                                }
                            }
                            NotifyKind::Input => {
                                if notif.required {
                                    "Enter for newline  ·  \u{2318}\u{21B5} to submit"
                                } else {
                                    "Enter for newline  ·  \u{2318}\u{21B5} to submit  ·  Esc to dismiss"
                                }
                            }
                        };
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(hint)
                                    .size(style::TEXT_HINT)
                                    .color(self.colors.text_dim),
                            );
                        });
                    });
            });

        // ── Resolve keyboard input into an action_cmd (only if not already
        //    produced by a mouse click). Mouse wins; keyboard is a fallback.
        if action_cmd.is_none() {
            match notif.kind {
                NotifyKind::Message => {
                    if enter_pressed || space_pressed {
                        action_cmd = Some(AppCommand::DeliverNotifyAction {
                            pane_id: notif.sender_pane_id,
                            notify_id: notif.notify_id.clone(),
                            action_label: "acknowledge".to_string(),
                            value: None,
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
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        // Esc cancels unless the notification is required.
        if action_cmd.is_none() && esc_pressed && !notif.required {
            action_cmd = Some(AppCommand::DeliverNotifyAction {
                pane_id: notif.sender_pane_id,
                notify_id: notif.notify_id.clone(),
                action_label: "cancel".to_string(),
                value: None,
            });
        }

        if let Some(cmd) = action_cmd {
            if offset < self.pending_notifications.len() {
                self.pending_notifications.remove(offset);
            }
            self.modal_focused_option = 0;
            self.modal_input_buffer.clear();
            self.modal_state_notify_id.clear();
            if self.pending_notifications.is_empty() {
                self.show_notification_modal = false;
                self.modal_queue_offset = 0;
            } else if self.modal_queue_offset >= self.pending_notifications.len() {
                self.modal_queue_offset = self.pending_notifications.len() - 1;
            }
            cmds.push(cmd);
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
