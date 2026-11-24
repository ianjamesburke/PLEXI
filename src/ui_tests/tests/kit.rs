use super::*;

/// Stint 1636: lists and tables built from self-describing row nodes
/// render a realistic SDK tree (a selected list row with description and
/// trailing text, a headered table with a selected row, a headerless
/// table). The tree is the SDK's golden encoding.
#[test]
fn screenshot_list_and_table_rows() {
    let tree = crate::host::wasm_python::decode_ui_tree(include_str!(
        "../../../sdk/python/tests/golden/rows/gallery.json"
    ))
    .expect("gallery tree decodes");
    let png = crate::host::wasm_render::render_ui_tree_to_png(&tree, 520.0, 420.0, 2.0)
        .expect("render rows");
    std::fs::write(evidence_png!(), png).expect("write evidence");
}

#[test]
fn screenshot_card_gallery_all_states_and_surfaces() {
    for theme in ["plexi-dusk", "plexi-dawn"] {
        for scale in [1.0, 2.0] {
            crate::app::card_gallery::FORCE.with(|f| f.set(true));
            let mut h = PlexiUiHarness::new_sized_scaled(1440.0 * scale, 1400.0 * scale, scale);
            apply_fixture_theme(&mut h, theme);
            h.run_steps(4);
            h.with_app(|app| {
                let reports = crate::ui::layout_report::snapshot(&app.ctx);
                assert!(reports.iter().any(|r| r.surface == "card"));
                let errors: Vec<_> = reports.iter().flat_map(|r| r.violations()).collect();
                assert!(errors.is_empty(), "{errors:?}");
            });
            h.save_screenshot(&evidence_png!(&format!("_{theme}_{scale}")))
                .unwrap();
            crate::app::card_gallery::FORCE.with(|f| f.set(false));
        }
    }
}

