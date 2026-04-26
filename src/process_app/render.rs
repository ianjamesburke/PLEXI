//! Frame rendering — translates committed DrawCommands into egui paint calls.

use crate::app_protocol::DrawCommand;
use crate::style;
use crate::theme::Colors;
use egui::Color32;

/// Render a committed frame's draw commands into the given egui Ui.
///
/// Only visual primitives reach this function — control commands are routed
/// upstream in `process_app/mod.rs` before commands enter the frame pipeline.
///
/// # Clip stack
///
/// `PushClip` / `PopClip` maintain a `Vec<egui::Rect>` per render pass. Each
/// `PushClip` intersects the new rect with the current top (or the pane rect
/// when the stack is empty) and pushes the result. Every other draw command
/// applies the current top as the painter's clip rect via
/// `painter.with_clip_rect(top)`. At frame end a non-empty stack is logged at
/// `warn` level and cleared.
/// Render a frame's draw commands inside `pane_rect`.
///
/// `pane_rect` is **passed in by the caller** rather than derived from the
/// `Ui`. This is deliberate — derivation invites two-sources-of-truth bugs
/// where the renderer and the caller silently disagree about geometry. An
/// earlier version used `ui.min_rect()` here and got an empty rect (because
/// process_app paints via `ui.painter()` without allocating, so min_rect
/// never grows), which clipped every draw to nothing — all apps appeared
/// blank. Single-source-of-geometry: the caller hands us the rect once.
pub(super) fn render_draw_commands(
    ui: &mut egui::Ui,
    pane_rect: egui::Rect,
    commands: &[DrawCommand],
    colors: &Colors,
) {
    let origin = pane_rect.min;

    // Clip stack. Entries are absolute egui screen-space Rects.
    let mut clip_stack: Vec<egui::Rect> = Vec::new();

    for cmd in commands {
        // Resolve the current effective clip rect: top of stack or pane rect.
        let clip = clip_stack.last().copied().unwrap_or(pane_rect);

        match cmd {
            // ── Clip stack ────────────────────────────────────────────────────
            DrawCommand::PushClip { x, y, w, h } => {
                let new_rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x + x, origin.y + y),
                    egui::vec2(*w, *h),
                );
                // Intersect with current top so nested clips can only tighten.
                let effective = clip.intersect(new_rect);
                clip_stack.push(effective);
                continue;
            }

            DrawCommand::PopClip => {
                if clip_stack.pop().is_none() {
                    log::warn!("render: PopClip on empty clip stack (app bug)");
                }
                continue;
            }

            DrawCommand::Rect {
                x,
                y,
                w,
                h,
                fill,
                radius,
            } => {
                let rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x + x, origin.y + y),
                    egui::vec2(*w, *h),
                );
                let color = parse_color(fill).unwrap_or(colors.bg_active);
                ui.painter().with_clip_rect(clip).rect_filled(rect, *radius, color);
            }

            DrawCommand::Text {
                x,
                y,
                text,
                size,
                color,
                monospace,
                bold,
                align,
                max_width,
                elide,
                selectable,
            } => {
                let color = parse_color(color).unwrap_or(colors.text_primary);
                let family = font_family_for_text(*monospace);
                let font_id = egui::FontId::new(*size, family.clone());
                let pos = egui::pos2(origin.x + x, origin.y + y);
                let anchor = match align.as_str() {
                    "center" => egui::Align2::CENTER_CENTER,
                    "top_center" => egui::Align2::CENTER_TOP,
                    "right" => egui::Align2::RIGHT_TOP,
                    "right_center" => egui::Align2::RIGHT_CENTER,
                    "left_center" => egui::Align2::LEFT_CENTER,
                    _ => egui::Align2::LEFT_TOP, // default
                };

                // Apply max_width clipping / elision using host font metrics.
                let display_text: std::borrow::Cow<'_, str> = if let Some(max_w) = max_width {
                    if *max_w > 0.0 {
                        let galley = ui.fonts(|f| {
                            f.layout_no_wrap(text.clone(), font_id.clone(), color)
                        });
                        if galley.size().x > *max_w {
                            // Binary-search for the longest prefix that fits.
                            let mut lo = 0usize;
                            let mut hi = text.chars().count();
                            while lo + 1 < hi {
                                let mid = (lo + hi) / 2;
                                let candidate: String = if *elide {
                                    text.chars().take(mid).collect::<String>() + "…"
                                } else {
                                    text.chars().take(mid).collect()
                                };
                                let g = ui.fonts(|f| {
                                    f.layout_no_wrap(candidate, font_id.clone(), color)
                                });
                                if g.size().x <= *max_w {
                                    lo = mid;
                                } else {
                                    hi = mid;
                                }
                            }
                            let truncated: String = if *elide {
                                text.chars().take(lo).collect::<String>() + "…"
                            } else {
                                text.chars().take(lo).collect()
                            };
                            std::borrow::Cow::Owned(truncated)
                        } else {
                            std::borrow::Cow::Borrowed(text.as_str())
                        }
                    } else {
                        std::borrow::Cow::Borrowed(text.as_str())
                    }
                } else {
                    std::borrow::Cow::Borrowed(text.as_str())
                };

                if *selectable {
                    // Selectable path: allocate a real egui Label so the user
                    // can drag-select inside the text and Cmd+C copies the
                    // current selection. egui owns the selection state across
                    // frames keyed on the widget's screen position; the label
                    // gets a wrap_width budget equal to (max_width or
                    // remaining-pane-width) so selection behaves the same as
                    // the painter path's clipping.
                    //
                    // We measure the natural galley size, then `ui.put` the
                    // label at the resolved top-left so painted geometry
                    // stays consistent with the non-selectable branch.
                    let wrap_width = max_width
                        .filter(|w| *w > 0.0)
                        .unwrap_or((pane_rect.max.x - pos.x).max(1.0));
                    let galley = ui.fonts(|f| {
                        f.layout(
                            display_text.to_string(),
                            font_id.clone(),
                            color,
                            wrap_width,
                        )
                    });
                    let sz = galley.size();
                    // Resolve the top-left from the requested anchor + size.
                    let top_left = match anchor {
                        egui::Align2::CENTER_CENTER => {
                            egui::pos2(pos.x - sz.x * 0.5, pos.y - sz.y * 0.5)
                        }
                        egui::Align2::CENTER_TOP => egui::pos2(pos.x - sz.x * 0.5, pos.y),
                        egui::Align2::RIGHT_TOP => egui::pos2(pos.x - sz.x, pos.y),
                        egui::Align2::RIGHT_CENTER => egui::pos2(pos.x - sz.x, pos.y - sz.y * 0.5),
                        egui::Align2::LEFT_CENTER => egui::pos2(pos.x, pos.y - sz.y * 0.5),
                        _ => pos,
                    };
                    let target = egui::Rect::from_min_size(top_left, sz);
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(target)
                            .layout(egui::Layout::left_to_right(egui::Align::TOP)),
                    );
                    child.set_clip_rect(clip);
                    let mut rich = egui::RichText::new(display_text.as_ref())
                        .color(color)
                        .font(font_id.clone());
                    if *bold {
                        rich = rich.strong();
                    }
                    let label = egui::Label::new(rich).selectable(true).wrap();
                    child.put(target, label);
                } else {
                    let painter = ui.painter().with_clip_rect(clip);
                    painter.text(pos, anchor, display_text.as_ref(), font_id.clone(), color);
                    if *bold {
                        // Fake-bold by re-painting the same text with a 0.45px
                        // horizontal offset. Same anchor so the center-aligned
                        // case stays centered.
                        let font_id_bold =
                            egui::FontId::new(*size, font_family_for_text(*monospace));
                        painter.text(
                            pos + egui::vec2(0.45, 0.0),
                            anchor,
                            display_text.as_ref(),
                            font_id_bold,
                            color,
                        );
                    }
                }
            }

            DrawCommand::Line {
                x1,
                y1,
                x2,
                y2,
                color,
                width,
            } => {
                let color = parse_color(color).unwrap_or(colors.bg_active);
                ui.painter().with_clip_rect(clip).line_segment(
                    [
                        egui::pos2(origin.x + x1, origin.y + y1),
                        egui::pos2(origin.x + x2, origin.y + y2),
                    ],
                    egui::Stroke::new(*width, color),
                );
            }

            DrawCommand::Circle { cx, cy, r, fill } => {
                let center = egui::pos2(origin.x + cx, origin.y + cy);
                let color = parse_color(fill).unwrap_or(colors.accent);
                ui.painter().with_clip_rect(clip).circle_filled(center, *r, color);
            }

            DrawCommand::Arc {
                cx,
                cy,
                r,
                start_angle,
                end_angle,
                fill,
            } => {
                let color = parse_color(fill).unwrap_or(colors.accent);
                let center = egui::pos2(origin.x + cx, origin.y + cy);
                let span = (end_angle - start_angle).abs();
                let steps = ((r * span) as usize).max(8).min(128);
                let mut points = Vec::with_capacity(steps + 2);
                points.push(center);
                for i in 0..=steps {
                    let t = start_angle + (end_angle - start_angle) * i as f32 / steps as f32;
                    points.push(egui::pos2(center.x + r * t.cos(), center.y + r * t.sin()));
                }
                let shape = egui::Shape::Path(egui::epaint::PathShape {
                    points,
                    closed: true,
                    fill: color,
                    stroke: egui::epaint::PathStroke::NONE,
                });
                ui.painter().with_clip_rect(clip).add(shape);
            }

            DrawCommand::List {
                x,
                y,
                w,
                h,
                items,
                selected,
                item_height,
            } => {
                let row_h = if *item_height > 0.0 {
                    *item_height
                } else {
                    20.0
                };
                let list_w = if *w > 0.0 { *w } else { ui.available_width() };
                let list_h = if *h > 0.0 { *h } else { ui.available_height() };
                // List has its own built-in clip rect; intersect with the current clip stack.
                let list_abs_rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x + x, origin.y + y),
                    egui::vec2(list_w, list_h),
                );
                let clip_rect = clip.intersect(list_abs_rect);
                let painter = ui.painter().with_clip_rect(clip_rect);

                for (i, item) in items.iter().enumerate() {
                    let row_y = origin.y + y + i as f32 * row_h;
                    let row_rect = egui::Rect::from_min_size(
                        egui::pos2(origin.x + x, row_y),
                        egui::vec2(list_w, row_h),
                    );
                    if !clip_rect.intersects(row_rect) {
                        continue; // row is entirely outside the clip rect
                    }
                    let is_sel = i == *selected;
                    if is_sel {
                        painter.rect_filled(row_rect, 2.0, colors.bg_active);
                    }
                    let icon = if item.is_dir { "▶ " } else { "  " };
                    let label = format!("{}{}", icon, item.label);
                    painter.text(
                        egui::pos2(row_rect.min.x + 8.0, row_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &label,
                        egui::FontId::monospace(12.0),
                        if is_sel {
                            colors.text_primary
                        } else {
                            colors.text_dim
                        },
                    );
                    if let Some(sec) = &item.secondary {
                        painter.text(
                            egui::pos2(row_rect.max.x - 8.0, row_rect.center().y),
                            egui::Align2::RIGHT_CENTER,
                            sec,
                            egui::FontId::proportional(10.0),
                            colors.text_dim,
                        );
                    }
                }
            }

            // ── Host-measured layout primitives ──────────────────────────

            DrawCommand::Badge { x, y, label, fill, fg, font_size, radius } => {
                render_badge(ui, origin, clip, *x, *y, label, fill, fg, *font_size, *radius);
            }

            DrawCommand::KeyChip { x, y, label, font_size } => {
                render_key_chip_at(ui, origin, clip, *x, *y, label, *font_size, colors);
            }

            DrawCommand::KeyChipRow { x, y, keys, description, font_size } => {
                render_key_chip_row(ui, origin, clip, *x, *y, keys, description.as_deref(), *font_size, colors);
            }

            DrawCommand::Shortcuts { x, y, max_width, pairs, font_size } => {
                render_shortcuts(ui, origin, clip, *x, *y, *max_width, pairs, *font_size, colors);
            }

            // MeasureText is handled in routing.rs (needs a response channel);
            // it is never a frame-scoped visual command.
            DrawCommand::MeasureText { .. } => {}

            // TextInput is rendered as an interactive egui widget by
            // `process_app::mod` after this painter pass finishes — it
            // can't share the painter-only path because it needs a
            // mutable buffer + focus tracking. See `render_text_inputs`.
            DrawCommand::TextInput { .. } => {}

            // These are handled at the App trait level or routed upstream — never rendered.
            DrawCommand::Log { .. }
            | DrawCommand::FrameDone { .. }
            | DrawCommand::CapabilityRequest { .. }
            | DrawCommand::SecretGet { .. }
            | DrawCommand::RunGet { .. }
            | DrawCommand::RunComplete { .. }
            | DrawCommand::Notify { .. }
            | DrawCommand::PipeOpen { .. }
            | DrawCommand::PipeSend { .. }
            | DrawCommand::StatusSummary { .. }
            | DrawCommand::SpawnApp { .. }
            | DrawCommand::HttpRequest { .. }
            | DrawCommand::LlmRequest { .. }
            | DrawCommand::IqQuery { .. }
            | DrawCommand::CdRequest { .. }
            | DrawCommand::Image { .. }
            | DrawCommand::OpenVideo { .. }
            | DrawCommand::SetVideoState { .. }
            | DrawCommand::CloseVideo { .. }
            | DrawCommand::AudioMeter { .. }
            | DrawCommand::AudioPlay { .. }
            | DrawCommand::AudioCapture { .. }
            | DrawCommand::ListAudioDevices { .. }
            | DrawCommand::ListMidiDevices { .. }
            | DrawCommand::OpenMidiInput { .. }
            | DrawCommand::CloseMidiInput { .. }
            | DrawCommand::SendMidi { .. }
            | DrawCommand::Ready { .. }
            | DrawCommand::ScheduleRender { .. }
            | DrawCommand::SetTimer { .. }
            | DrawCommand::CancelTimer { .. }
            | DrawCommand::CopyToClipboard { .. }
            // AppendConversation is consumed by the host's agent-pane conversation
            // history surface (issue #285); it never paints into the draw canvas.
            // The host integration (forthcoming follow-up PR) drains it from the
            // command stream before this painter sees it; this arm is the safety
            // net for the wire-only landing.
            | DrawCommand::AppendConversation { .. } => {}
        }
    }

    // Frame-end invariant: the clip stack must be empty (balanced push/pop).
    // If not, log a warning and clear — this is an app bug but must not corrupt
    // subsequent frames or other panes.
    if !clip_stack.is_empty() {
        log::warn!(
            "render: clip stack not empty at frame end (depth={}); app sent unbalanced PushClip/PopClip",
            clip_stack.len()
        );
    }
}

