//! Debug card gallery: every card-block state on every surface, full screen.
//!
//! Enabled by `PLEXI_DEBUG_CARD_GALLERY=1` (see `zoomspec/polish/GALLERY.md`).
//! It replaces the central canvas so the real binary renders the real
//! `ui::card_block` paint path against the active theme.

use crate::app::PlexiApp;
use crate::ui::card_block::{layout_card, paint_card, wrap_text, CardForm, CardSpec, CardState};
use crate::ui::style;
use egui::{Align2, FontFamily, FontId, Pos2, Rect, Sense, Vec2};
use std::sync::OnceLock;

/// Whether `PLEXI_DEBUG_CARD_GALLERY` asks for the gallery. Read once.
pub(crate) fn enabled() -> bool {
    #[cfg(test)]
    if FORCE.with(|f| f.get()) {
        return true;
    }
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var("PLEXI_DEBUG_CARD_GALLERY")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false);
        if on {
            log::info!("card gallery enabled via PLEXI_DEBUG_CARD_GALLERY");
        }
        on
    })
}

#[cfg(test)]
thread_local! {
    /// Per-thread override so a test can render the gallery without the env var.
    pub(crate) static FORCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

struct Sample {
    note: &'static str,
    state: CardState,
    title: &'static str,
    subtitle: &'static str,
    detail: &'static str,
    badge: Option<&'static str>,
}

const SAMPLES: &[Sample] = &[
    Sample {
        note: "command waiting to execute",
        state: CardState::CommandWaiting,
        title: "cargo test --workspace",
        subtitle: "build / ci",
        detail: "Queued — press Enter to run in the linked terminal.",
        badge: None,
    },
    Sample {
        note: "agent working, live activity",
        state: CardState::Working,
        title: "refactor-auth",
        subtitle: "backend / auth · 3 panes",
        detail: "Editing src/auth/session.rs — rewriting token refresh, 14 of 22 files touched",
        badge: None,
    },
    Sample {
        note: "needs you, blocked on an ask",
        state: CardState::NeedsYou,
        title: "deploy-bot",
        subtitle: "infra / prod",
        detail: "Allow `terraform apply` against production? It will change 14 resources and replace the primary database instance.",
        badge: Some("◆2"),
    },
    Sample {
        note: "done",
        state: CardState::Done,
        title: "docs-writer",
        subtitle: "docs",
        detail: "Finished: 12 pages updated, 3 diagrams regenerated.",
        badge: None,
    },
    Sample {
        note: "error / failed",
        state: CardState::Failed,
        title: "migrate-db",
        subtitle: "backend / db",
        detail: "exit 1: relation \"users\" already exists (migration 0042_add_users_table)",
        badge: Some("✕1"),
    },
    Sample {
        note: "idle shell",
        state: CardState::Idle,
        title: "zsh",
        subtitle: "~/code/plexi",
        detail: "",
        badge: None,
    },
    Sample {
        note: "very long name + long title",
        state: CardState::Working,
        title: "feature-refactor-the-entire-authentication-and-session-management-subsystem-for-v2",
        subtitle: "platform / identity / sessions / tokens / refresh / rotation / audit",
        detail: "Investigating why refresh-token rotation races with concurrent logout across tabs and devices and then writing the regression test that proves the fix holds under load",
        badge: Some("◆12"),
    },
    Sample {
        note: "short name, no detail",
        state: CardState::Done,
        title: "a",
        subtitle: "",
        detail: "",
        badge: None,
    },
    Sample {
        note: "unbreakable long token",
        state: CardState::Failed,
        title: "/very/long/unbroken/path/without/any/spaces/at/all/to/wrap/on/file.rs",
        subtitle: "ci",
        detail: "ERR_https://example.com/a/really/long/url/that/has/no/spaces/or/break/points/whatsoever",
        badge: Some("✕3"),
    },
    Sample {
        note: "many children",
        state: CardState::Working,
        title: "monorepo",
        subtitle: "14 children · 41 panes",
        detail: "Fleet of agents active across 14 sub-contexts.",
        badge: Some("◆5"),
    },
    Sample {
        note: "few children",
        state: CardState::Idle,
        title: "scratch",
        subtitle: "1 child · 2 panes",
        detail: "Nothing running.",
        badge: None,
    },
];

const NODE_W: f32 = style::ATLAS_NODE_W;
/// Sibling-card width in the gallery (narrower than the real `CARD_W` so two
/// sample columns plus the bar column fit a ~1490px canvas).
const SIB_W: f32 = 300.0;
const GAP: f32 = 10.0;
const CAPTION_H: f32 = 13.0;

/// Where every gallery item sits, relative to the canvas rect it was planned for.
struct Plan {
    /// Per sample: [sibling, atlas, bar] card rects.
    rects: Vec<[Rect; 3]>,
    /// Caption origin for the grid and bar sections, per sample.
    captions: Vec<[Pos2; 2]>,
    bars_label: Pos2,
    col_w: f32,
    total_h: f32,
}

/// Lay the gallery out inside `canvas`: a sibling|atlas grid (two columns when
/// they fit, else one) followed by a band of queue bars. Every rect stays
/// inside `canvas` horizontally.
fn plan(canvas: Rect, layouts: &[[crate::ui::card_block::CardLayout; 3]]) -> Plan {
    let origin = canvas.min;
    let col_w = SIB_W + GAP + NODE_W;
    let ncols = if canvas.width() >= col_w * 2.0 + GAP * 3.0 { 2 } else { 1 };
    let bar_w = ((canvas.width() - GAP * (ncols as f32 + 1.0)) / ncols as f32).max(1.0);
    let header_h = 24.0;
    let mut y = origin.y + header_h + GAP;
    let mut rects = vec![[Rect::NOTHING; 3]; layouts.len()];
    let mut captions = vec![[Pos2::ZERO; 2]; layouts.len()];
    let sib_w = SIB_W.min((canvas.width() - GAP * 3.0) * SIB_W / col_w);
    let node_w = NODE_W.min((canvas.width() - GAP * 3.0) * NODE_W / col_w);
    for chunk in (0..layouts.len()).collect::<Vec<_>>().chunks(ncols) {
        let row_h = chunk
            .iter()
            .map(|&i| layouts[i][0].height.max(layouts[i][1].height))
            .fold(0.0_f32, f32::max);
        for (c, &i) in chunk.iter().enumerate() {
            let x0 = origin.x + GAP + c as f32 * (col_w + GAP);
            captions[i][0] = Pos2::new(x0, y);
            rects[i][0] = Rect::from_min_size(Pos2::new(x0, y + CAPTION_H), Vec2::new(sib_w, layouts[i][0].height));
            rects[i][1] = Rect::from_min_size(
                Pos2::new(x0 + sib_w + GAP, y + CAPTION_H),
                Vec2::new(node_w, layouts[i][1].height),
            );
        }
        y += row_h + CAPTION_H + GAP;
    }
    let bars_label = Pos2::new(origin.x + GAP, y);
    y += CAPTION_H + GAP;
    for chunk in (0..layouts.len()).collect::<Vec<_>>().chunks(ncols) {
        let row_h = chunk.iter().map(|&i| layouts[i][2].height).fold(0.0_f32, f32::max);
        for (c, &i) in chunk.iter().enumerate() {
            let x0 = origin.x + GAP + c as f32 * (bar_w + GAP);
            captions[i][1] = Pos2::new(x0, y);
            rects[i][2] = Rect::from_min_size(Pos2::new(x0, y + CAPTION_H), Vec2::new(bar_w, layouts[i][2].height));
        }
        y += row_h + CAPTION_H + 4.0;
    }
    Plan { rects, captions, bars_label, col_w, total_h: y - origin.y + GAP }
}

fn specs_for(
    colors: &crate::ui::theme::Colors,
    sample: &Sample,
) -> [CardSpec<'static>; 3] {
    let badge = sample.badge.map(|t| (t.to_string(), colors_for(colors, sample.state)));
    let spec = |form: CardForm| {
        let badge = match form {
            CardForm::Bar => Some(match sample.state {
                CardState::NeedsYou => ("Enter ▸ Decide".to_string(), colors.zoom_needs),
                _ => ("✕".to_string(), colors.text_dim),
            }),
            _ => badge.clone(),
        };
        CardSpec {
            state: sample.state,
            title: sample.title,
            subtitle: sample.subtitle,
            detail: sample.detail,
            badge,
        }
    };
    [spec(CardForm::Sibling), spec(CardForm::Atlas), spec(CardForm::Bar)]
}

/// Measure every sample at the widths `plan` will give it.
fn measure_all(
    ctx: &egui::Context,
    colors: &crate::ui::theme::Colors,
    canvas_w: f32,
) -> (Vec<[CardSpec<'static>; 3]>, Vec<[crate::ui::card_block::CardLayout; 3]>) {
    let col_w = SIB_W + GAP + NODE_W;
    let ncols = if canvas_w >= col_w * 2.0 + GAP * 3.0 { 2.0 } else { 1.0 };
    let bar_w = ((canvas_w - GAP * (ncols + 1.0)) / ncols).max(1.0);
    let sib_w = SIB_W.min((canvas_w - GAP * 3.0) * SIB_W / col_w);
    let node_w = NODE_W.min((canvas_w - GAP * 3.0) * NODE_W / col_w);
    let specs: Vec<_> = SAMPLES.iter().map(|s| specs_for(colors, s)).collect();
    let layouts = specs
        .iter()
        .map(|sp| {
            [
                layout_card(ctx, CardForm::Sibling, &sp[0], sib_w),
                layout_card(ctx, CardForm::Atlas, &sp[1], node_w),
                layout_card(ctx, CardForm::Bar, &sp[2], bar_w),
            ]
        })
        .collect();
    (specs, layouts)
}

impl PlexiApp {
    pub(crate) fn draw_card_gallery(&mut self, ui: &mut egui::Ui) {
        let colors = self.colors;
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let avail_w = ui.available_width();
            let origin = ui.available_rect_before_wrap().min;
            let canvas = Rect::from_min_size(origin, Vec2::new(avail_w, 0.0));
            let header_font = crate::ui::theme::font_medium(style::TEXT_BODY);
            let note_font = FontId::new(style::TEXT_HINT, FontFamily::Proportional);
            let (specs, layouts) = measure_all(&ctx, &colors, avail_w);
            let plan = plan(canvas, &layouts);

            let (area, _) = ui.allocate_exact_size(Vec2::new(avail_w, plan.total_h), Sense::hover());
            let painter = ui.painter_at(area);
            painter.rect_filled(area, 0.0, colors.bg_darkest);
            painter.text(
                Pos2::new(origin.x + GAP, origin.y + 4.0),
                Align2::LEFT_TOP,
                "Card gallery — sibling · atlas node | queue bars (PLEXI_DEBUG_CARD_GALLERY=1)",
                header_font,
                colors.text_primary,
            );
            painter.text(
                plan.bars_label,
                Align2::LEFT_TOP,
                "Queue bars",
                note_font.clone(),
                colors.text_primary,
            );
            for (i, sample) in SAMPLES.iter().enumerate() {
                let note = wrap_text(
                    &ctx,
                    &format!("{} ({})", sample.note, sample.state.label()),
                    note_font.clone(),
                    plan.col_w,
                    Some(1),
                );
                painter.text(plan.captions[i][0], Align2::LEFT_TOP, note.join(""), note_font.clone(), colors.text_dim);
                painter.text(
                    plan.captions[i][1],
                    Align2::LEFT_TOP,
                    sample.state.label(),
                    note_font.clone(),
                    colors.text_dim,
                );
                for k in 0..2 {
                    paint_card(&painter, plan.rects[i][k], &layouts[i][k], &specs[i][k], &colors, colors.depth_tint(1), false);
                }
                paint_card(&painter, plan.rects[i][2], &layouts[i][2], &specs[i][2], &colors, colors.tile_bg, false);
            }
        });
    }
}