/// Stint 1405: text on an accent or danger fill is light whenever white
/// clears the 4.5:1 small-text floor, dark otherwise, and always clears
/// it, on every Plexi preset and on user-chosen accents. Renders
/// the SDK tab bar (the active tab is a Primary button), a Primary and a
/// Danger button and accent/danger badges through the real tree renderer,
/// then reads the colour each label was painted with.
#[test]
fn on_accent_text_prefers_white_and_clears_small_text_floor() {
    use crate::ui::theme::{contrast, luminance, preset_colors, Colors};
    fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Color32)>) {
        match shape {
            egui::Shape::Text(text) => {
                let color = text.override_text_color.unwrap_or_else(|| {
                    text.galley
                        .job
                        .sections
                        .first()
                        .map_or(text.fallback_color, |s| s.format.color)
                });
                out.push((text.galley.text().to_owned(), color));
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let button = |id: u32, label: &str, style: &str| {
        serde_json::json!({"id": id, "key": format!("0/{id}"), "data": {
            "type": "Button", "label": label, "on_click": label, "style": style}})
    };
    let badge = |id: u32, text: &str, color: &str| {
        serde_json::json!({"id": id, "key": format!("0/{id}"), "data": {
            "type": "Badge", "text": text, "color": color}})
    };
    let tree = serde_json::json!({"root": 0, "nodes": [
        {"id": 0, "key": "0", "data": {"type": "Row", "children": [1, 2, 3, 4, 5, 6, 7], "gap": 4.0}},
        button(1, "Overview", "primary"),
        button(2, "Details", "secondary"),
        button(3, "Logs", "secondary"),
        button(4, "Save", "primary"),
        button(5, "Delete", "danger"),
        badge(6, "3 new", "accent"),
        badge(7, "2 failed", "danger"),
    ]});
    let tree =
        crate::host::wasm_python::decode_ui_tree(&tree.to_string()).expect("tree decodes");
    // Every Plexi preset's own accent, then user-chosen accents on the
    // default preset: mid (the old dusk accent, white fails), light, dark.
    let mut cases: Vec<(String, crate::config::ThemeConfig)> =
        ["plexi-night", "plexi-dusk", "plexi-dawn", "plexi-day"]
            .into_iter()
            .map(|theme| (theme.to_string(), preset_colors(theme).unwrap()))
            .collect();
    for accent in ["#9568f6", "#00ff41", "#1e40af"] {
        let mut cfg = preset_colors(crate::ui::theme::DEFAULT_PRESET).unwrap();
        cfg.accent = Some(accent.to_string());
        cases.push((format!("custom-{}", &accent[1..]), cfg));
    }
    for (theme, cfg) in cases {
        let colors = Colors::from_config(&cfg);
        let render_tree = tree.clone();
        let mut fonts_ready = false;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(560.0, 64.0))
            .with_pixels_per_point(2.0)
            .build_ui(move |ui| {
                if !fonts_ready {
                    crate::ui::theme::setup_fonts(ui.ctx());
                    crate::ui::theme::setup_style(ui.ctx(), &colors);
                    fonts_ready = true;
                    ui.ctx().request_repaint();
                    return;
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(colors.terminal_bg).inner_margin(12))
                    .show_inside(ui, |ui| {
                        crate::host::wasm_render::render_ui_tree_with_surface(
                            ui,
                            &render_tree,
                            &colors,
                            None,
                            None,
                            0,
                        );
                    });
            });
        harness.run();
        harness
            .render()
            .expect("render on-accent tree")
            .save(evidence_png!(format!("_{theme}")))
            .expect("write evidence");
        let mut painted = Vec::new();
        for shape in &harness.output().shapes {
            texts(&shape.shape, &mut painted);
        }
        for (label, fill) in [
            ("Overview", colors.accent),
            ("Save", colors.accent),
            ("3 new", colors.accent),
            ("Delete", colors.danger),
            ("2 failed", colors.danger),
        ] {
            let (_, fg) = painted
                .iter()
                .find(|(text, _)| text == label)
                .unwrap_or_else(|| panic!("{theme}: {label:?} not painted: {painted:?}"));
            let ratio = contrast(luminance(*fg), luminance(fill));
            assert!(
                ratio >= 4.5,
                "{theme}: {label:?} {fg:?} on {fill:?} is {ratio:.2}:1"
            );
            // One rule: white whenever white clears the floor, dark otherwise.
            let white_ok = contrast(1.0, luminance(fill)) >= 4.5;
            if theme == crate::ui::theme::DEFAULT_PRESET && fill == colors.accent {
                assert!(
                    white_ok,
                    "default accent {fill:?} must take white text at 4.5:1"
                );
            }
            assert_eq!(
                luminance(*fg) > luminance(fill),
                white_ok,
                "{theme}: {label:?} painted {fg:?} on {fill:?}; light expected: {white_ok}"
            );
        }
    }
}
#[test]
fn screenshot_foreground_buttons_and_badges() {
    for theme in ["plexi-dusk", "plexi-dawn"] {
        for scale in [1.0, 2.0] {
            let mut h = PlexiUiHarness::new_sized_scaled(1280.0, 1000.0, scale);
            apply_fixture_theme(&mut h, theme);
            add_focused_pane(&mut h);
            h.with_app_mut(|app| app.show_ui_gallery = true);
            h.run_steps(4);
            for section in ["Buttons and chips", "Rows", "Sidebar rows"] {
                h.harness().get_by_label(section).scroll_to_me();
                h.run_steps(4);
                h.save_screenshot(&evidence_png!(&format!("_{theme}_{scale}_{section}")))
                    .unwrap();
            }
        }
    }
}

/// What `NaturalWidthFixture` painted: the width `KitButton::natural_width`
/// promised, the frame it got, and the rows its label was laid out in.
#[derive(Clone, Default)]
struct NaturalWidthSnapshot {
    natural: f32,
    frame: f32,
    rows: Vec<String>,
}

/// A wrapping kit button in a slot exactly `natural_width` wide.
struct NaturalWidthFixture {
    label: &'static str,
    snapshot: Arc<std::sync::Mutex<NaturalWidthSnapshot>>,
}

impl crate::app::view_trait::View for NaturalWidthFixture {
    fn type_id(&self) -> &'static str {
        "natural_width_fixture"
    }
    fn display_name(&self) -> String {
        "Button natural width".into()
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &crate::app::view_trait::ViewRenderContext<'_>,
    ) {
        use crate::ui::button::KitButton;
        let layer = ui.layer_id();
        let start = ui
            .ctx()
            .graphics(|g| g.get(layer).map_or(0, |p| p.all_entries().len()));
        let natural = KitButton::new(self.label, ctx.colors).natural_width(ui);
        let frame = ui
            .allocate_ui_with_layout(
                egui::vec2(natural, crate::ui::style::BUTTON_H_MD),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.add(
                        KitButton::new(self.label, ctx.colors)
                            .wrap_mode(egui::TextWrapMode::Wrap),
                    )
                    .rect
                    .width()
                },
            )
            .inner;
        let label = self.label;
        let rows = ui.ctx().graphics(|g| {
            g.get(layer).and_then(|list| {
                list.all_entries().skip(start).find_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        if text.galley.job.text == label {
                            return Some(
                                text.galley.rows.iter().map(|row| row.row.text()).collect(),
                            );
                        }
                    }
                    None
                })
            })
        });
        if let Some(rows) = rows {
            *self.snapshot.lock().unwrap() = NaturalWidthSnapshot {
                natural,
                frame,
                rows,
            };
        }
    }
}

/// `KitButton::natural_width` is the one width measurement: under the host
/// style a wrapping button given exactly that width keeps its label on one
/// line and its frame fills the width, less the single round-up. The frame
/// stroke is not extra: egui takes the style's 1px widget stroke out of
/// the padding, so a kit button is its label plus its padding (the premise
/// of stint 1356 holds only under bare egui, which has no such stroke).
#[test]
fn kit_button_in_its_natural_width_keeps_the_label_on_one_line() {
    for (case, label) in ["Reset", "Restore deleted note", "Friday"]
        .into_iter()
        .enumerate()
    {
        let mut h = PlexiUiHarness::new_sized(720.0, 420.0);
        let pane_id = add_focused_pane(&mut h);
        let snapshot = Arc::new(std::sync::Mutex::new(NaturalWidthSnapshot::default()));
        h.with_app_mut(|app| {
            if let Some(Pane::View(pane)) =
                app.windows[app.active_window].panes.get_mut(&pane_id)
            {
                pane.runtime = ViewRuntime::Builtin(Box::new(NaturalWidthFixture {
                    label,
                    snapshot: snapshot.clone(),
                }));
            }
        });
        h.run_steps(3);
        let painted = snapshot.lock().unwrap().clone();
        assert_eq!(
            painted.rows,
            vec![label.to_string()],
            "{label:?} must stay on one line in its natural width {}",
            painted.natural
        );
        assert!(
            painted.frame <= painted.natural && painted.natural - painted.frame < 1.0,
            "{label:?}: frame is {} wide but natural_width promised {} (rounded up once)",
            painted.frame,
            painted.natural
        );
        h.save_screenshot(&evidence_png!(&format!("_{case}")))
            .expect("natural width screenshot");
    }
}