// ── Render helpers for host-measured primitives ───────────────────────────────
//
// Called by the DrawCommand match arms above. Badge / KeyChip sizing uses
// real egui font metrics so app-emitted primitives and any future host-side
// overlays that reuse these helpers always match.

/// Render a Badge pill. `x` is the left edge; `y` is the vertical centre.
pub(crate) fn render_badge(
    ui: &mut egui::Ui,
    origin: egui::Pos2,
    clip: egui::Rect,
    x: f32,
    y_center: f32,
    label: &str,
    fill: &str,
    fg: &str,
    font_size: f32,
    radius: f32,
) {
    let fill_color = parse_color(fill).unwrap_or(egui::Color32::from_rgb(0x89, 0xb4, 0xfa));
    let fg_color = parse_color(fg).unwrap_or(egui::Color32::from_rgb(0x1e, 0x1e, 0x2e));
    let font_id = egui::FontId::proportional(font_size);
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font_id.clone(), fg_color));
    let text_w = galley.size().x;
    let text_h = galley.size().y;
    let pill_w = (text_w + style::BADGE_PAD_H * 2.0).max(style::BADGE_MIN_W);
    let pill_h = text_h + style::BADGE_PAD_V * 2.0;
    let pill_x = origin.x + x;
    let pill_y = origin.y + y_center - pill_h / 2.0;
    let pill_rect = egui::Rect::from_min_size(
        egui::pos2(pill_x, pill_y),
        egui::vec2(pill_w, pill_h),
    );
    let painter = ui.painter().with_clip_rect(clip);
    painter.rect_filled(pill_rect, radius, fill_color);
    // Centre text inside the pill using measured galley dimensions.
    let text_x = pill_rect.center().x - text_w / 2.0;
    let text_y = pill_rect.center().y - text_h / 2.0;
    painter.galley(egui::pos2(text_x, text_y), galley, fg_color);
}

