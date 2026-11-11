//! Diagnostics over emitted paint, not a second execution of the allocator.
//! Enabled in debug builds or with PLEXI_UI_LAYOUT_REPORT=1. Reports belong to
//! one egui pass; unrelated layers and intentional parent/child overlaps are
//! kept separate. Arbitrary canvas content is not implicitly covered.
use egui::{Color32, Painter, Rect, Shape};

#[derive(Clone, Debug)]
pub(crate) struct Action {
    pub id: String,
    pub name: String,
    pub painted_label: String,
    pub hit: Rect,
    pub minimum: egui::Vec2,
    /// The action is disabled. Its label is an inactive component (WCAG 1.4.3)
    /// and is held to `INACTIVE_TEXT_MIN` instead of the SmallText minimum.
    pub inactive: bool,
}

/// Legibility floor for inactive (disabled) text. WCAG 1.4.3 exempts inactive
/// components from any minimum; this floor keeps disabled labels from
/// vanishing into their chip.
pub(crate) const INACTIVE_TEXT_MIN: f32 = 2.0;

#[derive(Clone, Debug)]
pub(crate) struct TextPaint {
    pub id: String,
    pub text: String,
    pub allocated: Rect,
    pub paint: Rect,
    pub clip: Rect,
    pub foreground: Color32,
    pub background: Color32,
    pub fit: &'static str,
    pub inactive: bool,
}

#[derive(Clone, Debug)]
struct TextSlot {
    id: String,
    text: String,
    allocated: Rect,
    outcome: &'static str,
}

#[derive(Clone, Debug)]
pub(crate) struct LayoutReport {
    pub surface: String,
    pub layer: String,
    pub parent: Rect,
    pub texts: Vec<TextPaint>,
    pub actions: Vec<Action>,
    pub exclusions: Vec<Rect>,
    slots: Vec<TextSlot>,
    scroll: bool,
}

fn contained(parent: Rect, child: Rect) -> bool {
    child.is_finite() && !child.is_negative() && parent.expand(0.75).contains_rect(child)
}
fn overlaps(a: Rect, b: Rect) -> bool {
    a.intersect(b).width() > 0.75 && a.intersect(b).height() > 0.75
}

impl LayoutReport {
    pub fn violations(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if !self.parent.is_finite() || self.parent.is_negative() {
            errors.push("invalid parent".into());
        }
        for slot in &self.slots {
            if !contained(self.parent, slot.allocated) {
                errors.push(format!("{}: slot leaves parent", slot.id));
            }
            if slot.outcome != "hidden: insufficient text budget"
                && !self.texts.iter().any(|t| t.id == slot.id)
            {
                errors.push(format!("{}: allocated text was not painted", slot.id));
            }
        }
        for (index, text) in self.texts.iter().enumerate() {
            if !contained(self.parent, text.paint) || !contained(text.allocated, text.paint) {
                errors.push(format!("{}: paint leaves allocation", text.id));
            }
            // A scrolled row may intersect the viewport boundary deliberately.
            if text.fit != "scroll" && !contained(text.clip, text.paint) {
                errors.push(format!("{}: unexplained text clipping", text.id));
            }
            let ratio = super::theme::contrast(
                super::theme::luminance(text.foreground),
                super::theme::luminance(text.background),
            );
            let minimum = if text.inactive {
                INACTIVE_TEXT_MIN
            } else {
                super::foreground::Role::SmallText.minimum()
            };
            if ratio + 0.001 < minimum {
                errors.push(format!("{}: contrast {ratio:.3}", text.id));
            }
            for other in &self.texts[index + 1..] {
                if overlaps(text.paint, other.paint) {
                    errors.push(format!("{} overlaps {}", text.id, other.id));
                }
            }
            if self.exclusions.iter().any(|r| overlaps(*r, text.paint)) {
                errors.push(format!("{}: native exclusion", text.id));
            }
        }
        for (index, action) in self.actions.iter().enumerate() {
            if action.name.trim().is_empty()
                || action.painted_label.trim().is_empty()
                || !contained(self.parent, action.hit)
                || action.hit.width() + 0.75 < action.minimum.x
                || action.hit.height() + 0.75 < action.minimum.y
            {
                errors.push(format!("{}: inaccessible action", action.id));
            }
            if !self.texts.iter().any(|text| {
                text.text == action.painted_label
                    && !text.paint.is_negative()
                    && text.paint.width() > 0.0
                    && contained(action.hit, text.paint)
                    && (self.scroll || contained(text.clip, text.paint))
            }) {
                errors.push(format!("{}: required label missing or clipped", action.id));
            }
            if self.actions[index + 1..]
                .iter()
                .any(|other| overlaps(action.hit, other.hit))
            {
                errors.push(format!("{}: sibling hit overlap", action.id));
            }
            if self.exclusions.iter().any(|r| overlaps(*r, action.hit)) {
                errors.push(format!("{}: native hit exclusion", action.id));
            }
        }
        errors
            .into_iter()
            .map(|error| format!("{}: {error}", self.surface))
            .collect()
    }