#[derive(Clone)]
struct PressSnapshot {
    fill: egui::Color32,
    rect: egui::Rect,
    clicks: usize,
}

struct PressFixture {
    kind: u8,
    snapshot: Arc<std::sync::Mutex<PressSnapshot>>,
}

impl crate::app::view_trait::View for PressFixture {
    fn type_id(&self) -> &'static str {
        "press_fixture"
    }
    fn display_name(&self) -> String {
        "Button press feedback".into()
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &crate::app::view_trait::ViewRenderContext<'_>,
    ) {
        let layer = ui.layer_id();
        let start = ui
            .ctx()
            .graphics(|g| g.get(layer).map_or(0, |p| p.all_entries().len()));
        let clicked = match self.kind {
            0 => ui
                .add(
                    crate::ui::button::KitButton::new("Confirm changes", ctx.colors)
                        .kind(crate::ui::button::ButtonKind::Primary)
                        .min_width(160.0),
                )
                .clicked(),
            1 => {
                crate::ui::button::choice_button(ui, "Keep this version", "1", true, ctx.colors)
                    .clicked()
            }
            _ => {
                use crate::host::wasm_view::bindings::plexi::platform::types::ButtonNode;
                use crate::host::wasm_view::{ButtonStyle, IndexedNode, UiNodeData, UiTree};
                let tree = UiTree {
                    root: 0,
                    nodes: vec![IndexedNode {
                        id: 0,
                        key: "press".into(),
                        data: UiNodeData::Button(ButtonNode {
                            label: "Save document".into(),
                            on_click: "save".into(),
                            style: ButtonStyle::Primary,
                            disabled: false,
                        }),
                    }],
                };
                !crate::host::wasm_render::render_ui_tree_with_surface(
                    ui, &tree, ctx.colors, None, None, 0,
                )
                .actions
                .is_empty()
            }
        };
        let painted = ui.ctx().graphics(|g| {
            g.get(layer).and_then(|list| {
                list.all_entries().skip(start).find_map(|shape| {
                    if let egui::Shape::Rect(rect) = &shape.shape {
                        // The plugin background spans the pane; the button has rounded corners.
                        if rect.corner_radius != egui::CornerRadius::ZERO
                            && rect.fill != egui::Color32::TRANSPARENT
                        {
                            return Some((rect.rect, rect.fill));
                        }
                    }
                    None
                })
            })
        });
        if let Some((rect, fill)) = painted {
            let mut snapshot = self.snapshot.lock().unwrap();
            snapshot.rect = rect;
            snapshot.fill = fill;
            snapshot.clicks += usize::from(clicked);
        }
    }
}

#[test]
fn button_pointer_down_paints_active_before_release() {
    for kind in 0..3 {
        let mut h = PlexiUiHarness::new_sized(720.0, 420.0);
        let pane_id = add_focused_pane(&mut h);
        let snapshot = Arc::new(std::sync::Mutex::new(PressSnapshot {
            fill: egui::Color32::TRANSPARENT,
            rect: egui::Rect::NOTHING,
            clicks: 0,
        }));
        h.with_app_mut(|app| {
            if let Some(Pane::View(pane)) =
                app.windows[app.active_window].panes.get_mut(&pane_id)
            {
                pane.runtime = ViewRuntime::Builtin(Box::new(PressFixture {
                    kind,
                    snapshot: snapshot.clone(),
                }));
            }
        });
        h.run_steps(3);
        let idle = snapshot.lock().unwrap().clone();
        assert!(idle.rect.is_positive(), "kind={kind}");
        let pos = idle.rect.center();
        h.harness().input_mut().events.extend([
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        h.run_steps(1);
        let pressed = snapshot.lock().unwrap().clone();
        assert_eq!(pressed.clicks, 0, "press must not activate kind={kind}");
        assert_ne!(pressed.fill, idle.fill, "active fill kind={kind}");
        assert!(
            (pressed.rect.width() / idle.rect.width() - 0.97).abs() < 0.001,
            "kind={kind}: {:?} -> {:?}",
            idle.rect,
            pressed.rect
        );
        assert!((pressed.rect.height() / idle.rect.height() - 0.97).abs() < 0.001);
        assert!(pressed.rect.center().distance(idle.rect.center()) < 0.01);
        h.save_screenshot(&evidence_png!(&format!("_{kind}")))
            .expect("press screenshot");
        h.run_steps(2);
        assert_eq!(snapshot.lock().unwrap().clicks, 0);
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        h.run_steps(1);
        let released = snapshot.lock().unwrap().clone();
        assert_eq!(
            released.clicks, 1,
            "release activates exactly once kind={kind}"
        );
        assert!((released.rect.width() - idle.rect.width()).abs() < 0.01);
    }
}

#[test]
fn screenshot_micro_motion_floating_shells() {
    let mut h = PlexiUiHarness::new_sized(1100.0, 900.0);
    add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        app.show_ui_gallery = true;
        app.quit_press_count = 1;
        app.quit_last_press = None;
    });
    h.run_steps(3);
    assert!(h.harness().query_by_label("Host UI Gallery").is_some());
    let quit_toast = format!(
        "{} pressed 1 of 3 — press again to quit",
        crate::ui::shortcuts::key_text(&["cmd", "Q"])
    );
    assert!(h.harness().query_by_label(&quit_toast).is_some());
    h.save_screenshot(&evidence_png!())
        .expect("floating shells screenshot");
}