/// Render a single KeyChip at absolute position (`origin.x + x`, `origin.y + y`).
/// Returns the chip width so callers can flow chips horizontally.
pub(crate) fn render_key_chip_at(
    ui: &mut egui::Ui,
    origin: egui::Pos2,
    clip: egui::Rect,
    x: f32,
    y: f32,
    label: &str,
    font_size: f32,
    colors: &Colors,
) -> f32 {
    let font_id = egui::FontId::monospace(font_size);
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font_id, colors.text_dim));
    let text_w = galley.size().x;
    let text_h = galley.size().y;
    let chip_w = (text_w + style::KEYCHIP_PAD_H * 2.0).max(style::KEYCHIP_MIN_W);
    let chip_h = text_h + style::KEYCHIP_PAD_V * 2.0;
    let chip_rect = egui::Rect::from_min_size(
        egui::pos2(origin.x + x, origin.y + y),
        egui::vec2(chip_w, chip_h),
    );
    let painter = ui.painter().with_clip_rect(clip);
    painter.rect_filled(chip_rect, egui::CornerRadius::same(3), colors.bg_active);
    painter.rect_stroke(
        chip_rect,
        egui::CornerRadius::same(3),
        egui::Stroke::new(1.0, colors.border),
        egui::StrokeKind::Inside,
    );
    let text_x = chip_rect.center().x - text_w / 2.0;
    let text_y = chip_rect.min.y + style::KEYCHIP_PAD_V;
    painter.galley(egui::pos2(text_x, text_y), galley, colors.text_dim);
    chip_w
}