    #[cfg(test)]
    pub fn json(&self) -> serde_json::Value {
        let rect = |r: Rect| [r.left(), r.top(), r.right(), r.bottom()];
        serde_json::json!({
            "surface": self.surface, "layer": self.layer, "parent": rect(self.parent), "scroll": self.scroll,
            "texts": self.texts.iter().map(|t| serde_json::json!({
                "slot": t.id, "text": t.text, "allocated": rect(t.allocated), "paint": rect(t.paint),
                "clip": rect(t.clip), "foreground": t.foreground.to_array(),
                "background": t.background.to_array(), "fit": t.fit, "inactive": t.inactive,
            })).collect::<Vec<_>>(),
            "actions": self.actions.iter().map(|a| serde_json::json!({
                "slot": a.id, "name": a.name, "painted_label": a.painted_label,
                "hit": rect(a.hit), "minimum": [a.minimum.x, a.minimum.y],
            })).collect::<Vec<_>>(),
            "slots": self.slots.iter().map(|s| serde_json::json!({"slot": s.id, "allocated": rect(s.allocated), "fit": s.outcome})).collect::<Vec<_>>(),
            "exclusions": self.exclusions.iter().map(|r| rect(*r)).collect::<Vec<_>>(),
            "violations": self.violations(),
        })
    }
}

#[derive(Clone, Default)]
struct Reports {
    pass: u64,
    items: Vec<LayoutReport>,
}
fn key() -> egui::Id {
    egui::Id::new("host_layout_reports")
}

#[cfg(test)]
pub(crate) fn snapshot(ctx: &egui::Context) -> Vec<LayoutReport> {
    ctx.data(|d| d.get_temp::<Reports>(key()))
        .filter(|r| ctx.cumulative_pass_nr().saturating_sub(r.pass) <= 1)
        .map_or_else(Vec::new, |r| r.items)
}

pub(crate) struct Capture {
    surface: &'static str,
    start: Option<usize>,
    slots: Vec<TextSlot>,
    scroll: bool,
}

impl Capture {
    pub fn begin(painter: &Painter, surface: &'static str) -> Self {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let enabled = *ENABLED.get_or_init(|| {
            cfg!(debug_assertions) || std::env::var("PLEXI_UI_LAYOUT_REPORT").as_deref() == Ok("1")
        });
        Self {
            surface,
            slots: Vec::new(),
            scroll: false,
            start: enabled.then(|| {
                painter.ctx().graphics(|g| {
                    g.get(painter.layer_id())
                        .map_or(0, |p| p.all_entries().len())
                })
            }),
        }
    }

    /// Scroll containers explicitly own viewport clipping; full allocated
    /// geometry and labels are still checked, including offscreen rows.
    pub fn scrolling(&mut self) {
        self.scroll = true;
    }

    pub fn text_slot(&mut self, id: impl Into<String>, galley: &egui::Galley, allocated: Rect) {
        if self.start.is_some() {
            let text = galley.job.text.clone();
            let outcome = if text.is_empty() {
                "hidden: insufficient text budget"
            } else if text.contains('…') {
                "elided"
            } else if galley.rows.len() > 1 {
                "wrapped"
            } else {
                "full"
            };
            self.slots.push(TextSlot {
                id: id.into(),
                text,
                allocated,
                outcome,
            });
        }
    }

    pub fn finish(
        self,
        painter: &Painter,
        parent: Rect,
        backdrop: Color32,
        actions: Vec<Action>,
        exclusions: Vec<Rect>,
    ) {
        let Some(start) = self.start else {
            return;
        };
        let entries = painter.ctx().graphics(|g| {
            g.get(painter.layer_id())
                .map(|p| p.all_entries().skip(start).cloned().collect::<Vec<_>>())
                .unwrap_or_default()
        });
        let mut report = LayoutReport {
            surface: self.surface.into(),
            layer: format!("{:?}", painter.layer_id()),
            parent,
            texts: Vec::new(),
            actions,
            exclusions,
            slots: self.slots,
            scroll: self.scroll,
        };
        let mut fills = Vec::new();
        for entry in &entries {
            collect(
                &entry.shape,
                entry.clip_rect,
                parent,
                backdrop,
                &mut fills,
                &mut report.texts,
            );
        }
        if !report.scroll {
            report.texts.retain(|t| t.clip.intersects(t.paint));
        }
        for text in &mut report.texts {
            if let Some(slot) = report
                .slots
                .iter()
                .find(|s| s.text == text.text && contained(s.allocated, text.paint))
                .or_else(|| report.slots.iter().find(|s| s.text == text.text))
            {
                text.id = slot.id.clone();
                text.allocated = slot.allocated;
                text.fit = slot.outcome;
            }
            if let Some(action) = report.actions.iter().find(|a| a.painted_label == text.text) {
                text.id = action.id.clone();
                text.allocated = action.hit;
                text.inactive = action.inactive;
            }
        }
        if report.scroll {
            for text in &mut report.texts {
                if !contained(text.clip, text.paint) {
                    text.fit = "scroll";
                }
            }
        }
        let errors = report.violations();
        if !errors.is_empty() {
            // Diagnostics are bounded to one message per surface per context.
            let error_key = egui::Id::new(("layout_violation_logged", self.surface));
            if !painter
                .ctx()
                .data(|d| d.get_temp::<bool>(error_key).unwrap_or(false))
            {
                log::info!(
                    "ui layout {} layer={}: {}",
                    self.surface,
                    report.layer,
                    errors.join("; ")
                );
                painter.ctx().data_mut(|d| d.insert_temp(error_key, true));
            }
        }
        let pass = painter.ctx().cumulative_pass_nr();
        painter.ctx().data_mut(|d| {
            let mut reports = d.get_temp::<Reports>(key()).unwrap_or_default();
            if reports.pass != pass {
                reports.items.clear();
                reports.pass = pass;
            }
            reports.items.push(report);
            d.insert_temp(key(), reports);
        });
    }
}

