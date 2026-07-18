// Scene-graph renderer — maps a WASM app's arena `UiTree` onto egui widgets.
//
// The tree is a flat list of `IndexedNode`s referenced by u32 id (the WIT model
// avoids recursive types). Rendering walks from `tree.root`, looking children
// up by id. Interactive nodes (buttons, list rows, submitted inputs) collect
// their declared action strings into `RenderResult` so the pane can route them
// back to the guest. A depth cap guards against malformed/cyclic trees.

use egui::{Color32, RichText};
use std::collections::HashMap;

use crate::ui::style;
use crate::ui::theme::Colors;

use super::wasm_app::bindings::plexi::platform::types::{CanvasCommand, FooterKeysNode, PinnedEdge};
use super::wasm_app::{Alignment, BadgeColor, ButtonStyle, Color, IndexedNode, UiNodeData, UiTree};

use crate::render::app_chrome::{self, AppChrome};

const MAX_DEPTH: u32 = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanvasFit {
    #[default]
    Fill,
    Contain,
}

/// A canvas click reported in the app's own declared canvas coordinate
/// space (post `canvas_transform` inversion), not screen pixels. See
/// `MouseEvent` in `sdk/python/plexi_sdk/events.py`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasClick {
    pub x: f32,
    pub y: f32,
    pub button: Option<&'static str>,
    pub pressed: bool,
}

/// Interactions produced by one render pass, to be translated into guest input.
#[derive(Default, Debug)]
pub struct RenderResult {
    /// Action strings from clicked buttons, selected list rows, or submitted inputs.
    pub actions: Vec<String>,
    /// `(on_change action, new value)` pairs from edited text inputs.
    pub value_changes: Vec<(String, String)>,
    /// Canvas clicks, already inverted into the app's declared canvas space.
    pub canvas_clicks: Vec<CanvasClick>,
    pub canvas_time: std::time::Duration,
}

/// Render a view tree, compositing `surface` into the first surface-node
/// (the guest's GPU output). Pass `None` to draw a placeholder instead.
pub fn render_ui_tree_with_surface(
    ui: &mut egui::Ui,
    tree: &UiTree,
    colors: &Colors,
    surface: Option<egui::TextureId>,
    surface_key: Option<crate::ui::focus::SurfaceKey>,
) -> RenderResult {
    render_ui_tree_with_canvas_fits(ui, tree, colors, surface, None, None, surface_key)
}

pub fn render_ui_tree_with_canvas_fits(
    ui: &mut egui::Ui,
    tree: &UiTree,
    colors: &Colors,
    surface: Option<egui::TextureId>,
    canvas_fits: Option<&HashMap<u32, CanvasFit>>,
    pending_click: Option<crate::host::pane::PendingPaneClick>,
    surface_key: Option<crate::ui::focus::SurfaceKey>,
) -> RenderResult {
    let mut out = RenderResult::default();
    render_node(
        ui,
        &tree.nodes,
        tree.root,
        colors,
        &mut out,
        0,
        surface,
        canvas_fits,
        pending_click,
        surface_key,
    );
    out
}

/// Headless render of a `UiTree` to PNG bytes via `egui_kittest`'s wgpu
/// offscreen backend — the same rasterization path `PlexiUiHarness` uses for
/// screenshot tests, reused here for `plexi app render --png` /
/// `plexi app check --png-dir` so the CLI renders exactly what the live host
/// would paint instead of hand-rolling a second rasterizer for widget chrome
/// (buttons, fonts, footer key chips) that has no flat-primitive form.
pub fn render_ui_tree_to_png(tree: &UiTree, width: f32, height: f32) -> Result<Vec<u8>, String> {
    let colors = crate::ui::theme::colors_from_config(&crate::config::PlexiConfig::load());
    let tree = tree.clone();
    // `set_fonts` only takes effect on the *next* frame's `begin_frame`, so the
    // custom `ui-medium` family used by declarative Button/ListRow nodes is not
    // bound during the frame that calls it. Apply fonts on a first, content-less
    // frame and render the tree only once they are live — otherwise laying out
    // any button label on the first frame panics in epaint. The live host does
    // not hit this because `setup_fonts` runs once at startup, long before any
    // app frame.
    let mut fonts_ready = false;
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::Vec2::new(width, height))
        .build(move |ctx| {
            if !fonts_ready {
                crate::ui::theme::setup_fonts(ctx);
                fonts_ready = true;
                ctx.request_repaint();
                return;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let _ = render_ui_tree_with_surface(ui, &tree, &colors, None, None);
                });
        });
    harness.run();
    let img = harness
        .render()
        .map_err(|e| format!("offscreen render failed: {e}"))?;
    let mut bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|e| format!("PNG encode failed: {e}"))?;
    Ok(bytes)
}