/// Render a KeyChipRow: a sequence of chips followed by an optional description.
pub(crate) fn render_key_chip_row(
    ui: &mut egui::Ui,
    origin: egui::Pos2,
    clip: egui::Rect,
    x: f32,
    y: f32,
    keys: &[String],
    description: Option<&str>,
    font_size: f32,
    colors: &Colors,
) {
    let mut cursor_x = x;
    for (i, key) in keys.iter().enumerate() {
        if i > 0 {
            cursor_x += style::KEYCHIP_GAP;
        }
        let chip_w = render_key_chip_at(ui, origin, clip, cursor_x, y, key, font_size, colors);
        cursor_x += chip_w;
    }
    if let Some(desc) = description {
        cursor_x += style::KEYCHIP_DESC_GAP;
        // Measure one chip height to get vertical centre of the row.
        let font_id = egui::FontId::monospace(font_size);
        let sample = ui.fonts(|f| {
            f.layout_no_wrap("X".to_string(), font_id.clone(), colors.text_dim)
        });
        let chip_h = sample.size().y + style::KEYCHIP_PAD_V * 2.0;
        let desc_font = egui::FontId::proportional(font_size);
        ui.painter().with_clip_rect(clip).text(
            egui::pos2(origin.x + cursor_x, origin.y + y + chip_h / 2.0),
            egui::Align2::LEFT_CENTER,
            desc,
            desc_font,
            colors.text_dim,
        );
    }
}