/// Stint 1645: the metadata chip sits at the row's right edge and the
/// meta text right-aligns just left of it. Chips of different widths
/// share one reserved lane, so every meta label keeps one right edge, and
/// a long title elides before either slot.
#[test]
fn list_row_meta_sits_left_of_chip_on_one_edge_and_title_elides_first() {
    use crate::ui::{list::ListRow, style};
    const TITLE: &str = "A very long note title that cannot possibly fit beside its age";
    const RESERVE: &[&str] = &["active", "global", "workspace"];
    let rows = [
        (TITLE, "global", "Yesterday"),
        ("Second row", "workspace", "3h"),
        ("Third row", "active", "Nov 12"),
    ];
    let ctx = egui::Context::default();
    crate::ui::theme::setup_fonts(&ctx);
    let colors = crate::ui::theme::Colors::from_config(
        &crate::ui::theme::preset_colors("plexi-dusk").expect("preset"),
    );
    let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
    for (width, selected) in [(300.0, false), (420.0, true)] {
        let mut row_rect = egui::Rect::NOTHING;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(width);
            for (i, (title, chip, age)) in rows.iter().enumerate() {
                row_rect = ListRow::new(title)
                    .secondary("first line of the note")
                    .metadata_chips(std::slice::from_ref(chip))
                    .metadata_reserve(RESERVE)
                    .meta(age)
                    .selected(selected && i == 0)
                    .show(ui, &colors)
                    .response()
                    .rect;
            }
        });
        let mut texts = Vec::new();
        for shape in &output.shapes {
            gallery_text_shapes(&shape.shape, &mut texts);
        }
        let edge = row_rect.right() - style::LIST_ROW_PAD_H - style::SPACE_SM;
        let metas: Vec<_> = rows
            .iter()
            .map(|(_, _, age)| painted_rect(&texts, age))
            .collect();
        for ((_, chip, age), meta) in rows.iter().zip(&metas) {
            let chip_rect = painted_rect(&texts, chip);
            assert!(
                chip_rect.left() >= meta.right() + style::LIST_ROW_GAP - 0.5,
                "chip {chip:?} {chip_rect:?} must sit right of meta {age:?} {meta:?} at {width}"
            );
            assert!(
                chip_rect.right() <= edge + 0.5 && chip_rect.right() >= edge - 8.0,
                "chip {chip:?} {chip_rect:?} at the right edge {edge} at {width}"
            );
            assert!(
                (meta.right() - metas[0].right()).abs() <= 1.0,
                "meta {age:?} {meta:?} shares one right edge with {:?} at {width}",
                metas[0]
            );
        }
        let title = painted_rect(&texts, "A very long");
        let (_, title_text) = texts
            .iter()
            .find(|(_, t)| t.starts_with("A very long"))
            .unwrap();
        assert!(
            title_text.ends_with('…'),
            "title elides at {width}: {title_text:?}"
        );
        assert!(
            title.right() + style::LIST_ROW_GAP <= metas[0].left() + 0.5,
            "title {title:?} must stop before meta {:?} at {width}",
            metas[0]
        );
        let errors: Vec<_> = crate::ui::layout_report::snapshot(&ctx)
            .iter()
            .flat_map(|r| r.violations())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
    }
}

