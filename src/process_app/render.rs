//! Frame rendering — translates committed DrawCommands into egui paint calls.

use std::collections::HashMap;

use crate::app_protocol::{LayoutChild, LayoutDirection, RenderCommand, ResponsiveTier};
use crate::style;
use crate::theme::Colors;
use egui::Color32;
use taffy::prelude::*;

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
/// Returns true when `s` is a UUID v4 handle from `emit.load_image()`.
/// Format: 8-4-4-4-12 hex chars separated by hyphens (36 chars total).
/// where the renderer and the caller silently disagree about geometry. An
/// earlier version used `ui.min_rect()` here and got an empty rect (because
/// process_app paints via `ui.painter()` without allocating, so min_rect
/// never grows), which clipped every draw to nothing — all apps appeared
/// blank. Single-source-of-geometry: the caller hands us the rect once.
pub(crate) fn render_draw_commands(
    ui: &mut egui::Ui,
    pane_rect: egui::Rect,
    commands: &[RenderCommand],
    colors: &Colors,
    commonmark_cache: &mut egui_commonmark::CommonMarkCache,
    audio_peaks: &HashMap<String, f32>,
    image_cache: &mut super::image_cache::ImageCache,
    workspace_root: &std::path::Path,
    net_http_granted: bool,
    list_view_scroll_offsets: &mut HashMap<String, f32>,
    list_view_last_aligned_sel: &mut HashMap<String, usize>,
    outbound_events: &mut Vec<crate::app_protocol::PlexiEvent>,
    text_edit_buffers: &mut HashMap<String, String>,
    text_edit_focus_ctx: &mut crate::render_components::TextEditFocusCtx,
) {
    let origin = pane_rect.min;

    // Clip stack. Entries are absolute egui screen-space Rects.
    let mut clip_stack: Vec<egui::Rect> = Vec::new();

    for cmd in commands {
        // Resolve the current effective clip rect: top of stack or pane rect.
        let clip = clip_stack.last().copied().unwrap_or(pane_rect);

        match cmd {
            // ── Clip stack ────────────────────────────────────────────────────
            RenderCommand::PushClip { x, y, w, h } => {
                let new_rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x + x, origin.y + y),
                    egui::vec2(*w, *h),
                );
                // Intersect with current top so nested clips can only tighten.
                let effective = clip.intersect(new_rect);
                clip_stack.push(effective);
                continue;
            }

            RenderCommand::PopClip => {
                if clip_stack.pop().is_none() {
                    log::warn!("render: PopClip on empty clip stack (app bug)");
                }
                continue;
            }

            RenderCommand::Rect {
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

            RenderCommand::Text {
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
                max_lines,
            } => {
                let color = parse_color(color).unwrap_or(colors.text_primary);
                let family = font_family_for_text(*monospace);
                let font_id = egui::FontId::new(*size, family.clone());

                // max_lines: wrap text at max_width (or pane width) and clip to n rows with "…"
                if let Some(n) = max_lines {
                    if *n == 0 {
                        continue;
                    }
                    let wrap_w = max_width
                        .filter(|w| *w > 0.0)
                        .unwrap_or((pane_rect.max.x - (origin.x + x)).max(1.0));
                    let galley = ui.fonts(|f| {
                        f.layout(text.clone(), font_id.clone(), color, wrap_w)
                    });
                    let final_text: std::borrow::Cow<str> = if galley.rows.len() > *n as usize {
                        let mut lo = 0usize;
                        let mut hi = text.chars().count();
                        while lo + 1 < hi {
                            let mid = (lo + hi) / 2;
                            let candidate: String = text.chars().take(mid).collect::<String>() + "…";
                            let g = ui.fonts(|f| f.layout(candidate, font_id.clone(), color, wrap_w));
                            if g.rows.len() <= *n as usize {
                                lo = mid;
                            } else {
                                hi = mid;
                            }
                        }
                        std::borrow::Cow::Owned(text.chars().take(lo).collect::<String>() + "…")
                    } else {
                        std::borrow::Cow::Borrowed(text.as_str())
                    };
                    let final_galley = ui.fonts(|f| {
                        f.layout(final_text.to_string(), font_id.clone(), color, wrap_w)
                    });
                    let pos = egui::pos2(origin.x + x, origin.y + y);
                    ui.painter().with_clip_rect(clip).galley(pos, final_galley, color);
                    continue;
                }

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

            RenderCommand::Line {
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

            RenderCommand::Circle { cx, cy, r, fill } => {
                let center = egui::pos2(origin.x + cx, origin.y + cy);
                let color = parse_color(fill).unwrap_or(colors.accent);
                ui.painter().with_clip_rect(clip).circle_filled(center, *r, color);
            }

            RenderCommand::Arc {
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

            RenderCommand::List {
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

            RenderCommand::ListView {
                id,
                x,
                y,
                w,
                h,
                items,
                selected,
                loading,
                error,
            } => {
                use crate::app_protocol::ListViewLeading;

                log::debug!("ListView: id={id:?} items={} selected={selected} loading={loading}", items.len());

                let list_w = if *w > 0.0 { *w } else { pane_rect.width() };
                let list_h = if *h > 0.0 { *h } else { pane_rect.height() - y };
                let list_rect = egui::Rect::from_min_size(
                    egui::pos2(pane_rect.min.x + x, pane_rect.min.y + y),
                    egui::vec2(list_w, list_h),
                );
                let list_clip = clip.intersect(list_rect);

                use crate::app_protocol::ListViewItem as LVI;
                const LEADING_W: f32 = 48.0;
                const PAD_X: f32 = 12.0;
                const CHIP_GAP: f32 = 4.0;
                const CHIP_PAD_X: f32 = 8.0;
                const BADGE_TEXT_GAP: f32 = 8.0;
                const SCROLLBAR_W: f32 = 3.0;
                const FONT_CAPTION: f32 = 13.0;
                const FONT_HINT: f32 = 12.0;

                let heights: Vec<f32> = items.iter().map(|i| i.height()).collect();
                let total_h = if heights.is_empty() {
                    0.0
                } else {
                    heights.iter().sum::<f32>()
                };

                // Scroll-to-selected: only fire when the selection index changes.
                // Running this every frame would override mouse-wheel offsets on the
                // next repaint (selected item snaps back into view unconditionally).
                let scroll_y_ref = list_view_scroll_offsets.entry(id.clone()).or_insert(0.0);
                if !items.is_empty() {
                    let sel = (*selected).min(items.len().saturating_sub(1));
                    let last_sel = list_view_last_aligned_sel.get(id.as_str()).copied();
                    if last_sel != Some(sel) {
                        let mut item_top = 0.0f32;
                        for (i, h_item) in heights.iter().enumerate() {
                            if i == sel {
                                let item_bot = item_top + h_item;
                                if *h_item > list_h {
                                    *scroll_y_ref = item_top;
                                } else if item_top < *scroll_y_ref {
                                    *scroll_y_ref = item_top;
                                } else if item_bot > *scroll_y_ref + list_h {
                                    *scroll_y_ref = (item_bot - list_h).max(0.0);
                                }
                                break;
                            }
                            item_top += h_item;
                        }
                        list_view_last_aligned_sel.insert(id.clone(), sel);
                        log::debug!("list_view scroll-to-sel: id={id:?} sel={sel} offset={scroll_y_ref}");
                    }
                    let max_scroll = (total_h - list_h).max(0.0);
                    *scroll_y_ref = scroll_y_ref.clamp(0.0, max_scroll);
                }
                let scroll_y = *scroll_y_ref;

                let painter = ui.painter().with_clip_rect(list_clip);

                // Loading state: 8 skeleton rows
                if *loading {
                    let mut sy = list_rect.min.y;
                    for _ in 0..8 {
                        if sy > list_rect.max.y { break; }
                        let row_rect = egui::Rect::from_min_size(
                            egui::pos2(list_rect.min.x, sy),
                            egui::vec2(list_w, LVI::ROW_H_BASE),
                        );
                        painter.rect_filled(row_rect, 0.0, colors.terminal_bg);
                        let lead_rect = egui::Rect::from_min_size(
                            egui::pos2(list_rect.min.x + PAD_X, sy + (LVI::ROW_H_BASE - 20.0) / 2.0),
                            egui::vec2(20.0, 20.0),
                        );
                        painter.rect_filled(lead_rect, 10.0, colors.border);
                        let txt_rect = egui::Rect::from_min_size(
                            egui::pos2(list_rect.min.x + LEADING_W, sy + (LVI::ROW_H_BASE - FONT_CAPTION) / 2.0),
                            egui::vec2(list_w * 0.6, FONT_CAPTION),
                        );
                        painter.rect_filled(txt_rect, 3.0, colors.border);
                        sy += LVI::ROW_H_BASE;
                    }
                    // return is inside the match arm — use continue instead
                }
                // Error state
                else if let Some(err_msg) = error {
                    painter.text(
                        egui::pos2(list_rect.min.x + PAD_X, list_rect.min.y + PAD_X),
                        egui::Align2::LEFT_TOP,
                        err_msg.as_str(),
                        egui::FontId::proportional(FONT_CAPTION),
                        colors.text_dim,
                    );
                }
                // Empty state
                else if items.is_empty() {
                    painter.text(
                        egui::pos2(list_rect.min.x + PAD_X, list_rect.min.y + PAD_X),
                        egui::Align2::LEFT_TOP,
                        "No items.",
                        egui::FontId::proportional(FONT_CAPTION),
                        colors.text_dim,
                    );
                } else {
                    // Scrollbar (only when content overflows)
                    if total_h > list_h {
                        let track_x = list_rect.max.x - SCROLLBAR_W;
                        let thumb_ratio = list_h / total_h;
                        let thumb_h = (thumb_ratio * list_h).max(20.0);
                        let scroll_ratio = scroll_y / (total_h - list_h).max(1.0);
                        let thumb_y = list_rect.min.y + scroll_ratio * (list_h - thumb_h);
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(track_x, thumb_y),
                                egui::vec2(SCROLLBAR_W, thumb_h),
                            ),
                            1.5,
                            colors.border,
                        );
                    }

                    // Render visible rows
                    let mut item_top = 0.0f32;
                    for (i, item) in items.iter().enumerate() {
                        let h_item = heights[i];
                        let row_abs_y = list_rect.min.y + item_top - scroll_y;
                        item_top += h_item;

                        if row_abs_y + h_item < list_rect.min.y || row_abs_y > list_rect.max.y {
                            continue;
                        }

                        let row_rect = egui::Rect::from_min_size(
                            egui::pos2(list_rect.min.x, row_abs_y),
                            egui::vec2(list_w, h_item),
                        );
                        let is_sel = i == (*selected).min(items.len().saturating_sub(1));

                        let bg = if is_sel {
                            colors.bg_active
                        } else if i % 2 == 0 {
                            colors.terminal_bg
                        } else {
                            colors.bg_darkest
                        };
                        painter.rect_filled(row_rect, 0.0, bg);

                        match item {
                            LVI::CustomCell { .. } => {
                                // Custom cell: host only renders background. App draws over it.
                            }
                            LVI::Row(row) => {
                                let mid_y = row_rect.center().y;
                                let primary_y = if row.secondary.is_some() {
                                    row_rect.min.y + 8.0
                                } else {
                                    mid_y - FONT_CAPTION / 2.0
                                };

                                let text_start_x = match &row.leading {
                                    Some(ListViewLeading::Badge { label, color }) => {
                                        // render_badge takes origin + pane-local coords
                                        let badge_x = list_rect.min.x - pane_rect.min.x + PAD_X;
                                        let badge_y = row_abs_y - pane_rect.min.y + h_item / 2.0;
                                        let badge_rect = render_badge(
                                            ui,
                                            pane_rect.min,
                                            list_clip,
                                            badge_x,
                                            badge_y,
                                            label,
                                            color,
                                            contrast_text_hex(color),
                                            FONT_HINT,
                                            3.0,
                                        );
                                        // Flow title past badge's consumed rect so wide labels
                                        // (e.g. "#1792") never overlap the title. Keep at least
                                        // the fixed slot so short badges stay aligned.
                                        (list_rect.min.x + PAD_X + badge_rect.width() + BADGE_TEXT_GAP)
                                            .max(list_rect.min.x + LEADING_W)
                                    }
                                    Some(ListViewLeading::Avatar { handle }) => {
                                        let avatar_r = 14.0;
                                        let av_cx = list_rect.min.x + PAD_X + avatar_r;
                                        let av_cy = mid_y;
                                        if let Some(img_handle) = image_cache.get(handle) {
                                            let tex_id = img_handle.id();
                                            let n_segs: u32 = 32;
                                            let center = egui::pos2(av_cx, av_cy);
                                            let mut mesh = egui::Mesh::with_texture(tex_id);
                                            mesh.vertices.push(egui::epaint::Vertex {
                                                pos: center,
                                                uv: egui::pos2(0.5, 0.5),
                                                color: egui::Color32::WHITE,
                                            });
                                            for seg_i in 0..=n_segs {
                                                let angle = seg_i as f32 / n_segs as f32 * std::f32::consts::TAU;
                                                let cos = angle.cos();
                                                let sin = angle.sin();
                                                mesh.vertices.push(egui::epaint::Vertex {
                                                    pos: egui::pos2(center.x + cos * avatar_r, center.y + sin * avatar_r),
                                                    uv: egui::pos2(0.5 + cos * 0.5, 0.5 + sin * 0.5),
                                                    color: egui::Color32::WHITE,
                                                });
                                            }
                                            for seg_i in 0..n_segs {
                                                mesh.indices.push(0);
                                                mesh.indices.push(seg_i + 1);
                                                mesh.indices.push(seg_i + 2);
                                            }
                                            painter.add(egui::Shape::Mesh(std::sync::Arc::new(mesh)));
                                        } else {
                                            painter.circle_filled(
                                                egui::pos2(av_cx, av_cy),
                                                avatar_r,
                                                egui::Color32::from_rgb(0x3a, 0x3a, 0x4a),
                                            );
                                        }
                                        list_rect.min.x + LEADING_W
                                    }
                                    Some(ListViewLeading::Icon { name }) => {
                                        painter.text(
                                            egui::pos2(list_rect.min.x + PAD_X, mid_y),
                                            egui::Align2::LEFT_CENTER,
                                            name.as_str(),
                                            egui::FontId::proportional(FONT_CAPTION),
                                            colors.text_dim,
                                        );
                                        list_rect.min.x + LEADING_W
                                    }
                                    Some(ListViewLeading::None) | None => {
                                        list_rect.min.x + PAD_X
                                    }
                                };

                                // Trailing text (right-aligned)
                                let mut trailing_reserve = 0.0f32;
                                if let Some(trailing) = &row.trailing {
                                    let t_x = list_rect.max.x - PAD_X;
                                    painter.text(
                                        egui::pos2(t_x, primary_y),
                                        egui::Align2::RIGHT_TOP,
                                        trailing.as_str(),
                                        egui::FontId::proportional(FONT_HINT),
                                        colors.text_dim,
                                    );
                                    trailing_reserve = trailing.chars().count() as f32 * 6.0 + 16.0;
                                }

                                // Chips (right-aligned)
                                let chips_reserve: f32 = row.chips.iter()
                                    .map(|c| c.label.chars().count() as f32 * 7.0 + CHIP_PAD_X * 2.0 + CHIP_GAP)
                                    .sum();
                                let mut cx = list_rect.max.x - PAD_X - trailing_reserve - chips_reserve;
                                for chip in &row.chips {
                                    let chip_w = chip.label.chars().count() as f32 * 7.0 + CHIP_PAD_X * 2.0;
                                    let chip_color = parse_color(&chip.color)
                                        .unwrap_or(colors.accent);
                                    let chip_rect = egui::Rect::from_min_size(
                                        egui::pos2(cx, row_rect.min.y + (h_item - FONT_HINT - 4.0) / 2.0),
                                        egui::vec2(chip_w, FONT_HINT + 4.0),
                                    );
                                    painter.rect_filled(chip_rect, 3.0, chip_color.linear_multiply(0.2));
                                    painter.text(
                                        chip_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        chip.label.as_str(),
                                        egui::FontId::proportional(FONT_HINT),
                                        chip_color,
                                    );
                                    cx += chip_w + CHIP_GAP;
                                }

                                // Primary text — elide so the title never runs
                                // under the right-aligned chips / trailing text.
                                let primary_font = egui::FontId::proportional(FONT_CAPTION);
                                let has_right = !row.chips.is_empty() || row.trailing.is_some();
                                let text_right = if has_right {
                                    list_rect.max.x - PAD_X - trailing_reserve - chips_reserve - BADGE_TEXT_GAP
                                } else {
                                    list_rect.max.x - PAD_X
                                };
                                let avail_w = (text_right - text_start_x).max(0.0);
                                let display_primary: std::borrow::Cow<'_, str> = {
                                    let galley = ui.fonts(|f| {
                                        f.layout_no_wrap(
                                            row.primary.clone(),
                                            primary_font.clone(),
                                            colors.text_primary,
                                        )
                                    });
                                    if galley.size().x > avail_w {
                                        let mut lo = 0usize;
                                        let mut hi = row.primary.chars().count();
                                        while lo + 1 < hi {
                                            let mid = (lo + hi) / 2;
                                            let candidate: String =
                                                row.primary.chars().take(mid).collect::<String>() + "…";
                                            let g = ui.fonts(|f| {
                                                f.layout_no_wrap(candidate, primary_font.clone(), colors.text_primary)
                                            });
                                            if g.size().x <= avail_w { lo = mid; } else { hi = mid; }
                                        }
                                        std::borrow::Cow::Owned(
                                            row.primary.chars().take(lo).collect::<String>() + "…",
                                        )
                                    } else {
                                        std::borrow::Cow::Borrowed(row.primary.as_str())
                                    }
                                };
                                painter.text(
                                    egui::pos2(text_start_x, primary_y),
                                    egui::Align2::LEFT_TOP,
                                    display_primary.as_ref(),
                                    primary_font,
                                    colors.text_primary,
                                );

                                // Secondary text
                                if let Some(sec) = &row.secondary {
                                    painter.text(
                                        egui::pos2(text_start_x, primary_y + FONT_CAPTION + 4.0),
                                        egui::Align2::LEFT_TOP,
                                        sec.as_str(),
                                        egui::FontId::proportional(FONT_HINT),
                                        colors.text_dim,
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // ── Host-measured layout primitives ──────────────────────────

            RenderCommand::Badge { x, y, label, fill, fg, font_size, radius } => {
                render_badge(ui, origin, clip, *x, *y, label, fill, fg, *font_size, *radius);
            }

            RenderCommand::KeyChip { x, y, label, font_size } => {
                render_key_chip_at(ui, origin, clip, *x, *y, label, *font_size, colors);
            }

            RenderCommand::KeyChipRow { x, y, keys, description, font_size } => {
                render_key_chip_row(ui, origin, clip, *x, *y, keys, description.as_deref(), *font_size, colors);
            }

            RenderCommand::Shortcuts { x, y, max_width, pairs, font_size } => {
                render_shortcuts(ui, origin, clip, *x, *y, *max_width, pairs, *font_size, colors);
            }

            RenderCommand::TextRow { x, y, items, gap, align } => {
                render_text_row(ui, origin, clip, *x, *y, items, *gap, align, colors);
            }

            RenderCommand::Layout { x, y, direction, children, gap } => {
                render_layout_node(
                    ui, pane_rect, clip, *x, *y,
                    direction, children, *gap,
                    colors, commonmark_cache, audio_peaks,
                );
            }

            RenderCommand::Responsive { x, y, tiers } => {
                let avail_w = pane_rect.width();
                let avail_h = pane_rect.height();
                if let Some(tier) = select_responsive_tier(tiers, avail_w, avail_h) {
                    log::debug!(
                        "responsive: selected tier aspect={} for rect {}x{}",
                        tier.aspect, avail_w, avail_h
                    );
                    render_layout_node(
                        ui, pane_rect, clip, *x, *y,
                        &tier.direction, &tier.children, tier.gap,
                        colors, commonmark_cache, audio_peaks,
                    );
                } else {
                    log::warn!("responsive: no matching tier for rect {}x{}", avail_w, avail_h);
                }
            }

            RenderCommand::Markdown {
                x,
                y,
                w,
                text,
                base_size,
                color,
            } => {
                let text_color = parse_color(color).unwrap_or(colors.text_primary);
                let requested_rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x + x, origin.y + y),
                    egui::vec2(*w, pane_rect.max.y - (origin.y + y)),
                );
                let md_rect = requested_rect.intersect(clip);
                if !md_rect.is_positive() {
                    continue;
                }
                // Lay the markdown out at the full requested rect (whose top may
                // sit above the clip when a Scrollable has offset it upward), and
                // clip to the visible intersection. Using md_rect as the layout
                // rect would re-anchor content to the top of the visible area
                // every frame, so a scroll offset would move the scrollbar but
                // never the text.
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(requested_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT)),
                );
                child.set_clip_rect(md_rect);
                // Override default text colour so markdown body text matches the
                // app-specified colour. Header/code styling is handled internally
                // by egui_commonmark but inherits the base visuals.
                child.style_mut().visuals.override_text_color = Some(text_color);
                // Scale body font size to match the app's base_size.
                child.style_mut().text_styles.insert(
                    egui::TextStyle::Body,
                    egui::FontId::proportional(*base_size),
                );
                child.style_mut().text_styles.insert(
                    egui::TextStyle::Small,
                    egui::FontId::proportional(base_size * 0.85),
                );
                child.style_mut().text_styles.insert(
                    egui::TextStyle::Heading,
                    egui::FontId::proportional(base_size * 1.4),
                );
                child.style_mut().text_styles.insert(
                    egui::TextStyle::Monospace,
                    egui::FontId::monospace(base_size * 0.9),
                );
                egui_commonmark::CommonMarkViewer::new()
                    .show(&mut child, commonmark_cache, text);
            }

            // TextInput is rendered as an interactive egui widget by
            // `process_app::mod` after this painter pass finishes — it
            // can't share the painter-only path because it needs a
            // mutable buffer + focus tracking. See `render_text_inputs`.
            RenderCommand::TextInput { .. } => {}

            RenderCommand::Image { src, x, y, w, h, fit } => {
                if src.starts_with("http://") || src.starts_with("https://") {
                    image_cache.request_url(src, net_http_granted);
                } else {
                    image_cache.request(src, workspace_root);
                }
                // Handle-based srcs (UUID from emit.load_image) are already in the
                // cache from AppRequest::LoadImage processing. The request_url /
                // request calls above hit the contains_key early-return and no-op.
                let target_rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x + x, origin.y + y),
                    egui::vec2(*w, *h),
                );
                let painter = ui.painter().with_clip_rect(clip);
                if let Some(handle) = image_cache.get(src) {
                    let tex_size = handle.size_vec2();
                    let (dest_rect, uv) = fit_image(target_rect, tex_size, fit);
                    painter.image(handle.id(), dest_rect, uv, egui::Color32::WHITE);
                } else {
                    painter.rect_filled(
                        target_rect,
                        egui::CornerRadius::same(4),
                        egui::Color32::from_rgb(0x2a, 0x2a, 0x35),
                    );
                    let error_buf;
                    let state_text: &str = match image_cache.state(src) {
                        Some(super::image_cache::CachedImage::Error(msg)) => {
                            error_buf = format!("⚠ {msg}");
                            &error_buf
                        }
                        _ => "⏳ loading…",
                    };
                    let font = egui::FontId::proportional(crate::style::TEXT_HINT);
                    let wrap_width = (target_rect.width() - 12.0).max(0.0);
                    let galley = ui.fonts(|f| {
                        f.layout(state_text.to_string(), font, colors.text_dim, wrap_width)
                    });
                    let text_pos = egui::pos2(
                        target_rect.center().x - galley.size().x / 2.0,
                        target_rect.center().y - galley.size().y / 2.0,
                    );
                    painter.galley(text_pos, galley, colors.text_dim);
                }
            }

            RenderCommand::Avatar { src, cx, cy, radius } => {
                if src.starts_with("http://") || src.starts_with("https://") {
                    image_cache.request_url(src, net_http_granted);
                } else {
                    image_cache.request(src, workspace_root);
                }
                let center = egui::pos2(origin.x + cx, origin.y + cy);
                let painter = ui.painter().with_clip_rect(clip);
                if let Some(handle) = image_cache.get(src) {
                    let tex_id = handle.id();
                    let n_segs: u32 = 48;
                    let mut mesh = egui::Mesh::with_texture(tex_id);
                    // Centre vertex (UV 0.5, 0.5)
                    mesh.vertices.push(egui::epaint::Vertex {
                        pos: center,
                        uv: egui::pos2(0.5, 0.5),
                        color: egui::Color32::WHITE,
                    });
                    // Perimeter vertices
                    for i in 0..=n_segs {
                        let angle = i as f32 / n_segs as f32 * std::f32::consts::TAU;
                        let cos = angle.cos();
                        let sin = angle.sin();
                        mesh.vertices.push(egui::epaint::Vertex {
                            pos: egui::pos2(center.x + cos * radius, center.y + sin * radius),
                            uv: egui::pos2(0.5 + cos * 0.5, 0.5 + sin * 0.5),
                            color: egui::Color32::WHITE,
                        });
                    }
                    // Triangle fan: centre=0, perimeter i+1..i+2
                    for i in 0..n_segs {
                        mesh.indices.push(0);
                        mesh.indices.push(i + 1);
                        mesh.indices.push(i + 2);
                    }
                    painter.add(egui::Shape::Mesh(std::sync::Arc::new(mesh)));
                } else {
                    // Placeholder: filled circle with muted color
                    painter.circle_filled(center, *radius, egui::Color32::from_rgb(0x3a, 0x3a, 0x4a));
                }
            }

            RenderCommand::Skeleton { x, y, w, h, radius } => {
                let t = ui.input(|i| i.time) as f32;
                let alpha = 0.4 + 0.25 * (t * std::f32::consts::TAU * 0.8).sin();
                let fill = egui::Color32::from_rgba_unmultiplied(0x45, 0x47, 0x5a, (alpha * 255.0) as u8);
                let rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x + x, origin.y + y),
                    egui::vec2(*w, *h),
                );
                ui.painter().with_clip_rect(clip).rect_filled(
                    rect,
                    egui::CornerRadius::same(*radius as u8),
                    fill,
                );
                // Drive animation — request repaint at ~20fps while skeleton is visible
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
            }

            // AudioMeter (#341): renders a live peak-amplitude bar driven by
            // the cpal capture callback writing into `audio_peak_meters`.
            RenderCommand::AudioMeter { rect, pipe_id } => {
                let peak = *audio_peaks.get(pipe_id).unwrap_or(&0.0);
                let meter_rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x + rect.x, origin.y + rect.y),
                    egui::vec2(rect.w, rect.h),
                );
                // Background track
                ui.painter().with_clip_rect(clip).rect_filled(
                    meter_rect,
                    egui::CornerRadius::same(3),
                    egui::Color32::from_rgb(30, 30, 35),
                );
                // Filled bar proportional to peak amplitude [0, 1]
                let fill_w = (meter_rect.width() * peak.clamp(0.0, 1.0)).max(0.0);
                if fill_w > 0.0 {
                    let fill_rect = egui::Rect::from_min_size(
                        meter_rect.min,
                        egui::vec2(fill_w, meter_rect.height()),
                    );
                    // Color: green → yellow → red based on level
                    let color = if peak < 0.6 {
                        egui::Color32::from_rgb(80, 200, 80)
                    } else if peak < 0.85 {
                        egui::Color32::from_rgb(220, 180, 40)
                    } else {
                        egui::Color32::from_rgb(220, 60, 60)
                    };
                    ui.painter().with_clip_rect(clip).rect_filled(
                        fill_rect,
                        egui::CornerRadius::same(3),
                        color,
                    );
                }
            }

            // ── Host-managed scroll regions (#446) ───────────────────────────
            //
            // BeginScroll declares a clipped viewport. All draw commands until
            // the matching EndScroll are painted with that viewport as the clip
            // rect. The app has already translated its content coordinates by
            // the scroll offset (received via PlexiEvent::ScrollOffset); the
            // renderer's only job here is enforcing the clip boundary.
            //
            // Implementation mirrors PushClip/PopClip — the scroll viewport is
            // intersected with the current clip top so nested clips tighten
            // correctly, and the existing balanced-stack warning fires for
            // unmatched EndScroll calls.
            RenderCommand::BeginScroll { x, y, w, h, .. } => {
                let new_rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x + x, origin.y + y),
                    egui::vec2(*w, *h),
                );
                let effective = clip.intersect(new_rect);
                clip_stack.push(effective);
            }

            RenderCommand::EndScroll => {
                if clip_stack.pop().is_none() {
                    log::warn!("render: EndScroll on empty clip stack (app bug)");
                }
            }

            RenderCommand::ComponentTree { root } => {
                log::info!(
                    "render: ComponentTree received; rendering via render_component_tree; pane_origin={:?}",
                    pane_rect.min
                );
                let component_events =
                    crate::render_components::render_component_tree(ui, root, colors, text_edit_buffers, text_edit_focus_ctx);
                for evt in component_events {
                    log::info!(
                        "render: ComponentEvent node_id={} event_type={}",
                        evt.node_id,
                        evt.event_type
                    );
                    outbound_events.push(crate::app_protocol::PlexiEvent::ComponentEvent {
                        node_id: evt.node_id,
                        event_type: evt.event_type,
                        payload: evt.payload,
                    });
                }
            }
        }
    }

    // Frame-end invariant: the clip stack must be empty (balanced push/pop).
    // If not, log a warning and clear — this is an app bug but must not corrupt
    // subsequent frames or other panes.
    if !clip_stack.is_empty() {
        log::warn!(
            "render: clip stack not empty at frame end (depth={}); app sent unbalanced PushClip/PopClip or BeginScroll/EndScroll",
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
/// Renders a pill badge at (`origin.x + x`, vertically centred on `origin.y + y_center`).
/// Returns the pill width so callers can flow following content past the badge
/// instead of assuming a fixed slot (which clips wide labels like `#1792`).
/// Pick a legible text color for a filled chip/badge based on the fill's
/// perceptual luminance. Dark fills get light text, light fills get dark text.
/// Returns a hex string so it can be threaded through the `&str` fg params.
pub(crate) fn contrast_text_hex(fill: &str) -> &'static str {
    match parse_color(fill) {
        Some(c) => {
            let lum = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
            if lum > 140.0 { "#1e1e2e" } else { "#ffffff" }
        }
        None => "#1e1e2e",
    }
}

/// Render a badge pill centred vertically at `y_center` (pane-local).
/// Returns the consumed `Rect` in screen coordinates so callers can flow
/// the next element from `pill_rect.max.x`.
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
) -> egui::Rect {
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
    pill_rect
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

pub(crate) fn render_text_row(
    ui: &mut egui::Ui,
    origin: egui::Pos2,
    clip: egui::Rect,
    x: f32,
    y: f32,
    items: &[crate::app_protocol::TextRowItem],
    gap: f32,
    align: &str,
    colors: &Colors,
) {
    let mut cursor_x = x;
    for item in items {
        let font_id = if item.monospace {
            egui::FontId::monospace(item.size)
        } else {
            egui::FontId::proportional(item.size)
        };

        let color = parse_color(&item.color).unwrap_or(colors.text_primary);

        let galley = ui.fonts(|f| {
            f.layout_no_wrap(item.text.clone(), font_id, color)
        });

        let text_w = galley.size().x;

        let align2 = match align {
            "center" => egui::Align2::CENTER_CENTER,
            "left_top" => egui::Align2::LEFT_TOP,
            "left_center" => egui::Align2::LEFT_CENTER,
            _ => egui::Align2::LEFT_TOP,
        };

        let rect = align2.anchor_size(
            egui::pos2(origin.x + cursor_x, origin.y + y),
            galley.size(),
        );
        ui.painter().with_clip_rect(clip).galley(rect.min, galley, color);

        cursor_x += text_w + gap;
    }
}

fn font_family_for_text(monospace: bool) -> egui::FontFamily {
    if monospace {
        egui::FontFamily::Monospace
    } else {
        egui::FontFamily::Proportional
    }
}

// ── Declarative layout renderer (taffy flexbox) ───────────────────────────────

/// Measure the natural size of a leaf RenderCommand using egui font metrics.
/// Returns (width, height) in logical pixels.
/// Unsupported leaf types return (0.0, 0.0).
fn measure_leaf_size(ui: &egui::Ui, cmd: &RenderCommand) -> (f32, f32) {
    match cmd {
        RenderCommand::Badge { label, font_size, .. } => {
            let font_id = egui::FontId::proportional(*font_size);
            let galley = ui.fonts(|f| {
                f.layout_no_wrap(label.clone(), font_id, egui::Color32::WHITE)
            });
            let w = (galley.size().x + crate::style::BADGE_PAD_H * 2.0)
                .max(crate::style::BADGE_MIN_W);
            let h = galley.size().y + crate::style::BADGE_PAD_V * 2.0;
            (w, h)
        }
        RenderCommand::Text { text, size, monospace, .. } => {
            let font_id = if *monospace {
                egui::FontId::monospace(*size)
            } else {
                egui::FontId::proportional(*size)
            };
            let galley = ui.fonts(|f| {
                f.layout_no_wrap(text.clone(), font_id, egui::Color32::WHITE)
            });
            (galley.size().x, galley.size().y)
        }
        RenderCommand::KeyChip { label, font_size, .. } => {
            let font_id = egui::FontId::monospace(*font_size);
            let galley = ui.fonts(|f| {
                f.layout_no_wrap(label.clone(), font_id, egui::Color32::WHITE)
            });
            let w = (galley.size().x + crate::style::KEYCHIP_PAD_H * 2.0)
                .max(crate::style::KEYCHIP_MIN_W);
            let h = galley.size().y + crate::style::KEYCHIP_PAD_V * 2.0;
            (w, h)
        }
        _ => (0.0, 0.0),
    }
}

/// Build a taffy node for a LayoutChild. Leaves get measured sizes; nodes
/// get flex containers. Returns the NodeId in the given TaffyTree.
fn build_taffy_node<'a>(
    taffy: &mut TaffyTree<Option<&'a RenderCommand>>,
    child: &'a LayoutChild,
    ui: &egui::Ui,
) -> NodeId {
    match child {
        LayoutChild::Leaf { command } => {
            let (w, h) = measure_leaf_size(ui, command);
            let style = Style {
                size: taffy::geometry::Size {
                    width: length(w),
                    height: length(h),
                },
                ..Default::default()
            };
            taffy.new_leaf_with_context(style, Some(command.as_ref())).unwrap()
        }
        LayoutChild::Node { direction, children, gap } => {
            match direction {
                LayoutDirection::Stack => {
                    // Stack: all children layered at (0, 0). Container size = max of children.
                    let max_w = children
                        .iter()
                        .map(|c| match c {
                            LayoutChild::Leaf { command } => measure_leaf_size(ui, command).0,
                            _ => 0.0,
                        })
                        .fold(0.0f32, f32::max);
                    let max_h = children
                        .iter()
                        .map(|c| match c {
                            LayoutChild::Leaf { command } => measure_leaf_size(ui, command).1,
                            _ => 0.0,
                        })
                        .fold(0.0f32, f32::max);
                    let child_ids: Vec<NodeId> = children
                        .iter()
                        .map(|c| {
                            let (w, h) = match c {
                                LayoutChild::Leaf { command } => measure_leaf_size(ui, command),
                                _ => (0.0, 0.0),
                            };
                            let abs_style = Style {
                                position: Position::Absolute,
                                inset: taffy::geometry::Rect {
                                    left: auto(),
                                    right: auto(),
                                    top: length(0.0),
                                    bottom: auto(),
                                },
                                size: taffy::geometry::Size {
                                    width: length(w),
                                    height: length(h),
                                },
                                ..Default::default()
                            };
                            match c {
                                LayoutChild::Leaf { command } => taffy
                                    .new_leaf_with_context(abs_style, Some(command.as_ref()))
                                    .unwrap(),
                                _ => taffy.new_leaf_with_context(abs_style, None).unwrap(),
                            }
                        })
                        .collect();
                    let container_style = Style {
                        display: Display::Block,
                        size: taffy::geometry::Size {
                            width: length(max_w),
                            height: length(max_h),
                        },
                        ..Default::default()
                    };
                    taffy.new_with_children(container_style, &child_ids).unwrap()
                }
                LayoutDirection::Row | LayoutDirection::Column => {
                    let flex_dir = match direction {
                        LayoutDirection::Row => FlexDirection::Row,
                        LayoutDirection::Column => FlexDirection::Column,
                        LayoutDirection::Stack => unreachable!(),
                    };
                    let child_ids: Vec<NodeId> = children
                        .iter()
                        .map(|c| build_taffy_node(taffy, c, ui))
                        .collect();
                    let gap_size = match flex_dir {
                        FlexDirection::Column => taffy::geometry::Size {
                            width: length(0.0),
                            height: length(*gap),
                        },
                        _ => taffy::geometry::Size {
                            width: length(*gap),
                            height: length(0.0),
                        },
                    };
                    let style = Style {
                        display: Display::Flex,
                        flex_direction: flex_dir,
                        gap: gap_size,
                        ..Default::default()
                    };
                    taffy.new_with_children(style, &child_ids).unwrap()
                }
            }
        }
    }
}