/// Render a multi-group shortcut row with horizontal flow + multi-line wrap.
/// All chip widths come from real font metrics; the host owns the layout
/// entirely. SDK callers send one DrawCommand and trust the result.
///
/// Wrap rule: a pair fits on the current line iff `cursor_x + pair_width
/// <= max_width`. Otherwise advance to the next line at `cursor_x = x`,
/// `y += row_h`. The first pair on each line is always rendered (no
/// further wrap check) so a single pair wider than `max_width` still
/// renders, just past the budget.
pub(crate) fn render_shortcuts(
    ui: &mut egui::Ui,
    origin: egui::Pos2,
    clip: egui::Rect,
    x: f32,
    y: f32,
    max_width: f32,
    pairs: &[crate::app_protocol::ShortcutPair],
    font_size: f32,
    colors: &Colors,
) {
    use crate::style;

    // Pre-measure every chip + description so we can decide wrapping
    // before rendering. Storing the laid-out widths means we render in
    // a single linear pass, no re-measurement.
    let mono_font = egui::FontId::monospace(font_size);
    let prop_font = egui::FontId::proportional(font_size);

    // Each pair contributes: chip0 + (gap + chip1)* + desc_gap + desc_width.
    // Compute and cache per-pair total + per-chip widths.
    struct LaidPair {
        chip_widths: Vec<f32>,
        desc_w: f32,
        total_w: f32,
    }
    let mut laid: Vec<LaidPair> = Vec::with_capacity(pairs.len());

    let chip_h = {
        let g = ui.fonts(|f| {
            f.layout_no_wrap("X".to_string(), mono_font.clone(), colors.text_dim)
        });
        g.size().y + style::KEYCHIP_PAD_V * 2.0
    };

    for pair in pairs {
        let chip_widths: Vec<f32> = pair
            .keys
            .iter()
            .map(|k| {
                let g = ui.fonts(|f| {
                    f.layout_no_wrap(k.clone(), mono_font.clone(), colors.text_dim)
                });
                let text_w = g.size().x;
                (text_w + style::KEYCHIP_PAD_H * 2.0).max(style::KEYCHIP_MIN_W)
            })
            .collect();

        let chips_w: f32 = chip_widths.iter().sum::<f32>()
            + style::KEYCHIP_GAP * (chip_widths.len().saturating_sub(1) as f32);
        let desc_w = if pair.description.is_empty() {
            0.0
        } else {
            let g = ui.fonts(|f| {
                f.layout_no_wrap(
                    pair.description.clone(),
                    prop_font.clone(),
                    colors.text_dim,
                )
            });
            g.size().x
        };
        let desc_segment = if pair.description.is_empty() {
            0.0
        } else {
            style::KEYCHIP_DESC_GAP + desc_w
        };
        let total_w = chips_w + desc_segment;
        laid.push(LaidPair { chip_widths, desc_w, total_w });
    }

    // Inter-pair gap when flowing on the same line. Wider than the gap
    // between chips inside a single pair so visual groups read clearly.
    let pair_gap: f32 = 16.0;
    let row_h = chip_h + 4.0; // matches FooterKeys ROW_H aesthetic

    let mut cursor_x = x;
    let mut cursor_y = y;
    let mut on_line_first = true;

    for (pair, lp) in pairs.iter().zip(laid.iter()) {
        let pre_gap = if on_line_first { 0.0 } else { pair_gap };
        if !on_line_first && cursor_x + pre_gap + lp.total_w > x + max_width {
            // Wrap.
            cursor_x = x;
            cursor_y += row_h;
            on_line_first = true;
        }
        if !on_line_first {
            cursor_x += pair_gap;
        }

        // Render chips left-to-right.
        let mut chip_x = cursor_x;
        for (i, key) in pair.keys.iter().enumerate() {
            if i > 0 {
                chip_x += style::KEYCHIP_GAP;
            }
            let _ = render_key_chip_at(ui, origin, clip, chip_x, cursor_y, key, font_size, colors);
            chip_x += lp.chip_widths[i];
        }

        // Description.
        if !pair.description.is_empty() {
            let desc_x = chip_x + style::KEYCHIP_DESC_GAP;
            ui.painter().with_clip_rect(clip).text(
                egui::pos2(origin.x + desc_x, origin.y + cursor_y + chip_h / 2.0),
                egui::Align2::LEFT_CENTER,
                &pair.description,
                prop_font.clone(),
                colors.text_dim,
            );
        }

        cursor_x += lp.total_w;
        on_line_first = false;
        let _ = lp.desc_w; // (unused beyond total_w; kept on the struct for future tuning)
    }
}

fn font_family_for_text(monospace: bool) -> egui::FontFamily {
    if monospace {
        egui::FontFamily::Monospace
    } else {
        egui::FontFamily::Proportional
    }
}

/// Parse a hex color string like `"#1e1e2e"` into Color32.
pub(super) fn parse_color(hex: &str) -> Option<Color32> {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some(Color32::from_rgb(r, g, b))
    } else if hex.len() == 8 {
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
        Some(Color32::from_rgba_premultiplied(r, g, b, a))
    } else {
        None
    }
}