#[test]
fn screenshot_ui_gallery_hint_bar_stays_centered_at_wide_width() {
    let mut h = PlexiUiHarness::new_sized(900.0, 640.0);
    h.step();
    h.with_app_mut(|app| {
        app.show_ui_gallery = true;
    });
    // Two steps: egui Areas spend their first frame on an invisible sizing
    // pass; the modal is visible from the second frame.
    h.run_steps(2);

    // Modal title and the first section header are real ui.label widgets.
    h.harness().get_by_label("Host UI Gallery");
    h.harness().get_by_label("Chrome primitives");
    // With room for every group, the hint bar is one centered row.
    let rows: Vec<f32> = ["palette", "help", "dismiss"]
        .iter()
        .map(|label| h.harness().get_by_label(label).rect().center().y)
        .collect();
    assert!(
        rows.iter().all(|y| (y - rows[0]).abs() < 1.0),
        "hint groups must share one row at 900pt: {rows:?}"
    );
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

#[test]
fn screenshot_ui_gallery_hint_bar_wraps_at_narrow_width() {
    let mut h = PlexiUiHarness::new_sized(290.0, 640.0);
    h.step();
    h.harness().set_size(egui::vec2(290.0, 640.0));
    h.with_app_mut(|app| {
        app.show_ui_gallery = true;
    });
    h.run_steps(2);

    h.harness().get_by_label("Host UI Gallery");
    // Groups wrap onto further rows instead of running past the window.
    let screen = h.with_app(|app| app.ctx.content_rect());
    for label in ["palette", "help", "dismiss"] {
        let rect = h.harness().get_by_label(label).rect();
        assert_inside(label, 1.0, rect, screen);
    }
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

struct GalleryRowsFixture {
    width: f32,
}

impl crate::app::view_trait::View for GalleryRowsFixture {
    fn type_id(&self) -> &'static str {
        "gallery_rows_fixture"
    }
    fn display_name(&self) -> String {
        "Gallery rows".into()
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &crate::app::view_trait::ViewRenderContext<'_>,
    ) {
        ui.set_width(self.width);
        crate::overlays::ui_gallery::gallery_rows(
            ui,
            &crate::overlays::ui_gallery::UiGalleryDataMode::WorstCase.fixture(),
            ctx.colors,
        );
    }
}

#[test]
fn ui_gallery_worst_case_rows() {
    for width in [760.0, 320.0] {
        let mut h = PlexiUiHarness::new_sized(900.0, 1000.0);
        let pane_id = add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            if let Some(Pane::View(pane)) =
                app.windows[app.active_window].panes.get_mut(&pane_id)
            {
                pane.runtime = ViewRuntime::Builtin(Box::new(GalleryRowsFixture { width }));
            }
        });
        h.run_steps(3);
        // Worst-case titles, metadata and actions must elide or stack
        // inside each row at both widths, never paint past it.
        assert_layout_surface(&h, "list");
        h.save_screenshot(&evidence_png!(&format!("_{width}")))
            .expect("all worst-case rows");
    }
}

/// Whether any pixel inside `rect` (points at ppp 1, so pixels) differs
/// between two renders of the same harness size.
fn pixels_differ_in(a: &image::RgbaImage, b: &image::RgbaImage, rect: egui::Rect) -> bool {
    let x1 = (rect.max.x.max(0.0) as u32).min(a.width());
    let y1 = (rect.max.y.max(0.0) as u32).min(a.height());
    let x0 = (rect.min.x.max(0.0) as u32).min(x1);
    let y0 = (rect.min.y.max(0.0) as u32).min(y1);
    (y0..y1).any(|y| (x0..x1).any(|x| a.get_pixel(x, y) != b.get_pixel(x, y)))
}

