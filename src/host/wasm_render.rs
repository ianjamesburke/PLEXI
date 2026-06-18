// Scene-graph renderer — maps a WASM app's arena `UiTree` onto egui widgets.
//
// The tree is a flat list of `IndexedNode`s referenced by u32 id (the WIT model
// avoids recursive types). Rendering walks from `tree.root`, looking children
// up by id. Interactive nodes (buttons, list rows, submitted inputs) collect
// their declared action strings into `RenderResult` so the pane can route them
// back to the guest. A depth cap guards against malformed/cyclic trees.

use std::collections::HashMap;

use egui::{Color32, RichText};

use crate::ui::style;
use crate::ui::theme::Colors;

use super::wasm_app::{Alignment, BadgeColor, ButtonStyle, Color, IndexedNode, UiNodeData, UiTree};

const MAX_DEPTH: u32 = 256;

/// Interactions produced by one render pass, to be translated into guest input.
#[derive(Default, Debug)]
pub struct RenderResult {
    /// Action strings from clicked buttons, selected list rows, or submitted inputs.
    pub actions: Vec<String>,
    /// `(on_change action, new value)` pairs from edited text inputs.
    pub value_changes: Vec<(String, String)>,
}

/// Render a view tree, compositing `surface_tex` into the first surface-node
/// (the guest's GPU output). Pass `None` to draw a placeholder instead.
pub fn render_ui_tree_with_surface(
    ui: &mut egui::Ui,
    tree: &UiTree,
    colors: &Colors,
    surface_tex: Option<&egui::TextureHandle>,
) -> RenderResult {
    let index: HashMap<u32, &IndexedNode> = tree.nodes.iter().map(|n| (n.id, n)).collect();
    let mut out = RenderResult::default();
    render_node(ui, &index, tree.root, colors, &mut out, 0, surface_tex);
    out
}

fn rgba(c: &Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
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
    index: &HashMap<u32, &IndexedNode>,
    id: u32,
    colors: &Colors,
    out: &mut RenderResult,
    depth: u32,
    surface_tex: Option<&egui::TextureHandle>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let Some(node) = index.get(&id) else { return };

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
            let btn = egui::Button::new(label).fill(fill).corner_radius(style::RADIUS_SM);
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
                    render_node(ui, index, *child, colors, out, depth + 1, surface_tex);
                }
            });
        }

        UiNodeData::Column(c) => {
            ui.with_layout(
                egui::Layout::top_down(cross_align(c.align)),
                |ui| {
                    ui.spacing_mut().item_spacing.y = c.gap;
                    for child in &c.children {
                        render_node(ui, index, *child, colors, out, depth + 1, surface_tex);
                    }
                },
            );
        }

        UiNodeData::ProgressBar(p) => {
            let frac = if p.max > 0.0 { (p.value / p.max).clamp(0.0, 1.0) } else { 0.0 };
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
                        render_node(ui, index, *item_id, colors, out, depth + 1, surface_tex);
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
                render_node(ui, index, s.child, colors, out, depth + 1, surface_tex);
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
                    render_node(ui, index, p.child, colors, out, depth + 1, surface_tex);
                });
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
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(s.width as f32, s.height as f32), egui::Sense::hover());
            match surface_tex {
                Some(tex) => {
                    ui.painter().image(
                        tex.id(),
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                None => {
                    ui.painter().rect_filled(rect, style::RADIUS_MD, colors.bg_active);
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

    // G4 foundation: a real sysmon view tree (after delivering stats) renders
    // through the egui pipeline headlessly without panicking, and produces no
    // spurious actions.
    #[test]
    fn renders_sysmon_tree_headless() -> wasmtime::Result<()> {
        let mut app = WasmApp::load_ephemeral_run("sysmon-render", &fixture(), StateStore::ephemeral())?;
        app.init(&StateSnapshot { entries: vec![] }, (400.0, 300.0))?;
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

        assert!(captured.actions.is_empty(), "sysmon view declares no actions");
        Ok(())
    }
}
