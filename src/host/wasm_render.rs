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

use super::wasm_app::bindings::plexi::platform::types::CanvasCommand;
use super::wasm_app::{Alignment, BadgeColor, ButtonStyle, Color, IndexedNode, UiNodeData, UiTree};

const MAX_DEPTH: u32 = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanvasFit {
    #[default]
    Fill,
    Contain,
}

/// Interactions produced by one render pass, to be translated into guest input.
#[derive(Default, Debug)]
pub struct RenderResult {
    /// Action strings from clicked buttons, selected list rows, or submitted inputs.
    pub actions: Vec<String>,
    /// `(on_change action, new value)` pairs from edited text inputs.
    pub value_changes: Vec<(String, String)>,
    pub canvas_time: std::time::Duration,
}

/// Render a view tree, compositing `surface` into the first surface-node
/// (the guest's GPU output). Pass `None` to draw a placeholder instead.
pub fn render_ui_tree_with_surface(
    ui: &mut egui::Ui,
    tree: &UiTree,
    colors: &Colors,
    surface: Option<egui::TextureId>,
) -> RenderResult {
    render_ui_tree_with_canvas_fits(ui, tree, colors, surface, None)
}

pub fn render_ui_tree_with_canvas_fits(
    ui: &mut egui::Ui,
    tree: &UiTree,
    colors: &Colors,
    surface: Option<egui::TextureId>,
    canvas_fits: Option<&HashMap<u32, CanvasFit>>,
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
    );
    out
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

fn render_node(
    ui: &mut egui::Ui,
    nodes: &[IndexedNode],
    id: u32,
    colors: &Colors,
    out: &mut RenderResult,
    depth: u32,
    surface: Option<egui::TextureId>,
    canvas_fits: Option<&HashMap<u32, CanvasFit>>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(node) = nodes
        .get(id as usize)
        .filter(|node| node.id == id)
        .or_else(|| nodes.iter().find(|node| node.id == id))
    else {
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
            if ui.add_enabled(!b.disabled, btn).clicked() {
                out.actions.push(b.on_click.clone());
            }
        }

        UiNodeData::TextInput(ti) => {
            let mut buf = ti.value.clone();
            let edit = egui::TextEdit::singleline(&mut buf)
                .password(ti.password)
                .hint_text(&ti.placeholder);
            let resp = ui.add(edit);
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
                    render_node(ui, nodes, *child, colors, out, depth + 1, surface, canvas_fits);
                }
            });
        }

        UiNodeData::Column(c) => {
            ui.with_layout(egui::Layout::top_down(cross_align(c.align)), |ui| {
                ui.spacing_mut().item_spacing.y = c.gap;
                for child in &c.children {
                    render_node(ui, nodes, *child, colors, out, depth + 1, surface, canvas_fits);
                }
            });
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
                        render_node(ui, nodes, *item_id, colors, out, depth + 1, surface, canvas_fits);
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.clicked() {
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
                render_node(ui, nodes, s.child, colors, out, depth + 1, surface, canvas_fits);
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
                    render_node(ui, nodes, p.child, colors, out, depth + 1, surface, canvas_fits);
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
                ui.available_height().max(c.height)
            } else {
                c.height
            };
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(width, height.max(1.0)), egui::Sense::hover());
            let fit = canvas_fits
                .and_then(|fits| fits.get(&id))
                .copied()
                .unwrap_or_default();
            let (origin, sx, sy) = canvas_transform(rect, c.width, c.height, fit);
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

        UiNodeData::Space(px) => {
            ui.add_space(*px);
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
                captured = render_ui_tree_with_surface(ui, &tree, &colors, None);
            });
        });

        assert!(
            captured.actions.is_empty(),
            "sysmon view declares no actions"
        );
        Ok(())
    }
}