#[test]
fn ui_gallery_worst_case_actions() {
    use crate::overlays::ui_gallery::UiGalleryDataMode;
    use crate::ui::{button, style, surface};
    for mode in UiGalleryDataMode::ALL {
        let data = mode.fixture();
        for width in [760.0, 320.0, 280.0] {
            for kind in 0..5 {
                let ctx = egui::Context::default();
                crate::ui::theme::setup_fonts(&ctx);
                let _ = ctx.run_ui(Default::default(), |_| {});
                let colors = crate::ui::theme::Colors::from_config(
                    &crate::ui::theme::preset_colors("catppuccin-mocha").unwrap(),
                );
                let mut bounds = egui::Rect::NOTHING;
                let output = ctx.run_ui(Default::default(), |ui| {
                    ui.set_width(width);
                    let response = match kind {
                        0 => button::choice_button(ui, data.detail, "⌘Enter", false, &colors),
                        1 => {
                            ui.add(button::KitButton::new(data.detail, &colors).min_width(80.0))
                        }
                        2 => surface::copyable_command(
                            ui,
                            egui::Id::new("copy"),
                            data.command,
                            &colors,
                        ),
                        3 => surface::compact_copyable_command(
                            ui,
                            egui::Id::new("compact"),
                            data.command,
                            &colors,
                        ),
                        _ => surface::trust_decision_panel(
                            ui,
                            data.names.first().copied().unwrap_or(""),
                            data.detail,
                            surface::TrustTone::Warning,
                            &colors,
                        ),
                    };
                    bounds = response.rect;
                });
                assert!(
                    bounds.width() <= width + 0.1,
                    "action overflow {mode:?} {width} kind={kind}: {bounds:?}"
                );
                if kind < 2 {
                    assert_eq!(
                        bounds.height(),
                        if kind == 0 {
                            style::BUTTON_H_CHOICE
                        } else {
                            style::BUTTON_H_MD
                        }
                    );
                }
                let mut texts = vec![];
                for clipped in &output.shapes {
                    let painted = clipped.shape.visual_bounding_rect();
                    if painted.is_positive() {
                        assert!(
                            bounds.expand(1.0).contains_rect(painted),
                            "paint outside action {mode:?} {width} kind={kind}: {painted:?} in {bounds:?}"
                        );
                    }
                    gallery_text_shapes(&clipped.shape, &mut texts);
                }
                for (i, (rect, text)) in texts.iter().enumerate() {
                    for (other, label) in &texts[i + 1..] {
                        assert!(
                            !rect.shrink(0.5).intersects(other.shrink(0.5)),
                            "action text overlap {mode:?} {width} kind={kind}: {text:?} / {label:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn ui_gallery_worst_case() {
    use crate::overlays::ui_gallery::{gallery_rows, UiGalleryDataMode, UiGalleryPreview};
    for mode in UiGalleryDataMode::ALL {
        for width in [760.0, 320.0, 280.0] {
            // Inspect actual painted galleys, not copies of the layout arithmetic.
            let ctx = egui::Context::default();
            crate::ui::theme::setup_fonts(&ctx);
            let _ = ctx.run_ui(Default::default(), |_| {});
            let colors = crate::ui::theme::Colors::from_config(
                &crate::ui::theme::preset_colors("catppuccin-mocha").unwrap(),
            );
            let mut bounds = egui::Rect::NOTHING;
            let mut height = 0.0;
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 2000.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let start = ui.cursor().min;
                    gallery_rows(ui, &mode.fixture(), &colors);
                    height = ui.cursor().min.y - start.y;
                    bounds = egui::Rect::from_min_size(start, egui::vec2(width, height));
                },
            );
            assert!(
                (height
                    - mode.fixture().names.len() as f32
                        * (crate::ui::style::LIST_ROW_H
                            + crate::ui::style::LIST_ROW_DENSE_H
                                .max(crate::ui::style::BUTTON_H_MD)))
                .abs()
                    < 0.1,
                "row-height blowout: {mode:?} {width}: {height}"
            );
            let mut texts = vec![];
            for shape in &output.shapes {
                gallery_text_shapes(&shape.shape, &mut texts);
            }
            assert_eq!(
                texts
                    .iter()
                    .filter(|(_, text)| text == "Open" || text == "Delete")
                    .count(),
                mode.fixture().names.len() * 2
            );
            for (i, (rect, label)) in texts.iter().enumerate() {
                assert!(
                    bounds.expand(1.0).contains_rect(*rect),
                    "out of bounds {mode:?} {width}: {label:?} {rect:?} in {bounds:?}"
                );
                for (other, other_label) in &texts[i + 1..] {
                    assert!(
                        !rect.shrink(0.5).intersects(other.shrink(0.5)),
                        "text/chip overlap {mode:?} {width}: {label:?} and {other_label:?}"
                    );
                }
            }
        }
        for narrow in [false, true] {
            let mut h = PlexiUiHarness::new_sized(1100.0, 1000.0);
            add_focused_pane(&mut h);
            h.with_app_mut(|app| {
                app.show_ui_gallery = true;
            });
            h.run_steps(3);
            h.harness().get_by_label(mode.label()).click();
            h.run_steps(2);
            if narrow {
                h.harness().get_by_label("Narrow 320px").click();
                h.run_steps(2);
            }
            let preview = h.with_app(|app| {
                app.ctx.data_mut(|d| {
                    d.get_temp::<UiGalleryPreview>(UiGalleryPreview::id())
                        .unwrap()
                })
            });
            assert_eq!(preview.mode, mode);
            assert_eq!(preview.narrow, narrow);
            for label in [
                "Demo",
                "Worst case",
                "Empty",
                "1 item",
                "Full width",
                "Narrow 320px",
            ] {
                assert!(
                    h.harness().get_all_by_label(label).count() > 0,
                    "missing {label}"
                );
            }
            for section in [
                "Code and toast surfaces",
                "Trust and confirmations",
                "Rows",
                "Sidebar rows",
                "Text fields",
                "Buttons and chips",
                "Empty states",
            ] {
                h.harness().get_by_label(section).scroll_to_me();
                h.run_steps(4);
                let mut texts = vec![];
                let mut in_gallery = false;
                for clipped in &h.harness().output().shapes {
                    let mut painted = vec![];
                    gallery_text_shapes(&clipped.shape, &mut painted);
                    in_gallery |= painted.iter().any(|(_, text)| text == "Host UI Gallery");
                    if in_gallery {
                        texts.extend(
                            painted
                                .into_iter()
                                .filter(|(r, _)| clipped.clip_rect.contains_rect(*r)),
                        );
                    }
                }
                assert!(texts.len() > 5, "gallery paint guard must inspect content");
                for (i, (rect, label)) in texts.iter().enumerate() {
                    for (other, other_label) in &texts[i + 1..] {
                        assert!(
                            !rect.shrink(1.0).intersects(other.shrink(1.0)),
                            "gallery overlap {mode:?} narrow={narrow}: {label:?} and {other_label:?}"
                        );
                    }
                }
                if mode == UiGalleryDataMode::WorstCase {
                    h.save_screenshot(&evidence_png!(&format!("_{narrow}_{section}")))
                        .expect("gallery screenshot");
                }
            }
        }
    }
}
#[test]
fn screenshot_ui_kit_adoption() {
    use crate::overlays::ui_gallery::{UiGalleryDataMode, UiGalleryPreview};
    for theme in ["plexi-dusk", "plexi-dawn"] {
        for scale in [1.0, 2.0] {
            let mut h = PlexiUiHarness::new_sized_scaled(720.0, 1000.0, scale);
            apply_fixture_theme(&mut h, theme);
            let root = h.workspace_root().to_path_buf();
            let path = root.join("Release evidence — a deliberately long image name with an important distinguishing suffix 0123456789abcdef.png");
            std::fs::copy(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app-icon.png"),
                &path,
            )
            .unwrap();
            h.open_target(HarnessOpenTarget::Builtin {
                id: "image-viewer",
                cwd: &root,
                args: &[path.display().to_string()],
            })
            .unwrap();
            h.run_steps(4);
            assert_layout_surface(&h, "surface_header");
            h.save_screenshot(&evidence_png!(&format!("_{theme}_{scale}_appbar")))
                .unwrap();
            h.with_app_mut(|app| {
                app.show_ui_gallery = true;
                app.ctx.data_mut(|d| {
                    d.insert_temp(
                        UiGalleryPreview::id(),
                        UiGalleryPreview {
                            mode: UiGalleryDataMode::WorstCase,
                            narrow: true,
                        },
                    )
                });
            });
            h.run_steps(4);
            h.harness().get_by_label("Rows").scroll_to_me();
            h.run_steps(4);
            assert_layout_surface(&h, "list");
            h.save_screenshot(&evidence_png!(&format!("_{theme}_{scale}_list")))
                .unwrap();
            h.with_app_mut(|app| app.show_ui_gallery = false);
            seed_context_close_confirm(&mut h, true);
            assert_layout_surface(&h, "modal_action");
            h.save_screenshot(&evidence_png!(&format!("_{theme}_{scale}_modal")))
                .unwrap();
        }
    }
}

#[test]
fn screenshot_host_ui_gallery_trust_states() {
    let mut h = PlexiUiHarness::new_sized(1280.0, 900.0);
    add_focused_pane(&mut h);
    h.step();
    h.with_app_mut(|app| {
        app.show_ui_gallery = true;
    });
    h.run_steps(2);
    h.harness().get_by_label("Code and toast surfaces");
    h.harness().get_by_label("Shell completions aren't set up.");
    assert_eq!(
        h.harness()
            .get_all_by_label("plexi completions zsh")
            .count(),
        2,
        "both copy surfaces render the demo fixture"
    );
    assert!(
        h.harness()
            .get_all_by_role(egui::accesskit::Role::MultilineTextInput)
            .count()
            > 0,
        "gallery should render the host TextArea primitive"
    );
    assert!(
        h.harness().query_by_label("Open text-entry modal").is_none(),
        "the gallery has no hand-rolled text-entry dialog demo (stint 1561)"
    );
    h.save_screenshot(&evidence_png!()).expect("render failed");
    assert!(
        h.with_app(|app| app.show_ui_gallery),
        "gallery should remain open for screenshot"
    );
    println!("Screenshot saved to {}", evidence_png!());
}

/// Evidence seam for stint 1216: the button gallery at rest, with the Md
/// Primary hovered, pressed, and with keyboard focus on the first control.
/// Writes /tmp/plexi_screenshot_button_states_<theme>_<state>.png.
#[cfg(target_os = "macos")]
#[test]
fn screenshot_button_states() {
    for theme in ["plexi-dusk", "plexi-dawn"] {
        let mut h = PlexiUiHarness::new_sized(1100.0, 640.0);
        apply_fixture_theme(&mut h, theme);
        h.with_app_mut(|app| app.sidebar_visible = false);
        let root = h.workspace_root().to_path_buf();
        h.open_target(HarnessOpenTarget::Builtin {
            id: "button-gallery-fixture",
            cwd: &root,
            args: &[],
        })
        .expect("open button gallery fixture");
        h.run_steps(4);
        let mut primaries: Vec<egui::Rect> = h
            .harness()
            .get_all_by_label("Primary")
            .map(|node| node.rect())
            .collect();
        primaries.sort_by(|a, b| a.top().total_cmp(&b.top()));
        let md_primary = primaries[1];
        let rest = h.render().unwrap();
        rest.save(evidence_png!(format!("_{theme}_rest"))).unwrap();
        let pos = md_primary.center();
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(pos));
        h.run_steps(3);
        let hover = h.render().unwrap();
        hover
            .save(evidence_png!(format!("_{theme}_hover")))
            .unwrap();
        assert!(
            pixels_differ_in(&rest, &hover, md_primary),
            "{theme}: hover must repaint the Md Primary button"
        );
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
        h.run_steps(3);
        let pressed = h.render().unwrap();
        pressed
            .save(evidence_png!(format!("_{theme}_pressed")))
            .unwrap();
        assert!(
            pixels_differ_in(&hover, &pressed, md_primary),
            "{theme}: press must repaint the hovered button"
        );
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerGone);
        h.run_steps(2);
        h.harness().key_press(egui::Key::Tab);
        h.run_steps(3);
        let focus = h.render().unwrap();
        focus
            .save(evidence_png!(format!("_{theme}_focus")))
            .unwrap();
        assert!(
            focus.as_raw() != rest.as_raw(),
            "{theme}: keyboard focus must paint a visible ring"
        );
        // The ring is whole even where the focused button touches the
        // window edge (Tab lands on the toolbar "UI" button first).
        let (focused, accent) = h.with_app(|app| {
            (
                app.ctx
                    .memory(|m| m.focused())
                    .and_then(|id| app.ctx.read_response(id))
                    .map(|r| r.rect)
                    .expect("Tab focused a widget"),
                app.colors.accent,
            )
        });
        let rings: Vec<(egui::Rect, egui::Rect)> = h
            .harness()
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(r)
                    if r.stroke.color == accent
                        && (r.rect.min - focused.min).length() < 0.5
                        && (r.rect.max - focused.max).length() < 0.5 =>
                {
                    let painted = match r.stroke_kind {
                        egui::StrokeKind::Outside => r.rect.expand(r.stroke.width),
                        egui::StrokeKind::Middle => r.rect.expand(r.stroke.width / 2.0),
                        egui::StrokeKind::Inside => r.rect,
                    };
                    Some((painted, clipped.clip_rect))
                }
                _ => None,
            })
            .collect();
        assert_eq!(rings.len(), 1, "{theme}: one ring on the focused button {focused:?}");
        for (ring, clip) in rings {
            assert!(
                clip.contains_rect(ring),
                "{theme}: focus ring {ring:?} is cut by its clip {clip:?}"
            );
        }
    }
}