fn collect(
    shape: &Shape,
    clip: Rect,
    parent: Rect,
    backdrop: Color32,
    fills: &mut Vec<(Rect, Color32)>,
    texts: &mut Vec<TextPaint>,
) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, clip, parent, backdrop, fills, texts);
            }
        }
        Shape::Rect(rect) if rect.fill != Color32::TRANSPARENT => {
            fills.push((rect.rect, rect.fill))
        }
        Shape::Text(shape) if !shape.galley.job.text.is_empty() => {
            let paint = shape.galley.rect.translate(shape.pos.to_vec2());
            let background = fills
                .iter()
                .filter(|(r, _)| r.contains(paint.center()))
                .fold(backdrop, |bg, (_, fg)| bg.blend(*fg));
            let color = shape
                .override_text_color
                .or_else(|| {
                    shape
                        .galley
                        .job
                        .sections
                        .first()
                        .map(|s| s.format.color)
                        .filter(|c| *c != Color32::PLACEHOLDER)
                })
                .unwrap_or(shape.fallback_color);
            let text = shape.galley.job.text.clone();
            texts.push(TextPaint {
                id: format!("text-{}", texts.len()),
                text: text.clone(),
                allocated: parent,
                paint,
                clip,
                foreground: background.blend(color),
                background,
                fit: if text.contains('…') {
                    "elided"
                } else if shape.galley.rows.len() > 1 {
                    "wrapped"
                } else {
                    "full"
                },
                inactive: false,
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> LayoutReport {
        let parent = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 32.0));
        let hit = Rect::from_min_size(egui::pos2(140.0, 0.0), egui::vec2(60.0, 32.0));
        LayoutReport {
            surface: "toast".into(),
            layer: "foreground".into(),
            parent,
            texts: vec![TextPaint {
                id: "undo".into(),
                text: "Undo".into(),
                allocated: hit,
                paint: hit.shrink(8.0),
                clip: parent,
                foreground: Color32::WHITE,
                background: Color32::BLACK,
                fit: "full",
                inactive: false,
            }],
            actions: vec![Action {
                id: "undo".into(),
                name: "Undo".into(),
                painted_label: "Undo".into(),
                hit,
                minimum: egui::vec2(28.0, 28.0),
                inactive: false,
            }],
            exclusions: vec![],
            slots: vec![],
            scroll: false,
        }
    }
    #[test]
    fn emitted_button_contrast_includes_disabled_tint() {
        // Inactive text is exempt from SmallText 4.5:1 but held to
        // INACTIVE_TEXT_MIN. `faded_below_small_text` proves the exemption is
        // doing work: some theme's disabled label really is under 4.5:1.
        let mut faded_below_small_text = false;
        for theme in crate::ui::theme::preset_names() {
            let ctx = egui::Context::default();
            let colors = crate::ui::theme::Colors::from_config(
                &crate::ui::theme::preset_colors(theme).unwrap(),
            );
            // The kit button labels in the medium UI family, which a bare
            // context does not bind.
            crate::ui::theme::setup_fonts(&ctx);
            crate::ui::theme::setup_style(&ctx, &colors);
            for enabled in [true, false] {
                let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                    for kind in [
                        crate::ui::button::ButtonKind::Primary,
                        crate::ui::button::ButtonKind::Secondary,
                        crate::ui::button::ButtonKind::Danger,
                        crate::ui::button::ButtonKind::Ghost,
                    ] {
                        let capture = Capture::begin(ui.painter(), "button");
                        let response = ui
                            .add_enabled_ui(enabled, |ui| {
                                ui.add(
                                    crate::ui::button::KitButton::new("Action", &colors)
                                        .kind(kind)
                                        .min_width(80.0),
                                )
                            })
                            .inner;
                        capture.finish(
                            ui.painter(),
                            response.rect,
                            ui.visuals().panel_fill,
                            vec![Action {
                                id: "action".into(),
                                name: "Action".into(),
                                painted_label: "Action".into(),
                                hit: response.rect,
                                minimum: egui::vec2(28.0, 28.0),
                                inactive: !enabled,
                            }],
                            vec![],
                        );
                    }
                });
                let errors: Vec<_> = snapshot(&ctx)
                    .iter()
                    .flat_map(LayoutReport::violations)
                    .collect();
                assert!(errors.is_empty(), "{theme} enabled={enabled}: {errors:?}");
                for report in snapshot(&ctx) {
                    for text in &report.texts {
                        assert_eq!(text.inactive, !enabled, "{theme}: inactive flag");
                        let ratio = crate::ui::theme::contrast(
                            crate::ui::theme::luminance(text.foreground),
                            crate::ui::theme::luminance(text.background),
                        );
                        if !enabled {
                            assert!(
                                ratio + 0.001 >= INACTIVE_TEXT_MIN,
                                "{theme}: disabled label {ratio:.2} under the floor"
                            );
                            faded_below_small_text |=
                                ratio < crate::ui::foreground::Role::SmallText.minimum();
                        }
                    }
                }
            }
        }
        assert!(
            faded_below_small_text,
            "disabled labels are not dimmed below SmallText in any theme"
        );
    }

    #[test]
    fn inactive_text_below_the_floor_is_still_a_violation() {
        let mut report = sample();
        report.texts[0].foreground = Color32::from_gray(120);
        report.texts[0].background = Color32::from_gray(128);
        report.texts[0].inactive = true;
        assert!(
            report.violations().iter().any(|s| s.contains("contrast")),
            "inactive text near its background must still fail"
        );
        report.texts[0].foreground = Color32::from_gray(190);
        report.texts[0].background = Color32::from_gray(60);
        assert!(
            report.violations().iter().all(|s| !s.contains("contrast")),
            "inactive text above the floor passes: {:?}",
            report.violations()
        );
        report.texts[0].inactive = false;
        report.texts[0].foreground = Color32::from_gray(100);
        report.texts[0].background = Color32::from_gray(60);
        assert!(
            report.violations().iter().any(|s| s.contains("contrast")),
            "active low-contrast text keeps the 4.5:1 minimum"
        );
    }

    #[test]
    fn actual_painter_escaping_its_slot_is_detected() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let capture = Capture::begin(ui.painter(), "negative-painted-row");
            let parent = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(60.0, 32.0));
            ui.painter().text(
                egui::pos2(80.0, 8.0),
                egui::Align2::LEFT_TOP,
                "Undo",
                egui::FontId::proportional(12.0),
                Color32::WHITE,
            );
            capture.finish(ui.painter(), parent, Color32::BLACK, vec![], vec![]);
        });
        assert!(snapshot(&ctx)
            .iter()
            .flat_map(LayoutReport::violations)
            .any(|s| s.contains("allocation")));
    }

    #[test]
    fn layout_negative_controls_detect_incidents_and_wrong_paint() {
        let valid = sample();
        assert!(valid.violations().is_empty());
        let mut starved = valid.clone();
        starved.texts.clear();
        assert!(starved
            .violations()
            .iter()
            .any(|s| s.contains("label missing")));
        let mut native = valid.clone();
        native.exclusions.push(native.actions[0].hit);
        assert!(native.violations().iter().any(|s| s.contains("native")));
        let mut contrast = valid.clone();
        contrast.texts[0].foreground = Color32::BLACK;
        assert!(contrast.violations().iter().any(|s| s.contains("contrast")));
        let mut wrong_painter = valid;
        wrong_painter.texts[0].paint = wrong_painter.texts[0]
            .paint
            .translate(egui::vec2(-100.0, 0.0));
        assert!(wrong_painter
            .violations()
            .iter()
            .any(|s| s.contains("allocation")));
    }
}