fn rgba(c: &Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn canvas_align(align: Alignment) -> egui::Align2 {
    match align {
        Alignment::Start | Alignment::Stretch => egui::Align2::LEFT_TOP,
        Alignment::Center => egui::Align2::CENTER_CENTER,
        Alignment::End => egui::Align2::RIGHT_TOP,
    }
}

fn canvas_transform(
    rect: egui::Rect,
    width: f32,
    height: f32,
    fit: CanvasFit,
) -> (egui::Pos2, f32, f32) {
    let sx = if width > 0.0 {
        rect.width() / width
    } else {
        1.0
    };
    let sy = if height > 0.0 {
        rect.height() / height
    } else {
        1.0
    };
    if width <= 0.0 || height <= 0.0 || fit == CanvasFit::Fill {
        return (rect.min, sx, sy);
    }
    let scale = sx.min(sy);
    let content_size = egui::vec2(width * scale, height * scale);
    let origin = rect.center() - content_size / 2.0;
    (origin, scale, scale)
}

fn cross_align(a: Alignment) -> egui::Align {
    match a {
        Alignment::Start | Alignment::Stretch => egui::Align::Min,
        Alignment::Center => egui::Align::Center,
        Alignment::End => egui::Align::Max,
    }
}

/// True when `pending_click` (from `AppRequest::ClickPaneNode`/
/// `HostHarness::inject_node_click`) targets this frame's node `id` by its
/// arena id — the same honest match `PaneClickTarget::Pos` does by rect
/// containment, just keyed on identity instead of geometry.
fn node_click_matches(pending_click: Option<crate::host::pane::PendingPaneClick>, id: u32) -> bool {
    matches!(
        pending_click.map(|c| c.target),
        Some(crate::host::pane::PaneClickTarget::Node(n)) if n == id
    )
}

fn find_node(nodes: &[IndexedNode], id: u32) -> Option<&IndexedNode> {
    nodes
        .get(id as usize)
        .filter(|node| node.id == id)
        .or_else(|| nodes.iter().find(|node| node.id == id))
}

/// If `child_id` is a `Pinned{edge: Bottom}` wrapper around a `FooterKeys` node,
/// or is itself a bare trailing `FooterKeys` node, return the id to render and
/// its footer data. Used by `Column` to reserve a flush-bottom footer slot,
/// mirroring the legacy renderer's bottom-pin partition (components.rs
/// `render_stack`'s `StackDirection::Vertical` branch).
fn column_bottom_pin(nodes: &[IndexedNode], child_id: u32) -> Option<(u32, &FooterKeysNode)> {
    let node = find_node(nodes, child_id)?;
    match &node.data {
        UiNodeData::Pinned(p) if p.edge == PinnedEdge::Bottom => {
            let inner = find_node(nodes, p.child)?;
            match &inner.data {
                UiNodeData::FooterKeys(f) => Some((p.child, f)),
                _ => None,
            }
        }
        UiNodeData::FooterKeys(f) => Some((child_id, f)),
        _ => None,
    }
}

fn render_node(
    ui: &mut egui::Ui,
    nodes: &[IndexedNode],
    id: u32,
    colors: &Colors,
    out: &mut RenderResult,
    depth: u32,
    surface: Option<egui::TextureId>,
    canvas_fits: Option<&HashMap<u32, CanvasFit>>,
    pending_click: Option<crate::host::pane::PendingPaneClick>,
    surface_key: Option<crate::ui::focus::SurfaceKey>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(node) = find_node(nodes, id) else {
        return;
    };

    match &node.data {
        UiNodeData::Empty => {}

        UiNodeData::Text(t) => {
            let mut rich = RichText::new(&t.text).size(t.size.unwrap_or(style::TEXT_BODY));
            if t.bold {
                rich = rich.strong();
            }
            if let Some(c) = &t.color {
                rich = rich.color(rgba(c));
            }
            let mut label = egui::Label::new(rich);
            if t.truncate {
                label = label.truncate();
            }
            ui.add(label);
        }

        UiNodeData::Button(b) => {
            let fill = match b.style {
                ButtonStyle::Primary => colors.accent,
                ButtonStyle::Secondary => colors.bg_active,
                ButtonStyle::Danger => colors.danger,
                ButtonStyle::Ghost => Color32::TRANSPARENT,
            };
            let label = RichText::new(&b.label).color(colors.text_on(fill));
            let btn = egui::Button::new(label)
                .fill(fill)
                .corner_radius(style::RADIUS_SM);
            let synthetic_click = !b.disabled && node_click_matches(pending_click, id);
            if ui.add_enabled(!b.disabled, btn).clicked() || synthetic_click {
                out.actions.push(b.on_click.clone());
            }
        }

        UiNodeData::TextInput(ti) => {
            let mut buf = ti.value.clone();
            let edit = egui::TextEdit::singleline(&mut buf)
                .password(ti.password)
                .hint_text(&ti.placeholder);
            let resp = ui.add(edit);
            if let Some(key) = surface_key {
                crate::ui::focus::register_text_surface(ui.ctx(), key, resp.id);
                // Node-targeted clicks only focus the field — keystroke entry
                // into a focused node is out of scope (stint 0414 non-scope).
                // The claim routes through the reconciler (stint 0429), which
                // grants it while this pane owns input.
                if node_click_matches(pending_click, id) {
                    crate::ui::focus::claim_text_surface(ui.ctx(), key, resp.id);
                }
            }
            if resp.changed() && buf != ti.value {
                out.value_changes.push((ti.on_change.clone(), buf.clone()));
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                out.actions.push(ti.on_submit.clone());
            }
        }

        UiNodeData::Row(r) => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = r.gap;
                for child in &r.children {
                    render_node(ui, nodes, *child, colors, out, depth + 1, surface, canvas_fits, pending_click, surface_key);
                }
            });
        }

        UiNodeData::Column(c) => {
            let footer = c
                .children
                .last()
                .copied()
                .and_then(|last_id| column_bottom_pin(nodes, last_id));
            if let Some((footer_id, footer_data)) = footer {
                let footer_h =
                    app_chrome::footer_keys_height(ui, &footer_data.entries, footer_data.divider);
                let stack_size = egui::vec2(ui.available_width(), ui.available_height());
                let (stack_rect, _) = ui.allocate_exact_size(stack_size, egui::Sense::hover());
                let body_h = (stack_rect.height() - footer_h).max(0.0);
                let body_rect =
                    egui::Rect::from_min_size(stack_rect.min, egui::vec2(stack_rect.width(), body_h));

                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(body_rect), |ui| {
                    ui.set_clip_rect(body_rect);
                    ui.set_min_height(body_h);
                    ui.set_max_height(body_h);
                    ui.with_layout(egui::Layout::top_down(cross_align(c.align)), |ui| {
                        ui.spacing_mut().item_spacing.y = c.gap;
                        for &child in &c.children[..c.children.len() - 1] {
                            render_node(ui, nodes, child, colors, out, depth + 1, surface, canvas_fits, pending_click, surface_key);
                        }
                    });
                });

                let footer_rect = egui::Rect::from_min_size(
                    egui::pos2(stack_rect.min.x, stack_rect.max.y - footer_h),
                    egui::vec2(stack_rect.width(), footer_h),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
                    ui.set_clip_rect(footer_rect);
                    ui.set_min_height(footer_h);
                    ui.set_max_height(footer_h);
                    render_node(ui, nodes, footer_id, colors, out, depth + 1, surface, canvas_fits, pending_click, surface_key);
                });
            } else {
                ui.with_layout(egui::Layout::top_down(cross_align(c.align)), |ui| {
                    ui.spacing_mut().item_spacing.y = c.gap;
                    for child in &c.children {
                        render_node(ui, nodes, *child, colors, out, depth + 1, surface, canvas_fits, pending_click, surface_key);
                    }
                });
            }
        }

        UiNodeData::ProgressBar(p) => {
            let frac = if p.max > 0.0 {
                (p.value / p.max).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut bar = egui::ProgressBar::new(frac);
            if let Some(label) = &p.label {
                bar = bar.text(label);
            }
            if let Some(c) = &p.color {
                bar = bar.fill(rgba(c));
            }
            ui.add(bar);
        }

        UiNodeData::Badge(b) => {
            let fill = match b.color {
                BadgeColor::Accent => colors.accent,
                BadgeColor::Success => colors.success,
                BadgeColor::Warning => colors.warning,
                BadgeColor::Danger => colors.danger,
                BadgeColor::Neutral => colors.bg_active,
            };
            egui::Frame::new()
                .fill(fill)
                .inner_margin(egui::Margin::symmetric(6, 2))
                .corner_radius(style::RADIUS_SM)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(&b.text)
                            .size(style::TEXT_META)
                            .color(colors.text_on(fill)),
                    );
                });
        }

        UiNodeData::ListView(l) => {
            for (i, item_id) in l.items.iter().enumerate() {
                let selected = l.selected == Some(i as u32);
                let resp = ui
                    .scope(|ui| {
                        if selected {
                            ui.visuals_mut().override_text_color = Some(colors.accent);
                        }
                        render_node(ui, nodes, *item_id, colors, out, depth + 1, surface, canvas_fits, pending_click, surface_key);
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.clicked() || node_click_matches(pending_click, *item_id) {
                    if let Some(action) = &l.on_select {
                        out.actions.push(action.clone());
                    }
                }
            }
        }

        UiNodeData::Scroll(s) => {
            let area = if s.horizontal {
                egui::ScrollArea::horizontal()
            } else {
                egui::ScrollArea::vertical()
            };
            area.show(ui, |ui| {
                render_node(ui, nodes, s.child, colors, out, depth + 1, surface, canvas_fits, pending_click, surface_key);
            });
        }

        UiNodeData::Padding(p) => {
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: p.left as i8,
                    right: p.right as i8,
                    top: p.top as i8,
                    bottom: p.bottom as i8,
                })
                .show(ui, |ui| {
                    render_node(ui, nodes, p.child, colors, out, depth + 1, surface, canvas_fits, pending_click, surface_key);
                });
        }

        UiNodeData::Canvas(c) => {
            let canvas_started = std::time::Instant::now();
            let width = if c.grow {
                ui.available_width().max(1.0)
            } else if c.width > 0.0 {
                c.width.min(ui.available_width()).max(1.0)
            } else {
                ui.available_width().max(1.0)
            };
            let height = if c.grow {
                ui.available_height().max(1.0)
            } else {
                c.height
            };
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(width, height.max(1.0)), egui::Sense::click());
            let fit = canvas_fits
                .and_then(|fits| fits.get(&id))
                .copied()
                .unwrap_or_default();
            let (origin, sx, sy) = canvas_transform(rect, c.width, c.height, fit);
            // A real click is detected by egui's own `Sense::click()` (resolved
            // once per pass, inside `Context::begin_pass`, from that pass's
            // actual `RawInput` — it cannot be faked by mutating `ctx.input_mut()`
            // after the pass has started). `plexi pane click`/`HostHarness::
            // inject_click` deliver a `PendingPaneClick` instead, matched
            // against this frame's freshly-computed `rect` — the same
            // honest hit-test a real click would need, just resolved
            // explicitly rather than via egui's internal interact_widgets.
            let synthetic = pending_click.filter(|c| {
                matches!(c.target, crate::host::pane::PaneClickTarget::Pos(pos) if rect.contains(pos))
            });
            let pixel_pos = resp.interact_pointer_pos().or_else(|| {
                synthetic.and_then(|c| match c.target {
                    crate::host::pane::PaneClickTarget::Pos(pos) => Some(pos),
                    crate::host::pane::PaneClickTarget::Node(_) => None,
                })
            });
            if resp.clicked() || synthetic.is_some() {
                if let Some(pixel_pos) = pixel_pos {
                    let button = if let Some(c) = synthetic {
                        Some(c.button)
                    } else if resp.clicked_by(egui::PointerButton::Secondary) {
                        Some("right")
                    } else if resp.clicked_by(egui::PointerButton::Middle) {
                        Some("middle")
                    } else {
                        Some("left")
                    };
                    out.canvas_clicks.push(CanvasClick {
                        x: (pixel_pos.x - origin.x) / sx,
                        y: (pixel_pos.y - origin.y) / sy,
                        button,
                        pressed: true,
                    });
                }
            }
            for command in &c.commands {
                match command {
                    CanvasCommand::Rect(r) => {
                        let min = egui::pos2(origin.x + r.x * sx, origin.y + r.y * sy);
                        let size = egui::vec2(r.width * sx, r.height * sy);
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(min, size),
                            r.radius * sx.min(sy),
                            rgba(&r.fill),
                        );
                    }
                    CanvasCommand::Circle(circle) => {
                        ui.painter().circle_filled(
                            egui::pos2(origin.x + circle.x * sx, origin.y + circle.y * sy),
                            circle.radius * sx.min(sy),
                            rgba(&circle.fill),
                        );
                    }
                    CanvasCommand::Line(line) => {
                        ui.painter().line_segment(
                            [
                                egui::pos2(origin.x + line.x1 * sx, origin.y + line.y1 * sy),
                                egui::pos2(origin.x + line.x2 * sx, origin.y + line.y2 * sy),
                            ],
                            egui::Stroke::new(line.width * sx.min(sy), rgba(&line.color)),
                        );
                    }
                    CanvasCommand::Text(text) => {
                        ui.painter().text(
                            egui::pos2(origin.x + text.x * sx, origin.y + text.y * sy),
                            canvas_align(text.align),
                            &text.text,
                            egui::FontId::proportional(text.size * sx.min(sy)),
                            rgba(&text.color),
                        );
                    }
                }
            }
            out.canvas_time += canvas_started.elapsed();
        }

        UiNodeData::Divider => {
            ui.separator();
        }

        UiNodeData::Space(s) => {
            let amount = if s.grow {
                ui.available_height().max(0.0)
            } else {
                s.size
            };
            ui.add_space(amount);
        }

        // GPU surface: composite the guest's render (read back into an egui
        // texture by the live pane). Without one (e.g. no gpu grant), draw a
        // labelled placeholder so the layout still reserves the footprint.
        UiNodeData::Surface(s) => {
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(s.width as f32, s.height as f32),
                egui::Sense::hover(),
            );
            match surface {
                Some(tex) => {
                    ui.painter().image(
                        tex,
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                None => {
                    ui.painter()
                        .rect_filled(rect, style::RADIUS_MD, colors.bg_active);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "GPU surface",
                        egui::FontId::proportional(style::TEXT_CAPTION),
                        colors.text_dim,
                    );
                }
            }
        }

        UiNodeData::AppBar(a) => {
            AppChrome::new(colors).paint_app_bar(ui, &a.title, &a.subtitle);
        }

        UiNodeData::FooterKeys(f) => {
            AppChrome::new(colors).paint_footer_keys(ui, &f.entries, f.divider);
        }

        // Bottom-pinned nodes are partitioned and rendered by the enclosing
        // `Column` (see `column_bottom_pin`). When `Pinned` appears outside a
        // `Column` (e.g. at the tree root), render its child inline.
        UiNodeData::Pinned(p) => {
            render_node(ui, nodes, p.child, colors, out, depth + 1, surface, canvas_fits, pending_click, surface_key);
        }

        UiNodeData::Spinner(sp) => {
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                let t = ui.input(|i| i.time) as f32;
                ui.ctx().request_repaint();
                let angle = t * 4.0;
                let center = rect.center();
                for k in 0..8 {
                    let a = angle + k as f32 * std::f32::consts::TAU / 8.0;
                    let alpha = (k as f32 / 8.0 * 200.0) as u8 + 40;
                    let p = center + egui::vec2(a.cos(), a.sin()) * 6.0;
                    ui.painter().circle_filled(
                        p,
                        1.6,
                        colors.accent.linear_multiply(alpha as f32 / 255.0),
                    );
                }
                if !sp.label.is_empty() {
                    ui.add_space(style::SPACE_SM);
                    AppChrome::new(colors).text_label(
                        ui,
                        &sp.label,
                        style::TEXT_CAPTION,
                        colors.text_dim,
                        false,
                        false,
                        false,
                    );
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::wasm_app::bindings::plexi::platform::types::CanvasNode;
    use crate::host::wasm_app::{InputEvent, StateSnapshot, StateStore, SystemStats, WasmApp};
    use std::path::PathBuf;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/sysmon.wasm")
    }

    #[test]
    fn contained_canvas_preserves_source_aspect_ratio_and_centers_content() {
        let container = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 700.0));

        let (origin, sx, sy) =
            canvas_transform(container, 360.0, 440.0, CanvasFit::Contain);

        let expected_scale = 300.0 / 360.0;
        let expected_height = 440.0 * expected_scale;
        assert!((sx - expected_scale).abs() < f32::EPSILON);
        assert!((sy - expected_scale).abs() < f32::EPSILON);
        assert!((origin.x - container.left()).abs() < f32::EPSILON);
        assert!((origin.y - (container.top() + (700.0 - expected_height) / 2.0)).abs() < 0.001);
    }

    #[test]
    fn fill_canvas_uses_the_entire_allocated_rect() {
        let container = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 700.0));

        let (origin, sx, sy) = canvas_transform(container, 640.0, 360.0, CanvasFit::Fill);

        assert_eq!(origin, container.min);
        assert!((sx - 300.0 / 640.0).abs() < f32::EPSILON);
        assert!((sy - 700.0 / 360.0).abs() < f32::EPSILON);
    }

    // Stint 0390: a grow canvas's height must track the pane like its width
    // does, not stay floored at the app's declared height. Shrinking the pane
    // below the declared height previously left the canvas rect taller than
    // the pane, painting content below the visible edge.
    #[test]
    fn grow_canvas_height_tracks_pane_when_shrunk_below_declared_height() {
        let declared_height = 360.0;
        let pane_height = 100.0;

        let tree = UiTree {
            root: 0,
            nodes: vec![IndexedNode {
                id: 0,
                key: String::new(),
                data: UiNodeData::Canvas(CanvasNode {
                    width: 0.0,
                    height: declared_height,
                    grow: true,
                    commands: vec![],
                }),
            }],
        };

        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );

        let mut raw_input = egui::RawInput::default();
        raw_input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(200.0, pane_height),
        ));

        let mut allocated_height = 0.0f32;
        let _ = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let _ = render_ui_tree_with_surface(ui, &tree, &colors, None, None);
                    allocated_height = ui.min_rect().height();
                });
        });

        assert!(
            allocated_height <= pane_height + 1.0,
            "grow canvas allocated {allocated_height}px tall in a {pane_height}px pane, \
             declared height was {declared_height}px"
        );
    }

    // Stint 0397: a click at a known screen pixel inside a scaled `grow`
    // canvas must arrive as the correct canvas-space coordinate, not the raw
    // pixel. This is the transform-inversion regression guard — it exercises
    // the exact `canvas_transform` math the painter uses, inverted.
    #[test]
    fn canvas_click_inverts_fit_transform_to_canvas_space() {
        let declared_width = 360.0;
        let declared_height = 440.0;
        let pane_width = 200.0;
        let pane_height = 400.0;

        let tree = UiTree {
            root: 0,
            nodes: vec![IndexedNode {
                id: 0,
                key: String::new(),
                data: UiNodeData::Canvas(CanvasNode {
                    width: declared_width,
                    height: declared_height,
                    grow: true,
                    commands: vec![],
                }),
            }],
        };
        let mut fits = HashMap::new();
        fits.insert(0, CanvasFit::Contain);

        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );

        // sx = sy = 200/360 = 5/9; origin.y = 200 - (440 * 5/9)/2 = 700/9.
        // A click at screen pixel (50, 100) inverts to canvas space (90, 40).
        let click_px = egui::pos2(50.0, 100.0);
        let screen_rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(pane_width, pane_height));

        // Warm-up frame with no input: egui hit-tests a press/release pair
        // against the *previous* frame's widget rects, so the canvas must
        // already have been laid out once before a click can resolve.
        let warmup = egui::RawInput {
            screen_rect: Some(screen_rect),
            ..Default::default()
        };
        let _ = ctx.run(warmup, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    let _ =
                        render_ui_tree_with_canvas_fits(ui, &tree, &colors, None, Some(&fits), None, None);
                });
        });

        // Move + press + release delivered together as one frame's input,
        // mirroring the click-simulation pattern proven in `src/ui_tests.rs`
        // (`PlexiUiHarness` sidebar tests).
        let click_frame = egui::RawInput {
            screen_rect: Some(screen_rect),
            events: vec![
                egui::Event::PointerMoved(click_px),
                egui::Event::PointerButton {
                    pos: click_px,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::PointerButton {
                    pos: click_px,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..Default::default()
        };
        let mut captured = RenderResult::default();
        let _ = ctx.run(click_frame, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    captured =
                        render_ui_tree_with_canvas_fits(ui, &tree, &colors, None, Some(&fits), None, None);
                });
        });

        assert_eq!(
            captured.canvas_clicks.len(),
            1,
            "one click at a known pixel should produce exactly one canvas click"
        );
        let click = captured.canvas_clicks[0];
        assert!(
            (click.x - 90.0).abs() < 0.01,
            "expected canvas-space x 90.0, got {}",
            click.x
        );
        assert!(
            (click.y - 40.0).abs() < 0.01,
            "expected canvas-space y 40.0, got {}",
            click.y
        );
        assert_eq!(click.button, Some("left"));
        assert!(click.pressed);
    }

    // G4 foundation: a real sysmon view tree (after delivering stats) renders
    // through the egui pipeline headlessly without panicking, and produces no
    // spurious actions.
    #[test]
    fn renders_sysmon_tree_headless() -> wasmtime::Result<()> {
        let mut app =
            WasmApp::load_ephemeral_run("sysmon-render", &fixture(), StateStore::ephemeral())?;
        app.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0), &[])?;
        app.update(&InputEvent::SystemStatsResult(SystemStats {
            cpu_usage_pct: 42.0,
            memory_used_bytes: 8u64 << 30,
            memory_total_bytes: 16u64 << 30,
            disk_read_bps: 0,
            disk_write_bps: 0,
            net_rx_bps: 0,
            net_tx_bps: 0,
            uptime_secs: 0,
            load_avg_one_min: 0.0,
        }))?;
        let tree = app.view()?;

        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );

        let mut captured = RenderResult::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                captured = render_ui_tree_with_surface(ui, &tree, &colors, None, None);
            });
        });

        assert!(
            captured.actions.is_empty(),
            "sysmon view declares no actions"
        );
        Ok(())
    }
}
