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
const BAR_W: f32 = 520.0;
const NOTE_W: f32 = 150.0;
const GAP: f32 = 18.0;

impl PlexiApp {
    pub(crate) fn draw_card_gallery(&mut self, ui: &mut egui::Ui) {
        let colors = self.colors;
        let ctx = ui.ctx().clone();
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            let origin = ui.available_rect_before_wrap().min;
            let mut y = origin.y + GAP;
            let total_w = GAP + NOTE_W + style::CARD_W + GAP + NODE_W + GAP + BAR_W + GAP;
            let header_font = crate::ui::theme::font_medium(style::TEXT_BODY);
            let note_font = FontId::new(style::TEXT_HINT, FontFamily::Proportional);

            // Measure first so the scroll area knows its full extent.
            let mut rows = Vec::new();
            let mut total_h = GAP + 30.0;
            for sample in SAMPLES {
                let badge = sample
                    .badge
                    .map(|t| (t.to_string(), colors_for(&colors, sample.state)));
                let spec = |form: CardForm| {
                    let (badge, subtitle) = match form {
                        CardForm::Bar => (
                            Some(match sample.state {
                                CardState::NeedsYou => {
                                    ("Enter ▸ Decide".to_string(), colors.zoom_needs)
                                }
                                _ => ("✕".to_string(), colors.text_dim),
                            }),
                            sample.subtitle,
                        ),
                        _ => (badge.clone(), sample.subtitle),
                    };
                    CardSpec {
                        state: sample.state,
                        title: sample.title,
                        subtitle,
                        detail: sample.detail,
                        badge,
                    }
                };
                let specs = [
                    spec(CardForm::Sibling),
                    spec(CardForm::Atlas),
                    spec(CardForm::Bar),
                ];
                let layouts = [
                    layout_card(&ctx, CardForm::Sibling, &specs[0], style::CARD_W),
                    layout_card(&ctx, CardForm::Atlas, &specs[1], NODE_W),
                    layout_card(&ctx, CardForm::Bar, &specs[2], BAR_W),
                ];
                let h = layouts.iter().map(|l| l.height).fold(0.0_f32, f32::max);
                total_h += h + GAP;
                rows.push((specs, layouts, h));
            }

            let (area, _) = ui.allocate_exact_size(
                Vec2::new(total_w.max(ui.available_width()), total_h),
                Sense::hover(),
            );
            let painter = ui.painter_at(area);
            painter.rect_filled(area, 0.0, colors.bg_darkest);
            painter.text(
                Pos2::new(origin.x + GAP, y),
                Align2::LEFT_TOP,
                "Card gallery — sibling card · atlas node · queue bar (PLEXI_DEBUG_CARD_GALLERY=1)",
                header_font,
                colors.text_primary,
            );
            y += 30.0;

            for (sample, (specs, layouts, h)) in SAMPLES.iter().zip(rows.iter()) {
                let note = wrap_text(
                    &ctx,
                    &format!("{} ({})", sample.note, sample.state.label()),
                    note_font.clone(),
                    NOTE_W - GAP,
                    Some(4),
                );
                painter.text(
                    Pos2::new(origin.x + GAP, y + 4.0),
                    Align2::LEFT_TOP,
                    note.join("\n"),
                    note_font.clone(),
                    colors.text_dim,
                );
                let mut x = origin.x + GAP + NOTE_W;
                for (i, (form, w)) in [
                    (CardForm::Sibling, style::CARD_W),
                    (CardForm::Atlas, NODE_W),
                    (CardForm::Bar, BAR_W),
                ]
                .into_iter()
                .enumerate()
                {
                    let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, layouts[i].height));
                    let fill = match form {
                        CardForm::Bar => colors.tile_bg,
                        _ => colors.depth_tint(1),
                    };
                    paint_card(&painter, rect, &layouts[i], &specs[i], &colors, fill, false);
                    x += w + GAP;
                }
                y += h + GAP;
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