/// Paint a resolved taffy node tree onto the egui Ui.
/// `abs_x`, `abs_y` — accumulated taffy layout offset from the root.
fn paint_node(
    taffy: &TaffyTree<Option<&RenderCommand>>,
    node: NodeId,
    abs_x: f32,
    abs_y: f32,
    ui: &mut egui::Ui,
    screen_origin: egui::Pos2,
    pane_x: f32,
    pane_y: f32,
    clip: egui::Rect,
    colors: &Colors,
) {
    let layout = taffy.layout(node).unwrap();
    let my_abs_x = abs_x + layout.location.x;
    let my_abs_y = abs_y + layout.location.y;

    if let Some(Some(cmd)) = taffy.get_node_context(node) {
        let abs_screen_x = screen_origin.x + pane_x + my_abs_x;
        let abs_screen_y = screen_origin.y + pane_y + my_abs_y;
        let leaf_h = layout.size.height;
        let paint_origin = egui::Pos2::ZERO;
        match cmd {
            RenderCommand::Badge { label, fill, fg, font_size, radius, .. } => {
                render_badge(
                    ui, paint_origin, clip,
                    abs_screen_x, abs_screen_y + leaf_h / 2.0,
                    label, fill, fg, *font_size, *radius,
                );
            }
            RenderCommand::Text { text, size, color, monospace, .. } => {
                let font_id = if *monospace {
                    egui::FontId::monospace(*size)
                } else {
                    egui::FontId::proportional(*size)
                };
                let text_color = parse_color(color).unwrap_or(colors.text_primary);
                let galley = ui.fonts(|f| {
                    f.layout_no_wrap(text.clone(), font_id, text_color)
                });
                ui.painter().with_clip_rect(clip).galley(
                    egui::pos2(abs_screen_x, abs_screen_y),
                    galley,
                    text_color,
                );
            }
            RenderCommand::KeyChip { label, font_size, .. } => {
                render_key_chip_at(
                    ui, paint_origin, clip,
                    abs_screen_x, abs_screen_y,
                    label, *font_size, colors,
                );
            }
            _ => {
                log::warn!("render_layout_node: unsupported leaf command type, skipping");
            }
        }
    }

    for child in taffy.children(node).unwrap() {
        paint_node(taffy, child, my_abs_x, my_abs_y, ui, screen_origin, pane_x, pane_y, clip, colors);
    }
}