/// Stint 1460/1462 critic: a dialog's edge buttons keep their whole
/// keyboard-focus ring. The modal's width clamp clips at the content
/// edge, so the 2pt outside ring of a button touching that edge (a
/// left-aligned Cancel, a right-aligned button row, a full-width button)
/// must still fall inside the clip.
#[test]
fn screenshot_dialog_edge_button_focus_ring_is_not_clipped() {
    use crate::ui::button::{ButtonKind, KitButton};
    let colors = crate::ui::theme::Colors::from_config(&crate::ui::theme::preset_colors("plexi-dusk").unwrap());
    for (layout, name) in [
        (egui::Layout::left_to_right(egui::Align::Center), "left"),
        (egui::Layout::right_to_left(egui::Align::Center), "right"),
        (
            egui::Layout::top_down_justified(egui::Align::Center),
            "full",
        ),
    ] {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(640.0, 400.0))
            .build_ui(move |ui| {
                crate::ui::theme::setup_fonts(ui.ctx());
                // Fonts installed this pass bind from the next one.
                if ui.ctx().cumulative_pass_nr() == 0 {
                    return;
                }
                crate::ui::overlay::ModalShell::centered("focus_ring_dialog")
                    .title("Rename")
                    .width(320.0)
                    .show(ui.ctx(), &colors, |ui| {
                        ui.with_layout(layout, |ui| {
                            let response = ui.add(
                                KitButton::new("Cancel", &colors).kind(ButtonKind::Secondary),
                            );
                            response.request_focus();
                        });
                    });
            });
        harness.run_steps(4);
        let rings: Vec<(egui::Rect, egui::Rect)> = harness
            .output()
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(r)
                    if r.stroke.color == colors.accent
                        && r.stroke_kind == egui::StrokeKind::Outside =>
                {
                    Some((r.rect.expand(r.stroke.width), clipped.clip_rect))
                }
                _ => None,
            })
            .collect();
        assert_eq!(rings.len(), 1, "{name}: the focused button paints one ring");
        for (ring, clip) in rings {
            assert!(
                clip.contains_rect(ring),
                "{name}: focus ring {ring:?} is clipped by {clip:?}"
            );
        }
        harness
            .render()
            .expect("render")
            .save(evidence_png!(format!("_{name}")))
            .expect("save");
    }
}

