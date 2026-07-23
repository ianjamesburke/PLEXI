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

use super::wasm_app::bindings::plexi::platform::types::{
    CanvasCommand, FooterKeysNode, PinnedEdge,
};
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
    let render_root = |ui: &mut egui::Ui, out: &mut RenderResult| {
        render_node(
            ui,
            &tree.nodes,
            tree.root,
            colors,
            out,
            0,
            surface,
            canvas_fits,
            pending_click,
            surface_key,
        );
    };
    // Good-by-default spacing (stint 0445): a declarative app with no layout
    // code should not render flush against the pane edge. Wrap the whole tree
    // in the host UI kit's standard content inset so body content, app bars,
    // and footers all get comfortable breathing room. Full-bleed pixel apps (a
    // grow Canvas or a GPU Surface anywhere in the tree) own their own layout
    // and are exempt.
    if tree_wants_content_padding(&tree.nodes) {
        // Stint 0448: the inset must move only the *content* cursor, never the
        // app's visible surface. Paint the app-surface fill (`theme.bg`, the
        // color apps are told their container is) across the entire pane rect
        // first, then inset the layout. Without this the surface would start
        // inside the inset and the darker pane gutter would show through as a
        // black border around every flow app. AppBar/FooterKeys still paint
        // their own bands full-bleed via `clip_rect`, on top of this surface.
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, colors.terminal_bg);
        egui::Frame::NONE
            .inner_margin(root_content_inset())
            .show(ui, |ui| render_root(ui, &mut out));
    } else {
        render_root(ui, &mut out);
    }
    out
}

/// The host UI kit's standard content inset for a declarative app's root.
/// `SPACE_XL` horizontal matches the breathing room modal bodies get; the
/// slightly tighter `SPACE_MD` top/bottom keeps short apps from feeling
/// bottom-heavy. All values come from `crate::ui::style` design tokens.
fn root_content_inset() -> egui::Margin {
    egui::Margin {
        left: style::SPACE_XL as i8,
        right: style::SPACE_XL as i8,
        top: style::SPACE_MD as i8,
        bottom: style::SPACE_MD as i8,
    }
}

/// A tree earns the default content inset unless it is a full-bleed pixel app:
/// a grow `Canvas` (games, visualizers) or a GPU `Surface` (video/3D output)
/// anywhere in the tree signals the app owns every pixel, so the host must not
/// inset it. A fixed-size `Canvas` is treated as ordinary flow content.
fn tree_wants_content_padding(nodes: &[IndexedNode]) -> bool {
    !nodes.iter().any(|n| match &n.data {
        UiNodeData::Canvas(c) => c.grow,
        UiNodeData::Surface(_) => true,
        _ => false,
    })
}

/// Headless render of a `UiTree` to PNG bytes via `egui_kittest`'s wgpu
/// offscreen backend — the same rasterization path `PlexiUiHarness` uses for
/// screenshot tests, reused here for `plexi app render --png` /
/// `plexi app check --png-dir` so the CLI renders exactly what the live host
/// would paint instead of hand-rolling a second rasterizer for widget chrome
/// (buttons, fonts, footer key chips) that has no flat-primitive form.
/// `pixels_per_point` is explicit: the CLI renders at 1.0; HiDPI screenshot
/// tests render at 2.0 so surface-resolution and pixel-grid regressions are
/// visible in the captured pixels.
pub fn render_ui_tree_to_png(
    tree: &UiTree,
    width: f32,
    height: f32,
    pixels_per_point: f32,
) -> Result<Vec<u8>, String> {
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
        .with_pixels_per_point(pixels_per_point)
        .build_ui(move |ui| {
            if !fonts_ready {
                crate::ui::theme::setup_fonts(ui.ctx());
                fonts_ready = true;
                ui.ctx().request_repaint();
                return;
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    let _ = render_ui_tree_with_surface(ui, &tree, &colors, None, None);
                });
        });
    harness.run();
    let img = harness
        .render()
        .map_err(|e| format!("offscreen render failed: {e}"))?;
    let mut bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )
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
        Some(crate::host::pane::PaneClickTarget::Node(n)) if n == u64::from(id)
    )
}