fn colors_for(colors: &crate::ui::theme::Colors, state: CardState) -> egui::Color32 {
    match state {
        CardState::Failed => colors.zoom_failed,
        _ => colors.zoom_needs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1440x900 physical window at UI scale `s` has a `1440/s x 900/s`
    /// logical canvas; every gallery rect must stay inside it.
    #[test]
    fn card_gallery_items_fit_scaled_canvas() {
        let ctx = egui::Context::default();
        let colors = crate::ui::theme::Colors::from_config(&crate::config::ThemeConfig::default());
        crate::ui::theme::setup_fonts(&ctx);
        let _ = ctx.run(Default::default(), |_| {});
        for scale in [0.6_f32, 1.0, 2.0] {
            ctx.set_zoom_factor(scale);
            let _ = ctx.run(Default::default(), |ctx| {
                let (w, h) = (1440.0 / scale, 900.0 / scale);
                let canvas = Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h));
                let (_, layouts) = measure_all(ctx, &colors, w);
                let plan = plan(canvas, &layouts);
                for rs in &plan.rects {
                    for r in rs {
                        assert!(
                            r.min.x >= canvas.min.x - 0.5 && r.max.x <= canvas.max.x + 0.5,
                            "rect {r:?} outside {w}-wide canvas at scale {scale}"
                        );
                    }
                }
            });
        }
    }

    #[test]
    fn card_gallery_items_fit_canvas() {
        let ctx = egui::Context::default();
        let colors = crate::ui::theme::Colors::from_config(&crate::config::ThemeConfig::default());
        crate::ui::theme::setup_fonts(&ctx);
        let _ = ctx.run(Default::default(), |_| {});
        let _ = ctx.run(Default::default(), |ctx| {
            for (w, h) in [(1370.0, 930.0), (1000.0, 700.0)] {
                let canvas = Rect::from_min_size(Pos2::new(40.0, 20.0), Vec2::new(w, h));
                let (_, layouts) = measure_all(ctx, &colors, w);
                let plan = plan(canvas, &layouts);
                for (i, rs) in plan.rects.iter().enumerate() {
                    for r in rs {
                        assert!(
                            r.min.x >= canvas.min.x && r.max.x <= canvas.max.x,
                            "sample {i} rect {r:?} outside canvas x-range at {w}"
                        );
                    }
                }
            }
        });
    }
}