#[test]
fn screenshot_descriptor_kit_dropdown() {
    use egui_kittest::kittest::Queryable;
    for theme in ["plexi-dusk", "plexi-dawn"] {
        let mut h = PlexiUiHarness::new_sized(720.0, 480.0);
        h.with_app_mut(|app| {
            let cfg = crate::ui::theme::preset_colors(theme).unwrap();
            app.colors = crate::ui::theme::Colors::from_config(&cfg);
            crate::ui::theme::setup_style(&app.ctx, &app.colors);
            app.sidebar_visible = false;
        });
        let cwd = h.workspace_root().to_path_buf();
        h.open_target(HarnessOpenTarget::Builtin {
            id: "descriptor-dropdown-fixture",
            cwd: &cwd,
            args: &[],
        })
        .unwrap();
        h.run_steps(4);
        assert_eq!(
            h.harness()
                .get_by_role(egui::accesskit::Role::ComboBox)
                .rect()
                .height(),
            crate::ui::style::BUTTON_H_MD
        );
        h.save_screenshot(&crate::ui_tests::evidence_png!(format!("_{theme}_closed")))
            .unwrap();
        h.harness()
            .get_by_role(egui::accesskit::Role::ComboBox)
            .click();
        h.run_steps(3);
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(egui::pos2(700.0, 460.0)));
        h.run_steps(3);
        h.save_screenshot(&crate::ui_tests::evidence_png!(format!("_{theme}_open")))
            .unwrap();
        h.harness().get_by_label("Staging").hover();
        h.run_steps(3);
        let staging = h.harness().get_by_label("Staging").rect();
        assert_eq!(staging.height(), crate::ui::style::LIST_ROW_DENSE_H);
        h.save_screenshot(&crate::ui_tests::evidence_png!(format!("_{theme}")))
            .unwrap();
        h.harness().get_by_label("Staging").click();
        h.run_steps(3);
        h.harness().get_by_label("Selected environment: Staging");
    }
}
