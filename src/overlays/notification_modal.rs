use super::*;

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

/// Build a `UiNode::Text` for the notification modal title.
///
/// The title is rendered large and bold, centered inside the modal frame.
/// Extracted so construction is testable without a live egui context.
pub(crate) fn build_notification_title_node(
    title: &str,
    colors: &crate::theme::Colors,
) -> crate::app_protocol::UiNode {
    let color_hex = |c: egui::Color32| {
        format!("#{:02x}{:02x}{:02x}{:02x}", c.r(), c.g(), c.b(), c.a())
    };
    crate::app_protocol::UiNode::Text {
        text: title.to_string(),
        size: style::TEXT_TITLE_XL,
        color: color_hex(colors.text_primary),
        bold: true,
        monospace: false,
    }
}

impl PlexiApp {
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
        // Tombstoned notifications never render a TextEdit, so always consume.
        let consume_bare_enter = notif.tombstoned || !matches!(notif.kind, NotifyKind::Input);

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

                        // Title — centered, large. Node built via component tree.
                        let title_node =
                            build_notification_title_node(&notif.title, &self.colors);
                        ui.vertical_centered(|ui| {
                            crate::render_components::render_component_tree(
                                ui, &title_node, &self.colors,
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

        if action_cmd.is_none() && notif.tombstoned && (enter_pressed || space_pressed) {
            action_cmd = Some(AppCommand::DeliverNotifyAction {
                pane_id: notif.sender_pane_id,
                notify_id: notif.notify_id.clone(),
                action_label: "tombstone_dismiss".to_string(),
                value: Some("tombstone_dismiss".to_string()),
                response_file: notif.response_file.clone(),
                host_action: None,
            });
            log::info!(
                "notify:tombstone_dismiss:keyboard notify_id={}",
                notif.notify_id
            );
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

    pub(crate) fn notification_modal_handle_key(
        &mut self,
        ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        use crate::app_protocol::NotifyKind;
        let shortcuts_blocked = self
            .current_notify_id
            .as_ref()
            .and_then(|id| self.pending_notifications.iter().find(|n| n.notify_id == *id))
            .map(|n| matches!(n.kind, NotifyKind::Input))
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
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod notification_modal_component_tree_tests {
    use super::*;
    use crate::app_protocol::UiNode;
    use crate::config::ThemeConfig;
    use crate::theme::Colors;

    fn test_colors() -> Colors {
        Colors::from_config(&ThemeConfig::default())
    }

    /// Title node must be a `UiNode::Text` with the correct text, size, and bold flag.
    #[test]
    fn notification_title_node_structure() {
        let colors = test_colors();
        let node = build_notification_title_node("Hello world", &colors);
        if let UiNode::Text { text, size, bold, monospace, color } = node {
            assert_eq!(text, "Hello world");
            assert_eq!(size, style::TEXT_TITLE_XL);
            assert!(bold, "title must be bold");
            assert!(!monospace);
            assert!(!color.is_empty(), "color must be set");
        } else {
            panic!("expected UiNode::Text");
        }
    }

    /// Empty title produces an empty-text node rather than erroring.
    #[test]
    fn notification_title_node_empty_title() {
        let colors = test_colors();
        let node = build_notification_title_node("", &colors);
        if let UiNode::Text { text, .. } = node {
            assert_eq!(text, "");
        } else {
            panic!("expected UiNode::Text");
        }
    }
}
