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
const MIN_BAR_W: f32 = 240.0;
const GAP: f32 = 10.0;
const CAPTION_H: f32 = 13.0;

impl PlexiApp {
    pub(crate) fn draw_card_gallery(&mut self, ui: &mut egui::Ui) {
        let colors = self.colors;
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let avail_w = ui.available_width();
            let origin = ui.available_rect_before_wrap().min;
            let header_font = crate::ui::theme::font_medium(style::TEXT_BODY);
            let note_font = FontId::new(style::TEXT_HINT, FontFamily::Proportional);
            // Two sample columns of [sibling | atlas], then the bar column
            // taking whatever width is left.
            let col_w = SIB_W + GAP + NODE_W;
            let bar_w = (avail_w - GAP * 4.0 - col_w * 2.0 - GAP).max(MIN_BAR_W);

            let mut cells = Vec::new();
            for sample in SAMPLES {
                let badge = sample
                    .badge
                    .map(|t| (t.to_string(), colors_for(&colors, sample.state)));
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
                let specs = [spec(CardForm::Sibling), spec(CardForm::Atlas), spec(CardForm::Bar)];
                let layouts = [
                    layout_card(&ctx, CardForm::Sibling, &specs[0], SIB_W),
                    layout_card(&ctx, CardForm::Atlas, &specs[1], NODE_W),
                    layout_card(&ctx, CardForm::Bar, &specs[2], bar_w),
                ];
                cells.push((specs, layouts));
            }

            let header_h = 24.0;
            let rows = SAMPLES.len().div_ceil(2);
            let row_hs: Vec<f32> = (0..rows)
                .map(|r| {
                    cells[r * 2..(r * 2 + 2).min(cells.len())]
                        .iter()
                        .map(|(_, l)| l[0].height.max(l[1].height))
                        .fold(0.0_f32, f32::max)
                        + CAPTION_H
                        + GAP
                })
                .collect();
            let grid_h: f32 = row_hs.iter().sum();
            let bars_h: f32 = cells.iter().map(|(_, l)| l[2].height + CAPTION_H + 4.0).sum();
            let total_h = header_h + GAP + grid_h.max(bars_h) + GAP;

            let (area, _) = ui.allocate_exact_size(Vec2::new(avail_w, total_h), Sense::hover());
            let painter = ui.painter_at(area);
            painter.rect_filled(area, 0.0, colors.bg_darkest);
            painter.text(
                Pos2::new(origin.x + GAP, origin.y + 4.0),
                Align2::LEFT_TOP,
                "Card gallery — sibling · atlas node | queue bars (PLEXI_DEBUG_CARD_GALLERY=1)",
                header_font,
                colors.text_primary,
            );
            let top = origin.y + header_h + GAP;
            let caption = |sample: &Sample| {
                format!("{} ({})", sample.note, sample.state.label())
            };
            let bar_x = origin.x + GAP + (col_w + GAP) * 2.0;

            let mut ys = vec![top];
            for h in &row_hs {
                ys.push(ys.last().copied().unwrap_or(top) + h);
            }
            for (i, (sample, (specs, layouts))) in SAMPLES.iter().zip(cells.iter()).enumerate() {
                let (row, col) = (i / 2, i % 2);
                let x0 = origin.x + GAP + col as f32 * (col_w + GAP);
                let y = ys[row];
                let note = wrap_text(&ctx, &caption(sample), note_font.clone(), col_w, Some(1));
                painter.text(
                    Pos2::new(x0, y),
                    Align2::LEFT_TOP,
                    note.join(""),
                    note_font.clone(),
                    colors.text_dim,
                );
                for (k, x, w) in [(0, x0, SIB_W), (1, x0 + SIB_W + GAP, NODE_W)] {
                    let rect = Rect::from_min_size(
                        Pos2::new(x, y + CAPTION_H),
                        Vec2::new(w, layouts[k].height),
                    );
                    paint_card(&painter, rect, &layouts[k], &specs[k], &colors, colors.depth_tint(1), false);
                }
            }
            let mut y = top;
            for (sample, (specs, layouts)) in SAMPLES.iter().zip(cells.iter()) {
                painter.text(
                    Pos2::new(bar_x, y),
                    Align2::LEFT_TOP,
                    sample.state.label(),
                    note_font.clone(),
                    colors.text_dim,
                );
                let rect = Rect::from_min_size(
                    Pos2::new(bar_x, y + CAPTION_H),
                    Vec2::new(bar_w, layouts[2].height),
                );
                paint_card(&painter, rect, &layouts[2], &specs[2], &colors, colors.tile_bg, false);
                y += layouts[2].height + CAPTION_H + 4.0;
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