/// Resolve a `TextInput`'s edit buffer for this frame (stint 0456).
/// `stored` is `(buffer, last_app_value)` from egui temp memory; returns the
/// pair to edit this frame. The app's reported `value` wins only when it
/// *changed* since we last saw it (a reset after submit, an external
/// `SetState`) — an unchanged value (or the echo of our own `on_change`,
/// which the caller records into `last_app_value`) never clobbers a local
/// draft still round-tripping to the guest.
fn reconcile_text_input_buffer(
    stored: Option<(String, String)>,
    app_value: &str,
) -> (String, String) {
    match stored {
        Some((buf, last_app_value)) if app_value == last_app_value => (buf, last_app_value),
        _ => (app_value.to_string(), app_value.to_string()),
    }
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
                .corner_radius(style::RADIUS_SM)
                // Comfortable minimum click/touch target so single-glyph
                // buttons (calculator keys, toolbar chips) aren't cramped.
                // Content-sized labels grow past this floor as usual.
                .min_size(egui::vec2(style::BUTTON_H_MD, style::BUTTON_H_MD));
            let synthetic_click = !b.disabled && node_click_matches(pending_click, id);
            if ui.add_enabled(!b.disabled, btn).clicked() || synthetic_click {
                out.actions.push(b.on_click.clone());
            }
        }

        UiNodeData::TextInput(ti) => {
            // Host-owned edit buffer (stint 0456): the app's `value` is a
            // controlled input that round-trips asynchronously (on_change →
            // guest update → SetState → next tree), so painting `ti.value`
            // directly would clobber keystrokes typed while the echo is in
            // flight. The buffer lives in egui temp memory keyed by the
            // widget id; an app-pushed value change (reset after submit,
            // external SetState) still wins over any local draft.
            let widget_id = ui.id().with(("l1_text_input", id));
            let state_id = widget_id.with("edit_state");
            let stored: Option<(String, String)> =
                ui.ctx().memory_mut(|m| m.data.get_temp(state_id));
            let (mut buf, mut last_app_value) = reconcile_text_input_buffer(stored, &ti.value);
            let resp = crate::ui::text_field::TextField::singleline(widget_id, &ti.placeholder)
                .password(ti.password)
                .log_name("l1_text_input")
                .show(ui, &mut buf, colors);
            if let Some(key) = surface_key {
                crate::ui::focus::register_text_surface(ui.ctx(), key, resp.id);
                // Node-targeted clicks focus the field; once focused, the
                // dispatch gate (`focused_pane_text_surface`, stint 0456)
                // routes keystrokes here instead of the app's KeyEvent path.
                // The claim routes through the reconciler (stint 0429), which
                // grants it while this pane owns input.
                if node_click_matches(pending_click, id) {
                    crate::ui::focus::claim_text_surface(ui.ctx(), key, resp.id);
                }
            }
            if resp.changed() {
                out.value_changes.push((ti.on_change.clone(), buf.clone()));
                // Treat our own change as already-seen so the app's echo
                // doesn't reset the buffer mid-typing.
                last_app_value = buf.clone();
            }
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                out.actions.push(ti.on_submit.clone());
                // Keep focus for consecutive entries — Enter submits, it
                // doesn't dismiss the field. Escape leaves the field via
                // egui's native focus surrender in `begin_pass`; the
                // dispatch gate swallows that same-frame Escape so it
                // can't fire the AppActive CloseApp binding.
                if let Some(key) = surface_key {
                    crate::ui::focus::claim_text_surface(ui.ctx(), key, resp.id);
                }
            }
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(state_id, (buf, last_app_value)));
        }

        UiNodeData::Row(r) => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = r.gap;
                for child in &r.children {
                    render_node(
                        ui,
                        nodes,
                        *child,
                        colors,
                        out,
                        depth + 1,
                        surface,
                        canvas_fits,
                        pending_click,
                        surface_key,
                    );
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
                let body_rect = egui::Rect::from_min_size(
                    stack_rect.min,
                    egui::vec2(stack_rect.width(), body_h),
                );

                ui.scope_builder(egui::UiBuilder::new().max_rect(body_rect), |ui| {
                    ui.set_clip_rect(body_rect);
                    ui.set_min_height(body_h);
                    ui.set_max_height(body_h);
                    ui.with_layout(egui::Layout::top_down(cross_align(c.align)), |ui| {
                        ui.spacing_mut().item_spacing.y = c.gap;
                        for &child in &c.children[..c.children.len() - 1] {
                            render_node(
                                ui,
                                nodes,
                                child,
                                colors,
                                out,
                                depth + 1,
                                surface,
                                canvas_fits,
                                pending_click,
                                surface_key,
                            );
                        }
                    });
                });

                let footer_rect = egui::Rect::from_min_size(
                    egui::pos2(stack_rect.min.x, stack_rect.max.y - footer_h),
                    egui::vec2(stack_rect.width(), footer_h),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(footer_rect), |ui| {
                    ui.set_clip_rect(footer_rect);
                    ui.set_min_height(footer_h);
                    ui.set_max_height(footer_h);
                    render_node(
                        ui,
                        nodes,
                        footer_id,
                        colors,
                        out,
                        depth + 1,
                        surface,
                        canvas_fits,
                        pending_click,
                        surface_key,
                    );
                });
            } else {
                ui.with_layout(egui::Layout::top_down(cross_align(c.align)), |ui| {
                    ui.spacing_mut().item_spacing.y = c.gap;
                    for child in &c.children {
                        render_node(
                            ui,
                            nodes,
                            *child,
                            colors,
                            out,
                            depth + 1,
                            surface,
                            canvas_fits,
                            pending_click,
                            surface_key,
                        );
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
                        render_node(
                            ui,
                            nodes,
                            *item_id,
                            colors,
                            out,
                            depth + 1,
                            surface,
                            canvas_fits,
                            pending_click,
                            surface_key,
                        );
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
                render_node(
                    ui,
                    nodes,
                    s.child,
                    colors,
                    out,
                    depth + 1,
                    surface,
                    canvas_fits,
                    pending_click,
                    surface_key,
                );
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
                    render_node(
                        ui,
                        nodes,
                        p.child,
                        colors,
                        out,
                        depth + 1,
                        surface,
                        canvas_fits,
                        pending_click,
                        surface_key,
                    );
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
            let (rect, resp) = ui.allocate_exact_size(
                egui::vec2(width, height.max(1.0)),
                egui::Sense::click_and_drag(),
            );
            let fit = canvas_fits
                .and_then(|fits| fits.get(&id))
                .copied()
                .unwrap_or_default();
            let (origin, sx, sy) = canvas_transform(rect, c.width, c.height, fit);
            let to_canvas = |pos: egui::Pos2, button: Option<&'static str>, pressed: bool| {
                CanvasClick {
                    x: (pos.x - origin.x) / sx,
                    y: (pos.y - origin.y) / sy,
                    button,
                    pressed,
                }
            };
            // A real click is detected by egui's own `Sense` resolution
            // (resolved once per pass, inside `Context::begin_pass`, from that
            // pass's actual `RawInput` — it cannot be faked by mutating
            // `ctx.input_mut()` after the pass has started). `plexi pane
            // click`/`HostHarness::inject_click` deliver a `PendingPaneClick`
            // instead, matched against this frame's freshly-computed `rect` —
            // the same honest hit-test a real click would need, just resolved
            // explicitly rather than via egui's internal interact_widgets.
            // Drag samples (stint 0510) arrive the same way, one per frame,
            // and map to press/move/release mouse events in canvas space.
            let synthetic = pending_click.filter(|c| {
                matches!(c.target, crate::host::pane::PaneClickTarget::Pos(pos) if rect.contains(pos))
            });
            if let Some(c) = synthetic {
                if let crate::host::pane::PaneClickTarget::Pos(pos) = c.target {
                    use crate::host::pane::PointerPhase;
                    out.canvas_clicks.push(match c.phase {
                        // `Click` keeps the pre-drag contract: one
                        // pressed=true event, no synthetic release.
                        PointerPhase::Click | PointerPhase::Press => {
                            to_canvas(pos, Some(c.button), true)
                        }
                        PointerPhase::Move => to_canvas(pos, None, false),
                        PointerPhase::Release => to_canvas(pos, Some(c.button), false),
                    });
                }
            }
            // Real pointer interactions, through egui's own hit-testing. A
            // plain click stays a single pressed=true event (the existing app
            // contract); a genuine drag delivers press → move… → release so
            // scrub/trim interactions work with a physical mouse too.
            let real_button = [
                (egui::PointerButton::Primary, "left"),
                (egui::PointerButton::Secondary, "right"),
                (egui::PointerButton::Middle, "middle"),
            ];
            if let Some(pos) = resp.interact_pointer_pos() {
                if resp.clicked() {
                    let button = real_button
                        .iter()
                        .find(|(b, _)| resp.clicked_by(*b))
                        .map_or("left", |(_, name)| *name);
                    out.canvas_clicks.push(to_canvas(pos, Some(button), true));
                } else if let Some((_, button)) = real_button
                    .iter()
                    .find(|(b, _)| resp.drag_started_by(*b))
                {
                    out.canvas_clicks.push(to_canvas(pos, Some(button), true));
                } else if let Some((_, button)) = real_button
                    .iter()
                    .find(|(b, _)| resp.drag_stopped_by(*b))
                {
                    out.canvas_clicks.push(to_canvas(pos, Some(button), false));
                } else if resp.dragged() {
                    out.canvas_clicks.push(to_canvas(pos, None, false));
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
                        // Rasterize at the final effective size snapped to the
                        // physical pixel grid (stint 0527): a raw
                        // `size × scale` yields fractional per-frame font
                        // sizes that re-rasterize soft at every canvas resize.
                        let ppp = ui.painter().pixels_per_point();
                        let effective = (text.size * sx.min(sy)).max(1.0);
                        let snapped = (effective * ppp).round().max(1.0) / ppp;
                        ui.painter().text(
                            egui::pos2(origin.x + text.x * sx, origin.y + text.y * sy),
                            canvas_align(text.align),
                            &text.text,
                            egui::FontId::proportional(snapped),
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
            // The backing texture is allocated at `logical × ppp` physical
            // pixels (stint 0527); snapping the composite rect to the pixel
            // grid makes the texel→pixel mapping the identity at integer ppp,
            // so no resampling blurs the guest's render.
            let rect = {
                use egui::emath::GuiRounding;
                rect.round_to_pixels(ui.painter().pixels_per_point())
            };
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
                    crate::ui::snap::text_snapped(
                        ui.painter(),
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
            render_node(
                ui,
                nodes,
                p.child,
                colors,
                out,
                depth + 1,
                surface,
                canvas_fits,
                pending_click,
                surface_key,
            );
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
    use crate::host::wasm_app::bindings::plexi::platform::types::{
        AppBarNode, ButtonNode, CanvasNode, CanvasRect, CanvasText, ColumnNode, SurfaceNode,
        TextInputNode, TextNode,
    };
    use crate::host::wasm_app::{InputEvent, StateSnapshot, StateStore, SystemStats, WasmApp};

    /// Stint 0456: the declarative TextInput renders through the styled
    /// host field (`crate::ui::text_field`) — bg_active fill, border
    /// stroke, dimmed hint — not egui's default TextEdit chrome. Visual
    /// review artifact: /tmp/plexi-render-0456-textinput.png.
    #[test]
    fn screenshot_declarative_text_input_styled_field() {
        let tree = UiTree {
            root: 0,
            nodes: vec![
                node(
                    0,
                    UiNodeData::Column(ColumnNode {
                        children: vec![1, 2],
                        gap: 8.0,
                        align: Alignment::Start,
                        grow: true,
                    }),
                ),
                node(
                    1,
                    UiNodeData::Text(TextNode {
                        text: "I'm thinking of a number between 1 and 100.".to_string(),
                        size: None,
                        bold: false,
                        color: None,
                        truncate: false,
                        align: Alignment::Start,
                    }),
                ),
                node(
                    2,
                    UiNodeData::TextInput(TextInputNode {
                        value: String::new(),
                        placeholder: "Enter your guess (1-100)".to_string(),
                        on_change: "guess".to_string(),
                        on_submit: "guess".to_string(),
                        password: false,
                    }),
                ),
            ],
        };
        let png = render_ui_tree_to_png(&tree, 420.0, 180.0, 1.0).expect("render TextInput tree");
        std::fs::write("/tmp/plexi-render-0456-textinput.png", png)
            .expect("write screenshot for visual review");
    }

    /// Stints 0527/0528/0530 evidence: a text-heavy app (AppBar with
    /// title+subtitle, flow text, canvas text commands) rendered at ppp 1.0
    /// and 2.0. The ppp-2.0 capture must show 2× pixel detail — crisp canvas
    /// text at the snapped effective size, pixel-grid AppBar galleys, and the
    /// contrast-floored subtitle tone. Review artifacts:
    /// /tmp/plexi-render-0527-app-text-ppp{1,2}.png.
    #[test]
    fn screenshot_text_heavy_app_at_ppp_1_and_2() {
        let white = Color {
            r: 0xe6,
            g: 0xe6,
            b: 0xf0,
            a: 0xff,
        };
        let canvas_text = |x: f32, y: f32, size: f32, text: &str| {
            CanvasCommand::Text(CanvasText {
                x,
                y,
                text: text.to_string(),
                size,
                color: white,
                bold: false,
                align: Alignment::Start,
            })
        };
        let tree = UiTree {
            root: 0,
            nodes: vec![
                node(
                    0,
                    UiNodeData::Column(ColumnNode {
                        children: vec![1, 2, 3],
                        gap: 8.0,
                        align: Alignment::Start,
                        grow: true,
                    }),
                ),
                node(
                    1,
                    UiNodeData::AppBar(AppBarNode {
                        title: "Pixel Grid Fixture".to_string(),
                        subtitle: "surface resolution · typography evidence".to_string(),
                    }),
                ),
                node(
                    2,
                    UiNodeData::Text(TextNode {
                        text: "The quick brown fox jumps over the lazy dog 0123456789"
                            .to_string(),
                        size: None,
                        bold: false,
                        color: None,
                        truncate: false,
                        align: Alignment::Start,
                    }),
                ),
                node(
                    3,
                    UiNodeData::Canvas(CanvasNode {
                        width: 420.0,
                        height: 160.0,
                        grow: false,
                        commands: vec![
                            canvas_text(8.0, 12.0, 11.0, "canvas 11pt: waveform ruler 0 dB"),
                            canvas_text(8.0, 40.0, 14.0, "canvas 14pt: The quick brown fox"),
                            canvas_text(8.0, 74.0, 18.0, "canvas 18pt: 120.0 BPM 44.1 kHz"),
                        ],
                    }),
                ),
            ],
        };
        for (ppp, path) in [
            (1.0, "/tmp/plexi-render-0527-app-text-ppp1.png"),
            (2.0, "/tmp/plexi-render-0527-app-text-ppp2.png"),
        ] {
            let png = render_ui_tree_to_png(&tree, 480.0, 300.0, ppp)
                .unwrap_or_else(|e| panic!("render text-heavy tree at ppp {ppp}: {e}"));
            std::fs::write(path, png).expect("write screenshot for visual review");
        }
    }

    /// Stint 0456: the TextInput edit buffer only resets when the app
    /// *pushes a different value* — our own echo and unchanged app frames
    /// never clobber a local draft mid-round-trip.
    #[test]
    fn text_input_buffer_reconciliation() {
        // Fresh widget: adopt the app's value.
        assert_eq!(
            reconcile_text_input_buffer(None, "seed"),
            ("seed".to_string(), "seed".to_string())
        );

        // Local draft survives frames where the app still reports the value
        // we last saw (the typed change is still round-tripping).
        assert_eq!(
            reconcile_text_input_buffer(Some(("typed".to_string(), "".to_string())), ""),
            ("typed".to_string(), "".to_string()),
            "stale app value must not clobber the draft"
        );

        // The caller records our own change into last_app_value, so the
        // app's echo of it is a no-op.
        assert_eq!(
            reconcile_text_input_buffer(Some(("typed".to_string(), "typed".to_string())), "typed"),
            ("typed".to_string(), "typed".to_string())
        );

        // An app-pushed change (reset after submit, external SetState) wins
        // over any local draft.
        assert_eq!(
            reconcile_text_input_buffer(Some(("typed".to_string(), "typed".to_string())), ""),
            ("".to_string(), "".to_string()),
            "app reset must clear the draft"
        );
    }
    use std::path::PathBuf;

    fn node(id: u32, data: UiNodeData) -> IndexedNode {
        IndexedNode {
            id,
            key: String::new(),
            data,
        }
    }

    /// Bounding box of everything painted when `tree` renders into a `pane`-sized
    /// screen. Used to observe where the good-by-default content inset places
    /// real content vs. where a full-bleed app draws.
    fn painted_bounds(tree: &UiTree, pane: egui::Vec2) -> egui::Rect {
        let mut bounds = egui::Rect::NOTHING;
        for r in painted_shapes(tree, pane) {
            bounds = bounds.union(r);
        }
        bounds
    }

    /// Every finite, positive shape bounding rect painted by `tree` into a
    /// `pane`-sized screen (the host panel is `Frame::NONE`, so shapes are
    /// exactly what the tree render draws).
    fn painted_shapes(tree: &UiTree, pane: egui::Vec2) -> Vec<egui::Rect> {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let colors = Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, pane)),
            ..Default::default()
        };
        let output = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    let _ = render_ui_tree_with_surface(ui, tree, &colors, None, None);
                });
        });
        output
            .shapes
            .into_iter()
            .map(|clipped| clipped.shape.visual_bounding_rect())
            .filter(|r| r.is_finite() && r.is_positive())
            .collect()
    }

    /// Split painted shapes into the pane-filling background surface (any shape
    /// that covers the whole `pane` rect) and the union of everything else
    /// (the actual content). Used to prove the surface fills the pane while
    /// content stays inset.
    fn surface_and_content(tree: &UiTree, pane: egui::Vec2) -> (Option<egui::Rect>, egui::Rect) {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, pane);
        let mut surface: Option<egui::Rect> = None;
        let mut content = egui::Rect::NOTHING;
        for r in painted_shapes(tree, pane) {
            let fills_pane = r.min.x <= screen.min.x + 0.5
                && r.min.y <= screen.min.y + 0.5
                && r.max.x >= screen.max.x - 0.5
                && r.max.y >= screen.max.y - 0.5;
            if fills_pane {
                surface = Some(surface.map_or(r, |s| s.union(r)));
            } else {
                content = content.union(r);
            }
        }
        (surface, content)
    }

    // Stint 0445: a declarative flow app (no grow Canvas / Surface) earns the
    // host's standard content inset, so its content never renders flush against
    // the pane edge, and buttons get a comfortable minimum click target.
    #[test]
    fn flow_root_is_inset_and_buttons_meet_minimum_size() {
        let tree = UiTree {
            root: 0,
            nodes: vec![node(
                0,
                UiNodeData::Button(ButtonNode {
                    label: "X".to_string(),
                    on_click: "x".to_string(),
                    style: ButtonStyle::Primary,
                    disabled: false,
                }),
            )],
        };
        let pane = egui::vec2(400.0, 300.0);
        let (surface, content) = surface_and_content(&tree, pane);
        // Stint 0448: the app surface fills the whole pane; only content insets.
        let surface = surface.expect("flow app must paint a pane-filling surface");
        assert!(
            surface.min.x <= 0.5
                && surface.min.y <= 0.5
                && surface.max.x >= pane.x - 0.5
                && surface.max.y >= pane.y - 0.5,
            "flow surface {surface:?} should fill the pane {pane:?}"
        );
        assert!(
            content.min.x >= style::SPACE_XL - 0.5,
            "flow content left edge {} should be inset by ~SPACE_XL ({})",
            content.min.x,
            style::SPACE_XL
        );
        assert!(
            content.min.y >= style::SPACE_MD - 0.5,
            "flow content top edge {} should be inset by ~SPACE_MD ({})",
            content.min.y,
            style::SPACE_MD
        );
        assert!(
            content.height() >= style::BUTTON_H_MD - 0.5,
            "button height {} should meet the minimum target ({})",
            content.height(),
            style::BUTTON_H_MD
        );
    }

    // Stint 0448: a flow app's surface/background fill must span the entire
    // pane rect — the content inset moves only the layout cursor, never the
    // visible surface. Regression guard against the 0445 black-border bug.
    #[test]
    fn flow_root_background_fills_the_pane() {
        let tree = UiTree {
            root: 0,
            nodes: vec![
                node(
                    0,
                    UiNodeData::Column(ColumnNode {
                        children: vec![1],
                        gap: 0.0,
                        align: Alignment::Start,
                        grow: true,
                    }),
                ),
                node(
                    1,
                    UiNodeData::Text(TextNode {
                        text: "hello".to_string(),
                        size: None,
                        bold: false,
                        color: None,
                        truncate: false,
                        align: Alignment::Start,
                    }),
                ),
            ],
        };
        let pane = egui::vec2(400.0, 300.0);
        let (surface, content) = surface_and_content(&tree, pane);
        let surface = surface.expect("flow app must paint a pane-filling surface");
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, pane);
        assert!(
            surface.min.x <= 0.5
                && surface.min.y <= 0.5
                && surface.max.x >= pane.x - 0.5
                && surface.max.y >= pane.y - 0.5,
            "surface {surface:?} must equal the full pane rect {screen:?}"
        );
        // Content still lives inside the inset.
        assert!(
            content.min.x >= style::SPACE_XL - 0.5 && content.min.y >= style::SPACE_MD - 0.5,
            "content {content:?} must stay inset while the surface fills the pane"
        );
    }

    // A grow Canvas signals a full-bleed pixel app; the host must not inset it,
    // so its drawing reaches the pane edge.
    #[test]
    fn grow_canvas_app_is_not_inset() {
        let fill = Color {
            r: 255,
            g: 0,
            b: 0,
            a: 255,
        };
        let tree = UiTree {
            root: 0,
            nodes: vec![node(
                0,
                UiNodeData::Canvas(CanvasNode {
                    width: 100.0,
                    height: 100.0,
                    grow: true,
                    commands: vec![CanvasCommand::Rect(CanvasRect {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 100.0,
                        radius: 0.0,
                        fill,
                    })],
                }),
            )],
        };
        let bounds = painted_bounds(&tree, egui::vec2(400.0, 300.0));
        assert!(
            bounds.min.x <= 1.0 && bounds.min.y <= 1.0,
            "full-bleed canvas should reach the pane origin, got {:?}",
            bounds.min
        );
    }

    #[test]
    fn content_padding_exemption_tracks_full_bleed_nodes() {
        let flow = vec![
            node(
                0,
                UiNodeData::Column(ColumnNode {
                    children: vec![1],
                    gap: 0.0,
                    align: Alignment::Start,
                    grow: false,
                }),
            ),
            node(
                1,
                UiNodeData::Text(TextNode {
                    text: "hi".to_string(),
                    size: None,
                    bold: false,
                    color: None,
                    truncate: false,
                    align: Alignment::Start,
                }),
            ),
        ];
        assert!(
            tree_wants_content_padding(&flow),
            "a text/column tree is flow content and wants the inset"
        );

        let mut with_grow_canvas = flow.clone();
        with_grow_canvas[1] = node(
            1,
            UiNodeData::Canvas(CanvasNode {
                width: 10.0,
                height: 10.0,
                grow: true,
                commands: vec![],
            }),
        );
        assert!(
            !tree_wants_content_padding(&with_grow_canvas),
            "a grow canvas makes the app full-bleed"
        );

        let mut with_fixed_canvas = flow.clone();
        with_fixed_canvas[1] = node(
            1,
            UiNodeData::Canvas(CanvasNode {
                width: 10.0,
                height: 10.0,
                grow: false,
                commands: vec![],
            }),
        );
        assert!(
            tree_wants_content_padding(&with_fixed_canvas),
            "a fixed-size canvas is ordinary flow content and still wants the inset"
        );

        let mut with_surface = flow.clone();
        with_surface[1] = node(
            1,
            UiNodeData::Surface(SurfaceNode {
                width: 8,
                height: 8,
                texture_handle: None,
            }),
        );
        assert!(
            !tree_wants_content_padding(&with_surface),
            "a GPU surface makes the app full-bleed"
        );
    }

    #[test]
    fn root_content_inset_uses_design_tokens() {
        let inset = root_content_inset();
        assert_eq!(inset.left, style::SPACE_XL as i8);
        assert_eq!(inset.right, style::SPACE_XL as i8);
        assert_eq!(inset.top, style::SPACE_MD as i8);
        assert_eq!(inset.bottom, style::SPACE_MD as i8);
    }

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/sysmon.wasm")
    }

    #[test]
    fn contained_canvas_preserves_source_aspect_ratio_and_centers_content() {
        let container = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 700.0));

        let (origin, sx, sy) = canvas_transform(container, 360.0, 440.0, CanvasFit::Contain);

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
        let _ = ctx.run_ui(raw_input, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
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
        let _ = ctx.run_ui(warmup, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    let _ = render_ui_tree_with_canvas_fits(
                        ui,
                        &tree,
                        &colors,
                        None,
                        Some(&fits),
                        None,
                        None,
                    );
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
        let _ = ctx.run_ui(click_frame, |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    captured = render_ui_tree_with_canvas_fits(
                        ui,
                        &tree,
                        &colors,
                        None,
                        Some(&fits),
                        None,
                        None,
                    );
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
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show_inside(ui, |ui| {
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