/// Resolve a declarative flex layout tree using taffy, then paint each leaf.
///
/// `pane_x`, `pane_y` — pane-relative anchor of this layout node (from the Layout command).
/// `direction`, `children`, `gap` — the root node's layout properties.
pub(crate) fn render_layout_node(
    ui: &mut egui::Ui,
    pane_rect: egui::Rect,
    clip: egui::Rect,
    pane_x: f32,
    pane_y: f32,
    direction: &LayoutDirection,
    children: &[LayoutChild],
    gap: f32,
    colors: &Colors,
    _commonmark_cache: &mut egui_commonmark::CommonMarkCache,
    _audio_peaks: &HashMap<String, f32>,
) {
    // ── Phase 1: Build taffy tree ─────────────────────────────────────────────
    let mut taffy: TaffyTree<Option<&RenderCommand>> = TaffyTree::new();

    // Stack at the root: overlay all leaf children at the same origin.
    if matches!(direction, LayoutDirection::Stack) {
        let screen_origin = pane_rect.min;
        for child in children {
            if let LayoutChild::Leaf { command } = child {
                let (leaf_w, leaf_h) = measure_leaf_size(ui, command);
                let abs_screen_x = screen_origin.x + pane_x;
                let abs_screen_y = screen_origin.y + pane_y;
                let paint_origin = egui::Pos2::ZERO;
                match command.as_ref() {
                    RenderCommand::Badge { label, fill, fg, font_size, radius, .. } => {
                        render_badge(
                            ui, paint_origin, clip,
                            abs_screen_x, abs_screen_y + leaf_h / 2.0,
                            label, fill, fg, *font_size, *radius,
                        );
                    }
                    RenderCommand::Text { text, size, color, monospace, .. } => {
                        let font_id = if *monospace {
                            egui::FontId::monospace(*size)
                        } else {
                            egui::FontId::proportional(*size)
                        };
                        let text_color = parse_color(color).unwrap_or(colors.text_primary);
                        let galley = ui.fonts(|f| {
                            f.layout_no_wrap(text.clone(), font_id, text_color)
                        });
                        ui.painter().with_clip_rect(clip).galley(
                            egui::pos2(abs_screen_x, abs_screen_y),
                            galley,
                            text_color,
                        );
                    }
                    RenderCommand::KeyChip { label, font_size, .. } => {
                        render_key_chip_at(
                            ui, paint_origin, clip,
                            abs_screen_x, abs_screen_y,
                            label, *font_size, colors,
                        );
                    }
                    _ => {
                        log::warn!("render_layout_node: unsupported stack leaf type, skipping");
                    }
                }
                let _ = leaf_w; // size used for hit-rect in future; paint uses leaf_h only now
            } else {
                log::warn!("render_layout_node: nested nodes inside root Stack not yet supported");
            }
        }
        log::debug!("render_layout_node: painted stack at pane ({pane_x}, {pane_y})");
        return;
    }

    let flex_dir = match direction {
        LayoutDirection::Row => FlexDirection::Row,
        LayoutDirection::Column => FlexDirection::Column,
        LayoutDirection::Stack => unreachable!(),
    };
    let child_ids: Vec<NodeId> = children
        .iter()
        .map(|c| build_taffy_node(&mut taffy, c, ui))
        .collect();
    let gap_size = match flex_dir {
        FlexDirection::Column => taffy::geometry::Size {
            width: length(0.0),
            height: length(gap),
        },
        _ => taffy::geometry::Size {
            width: length(gap),
            height: length(0.0),
        },
    };
    let root_style = Style {
        display: Display::Flex,
        flex_direction: flex_dir,
        gap: gap_size,
        ..Default::default()
    };
    let root = taffy.new_with_children(root_style, &child_ids).unwrap();

    // ── Phase 2: Compute layout ───────────────────────────────────────────────
    let available = taffy::geometry::Size {
        width: AvailableSpace::Definite(pane_rect.width() - pane_x),
        height: AvailableSpace::MaxContent,
    };
    taffy.compute_layout(root, available).unwrap();

    // ── Phase 3: Paint leaves ─────────────────────────────────────────────────
    let screen_origin = pane_rect.min;
    paint_node(&taffy, root, 0.0, 0.0, ui, screen_origin, pane_x, pane_y, clip, colors);

    log::debug!("render_layout_node: painted layout tree at pane ({pane_x}, {pane_y})");
}

