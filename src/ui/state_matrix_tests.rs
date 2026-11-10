//! State changes must not change a control's allocated geometry.

use crate::ui::button::{ButtonKind, KitButton};

fn measure(selected: bool, hover: bool, ppp: f32) -> (egui::Rect, Vec<egui::Color32>) {
    let host = crate::testing::HostHarness::new();
    let ctx = &host.app.ctx;
    ctx.set_pixels_per_point(ppp);
    let mut rect = egui::Rect::NOTHING;
    let mut output = None;
    for frame in 0..3 {
        let events = if hover && frame > 0 {
            vec![egui::Event::PointerMoved(rect.center())]
        } else {
            Vec::new()
        };
        output = Some(ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show_inside(ui, |ui| {
                    rect = ui
                        .add(
                            KitButton::new("Option", &host.app.colors)
                                .kind(ButtonKind::Ghost)
                                .selected(selected),
                        )
                        .rect;
                });
            },
        ));
    }
    let fills = output
        .unwrap()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.rect.contains(rect.center()) => Some(r.fill),
            _ => None,
        })
        .collect();
    (rect, fills)
}

#[test]
fn selected_ghost_preserves_size_at_every_scale() {
    for ppp in [1.0, 1.25, 1.5, 2.0] {
        assert_eq!(
            measure(false, false, ppp).0.size(),
            measure(true, false, ppp).0.size(),
            "ppp={ppp}"
        );
    }
}

#[test]
fn ghost_hover_preserves_size_at_every_scale() {
    for ppp in [1.0, 1.25, 1.5, 2.0] {
        assert_eq!(
            measure(false, false, ppp).0.size(),
            measure(false, true, ppp).0.size(),
            "ppp={ppp}"
        );
    }
}

#[test]
fn scope_popup_rows_have_equal_size() {
    let host = crate::testing::HostHarness::new();
    let mut sizes = Vec::new();
    for selected in [false, true] {
        let mut rect = egui::Rect::NOTHING;
        for _ in 0..3 {
            let _ = host.app.ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show_inside(ui, |ui| {
                    rect = crate::ui::list::ListRow::new("Workspace")
                        .dense()
                        .selected(selected)
                        .show(ui, &host.app.colors)
                        .rect();
                });
            });
        }
        sizes.push(rect.size());
    }
    assert_eq!(
        sizes[0], sizes[1],
        "scope rows must not resize on selection"
    );
}