/// Parse a hex color string like `"#1e1e2e"` into Color32.
pub(crate) fn parse_color(hex: &str) -> Option<Color32> {
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

/// Compute destination rect and UV coordinates for an image given a fit mode.
///
/// - `"contain"` (default): scale to fit inside target, preserving aspect ratio (letterbox).
/// - `"cover"`: fill target entirely, preserving aspect ratio (crop edges).
/// - `"fill"`: stretch to fill target exactly (no aspect ratio preservation).
fn fit_image(
    target: egui::Rect,
    tex_size: egui::Vec2,
    fit: &str,
) -> (egui::Rect, egui::Rect) {
    let full_uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    if tex_size.x <= 0.0 || tex_size.y <= 0.0 || target.width() <= 0.0 || target.height() <= 0.0 {
        return (target, full_uv);
    }
    match fit {
        "cover" => {
            let scale = (target.width() / tex_size.x).max(target.height() / tex_size.y);
            let scaled = tex_size * scale;
            // Crop UV to show only the centered portion.
            let u_offset = (scaled.x - target.width()) / 2.0 / scaled.x;
            let v_offset = (scaled.y - target.height()) / 2.0 / scaled.y;
            let uv = egui::Rect::from_min_max(
                egui::pos2(u_offset, v_offset),
                egui::pos2(1.0 - u_offset, 1.0 - v_offset),
            );
            (target, uv)
        }
        "fill" => (target, full_uv),
        _ => {
            // "contain" — letterbox
            let scale = (target.width() / tex_size.x).min(target.height() / tex_size.y);
            let dest_size = tex_size * scale;
            let offset = (target.size() - dest_size) * 0.5;
            let dest = egui::Rect::from_min_size(target.min + offset, dest_size);
            (dest, full_uv)
        }
    }
}

// ── Responsive tier selection ──────────────────────────────────────────────────

/// Select the first matching responsive tier based on the available rect's aspect ratio.
/// - "landscape": width > height
/// - "portrait": height > width
/// - "square": ratio between 0.83 and 1.2 (fallback)
pub(crate) fn select_responsive_tier(
    tiers: &[ResponsiveTier],
    width: f32,
    height: f32,
) -> Option<&ResponsiveTier> {
    let ratio = if height > 0.0 { width / height } else { f32::MAX };
    tiers.iter().find(|tier| match tier.aspect.as_str() {
        "landscape" => ratio > 1.2,
        "portrait" => ratio < 0.83,
        "square" => true, // fallback — matches anything not caught above
        _ => false,
    })
}
