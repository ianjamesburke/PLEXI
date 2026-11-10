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

#[derive(Clone, Copy, Debug)]
enum MatrixState {
    Idle,
    Hover,
    Pressed,
    Selected,
    Disabled,
}

fn assert_state_stable(mut specimen: impl FnMut(&mut egui::Ui, MatrixState) -> egui::Response) {
    for ppp in [1.0, 1.25, 1.5, 2.0] {
        let host = crate::testing::HostHarness::new();
        let ctx = &host.app.ctx;
        ctx.set_pixels_per_point(ppp);
        let mut idle = egui::Rect::NOTHING;
        let mut time = 0.0;
        for state in [
            MatrixState::Idle,
            MatrixState::Hover,
            MatrixState::Pressed,
            MatrixState::Selected,
            MatrixState::Disabled,
        ] {
            let mut rect = egui::Rect::NOTHING;
            let mut hovered = false;
            let mut down = false;
            let mut paint = Vec::new();
            for frame in 0..3 {
                let mut events = Vec::new();
                if frame > 0 && matches!(state, MatrixState::Hover | MatrixState::Pressed) {
                    events.push(egui::Event::PointerMoved(idle.center()));
                    if frame == 2 && matches!(state, MatrixState::Pressed) {
                        events.push(egui::Event::PointerButton {
                            pos: idle.center(),
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                } else if frame == 0 && idle.is_positive() {
                    events.push(egui::Event::PointerGone);
                    events.push(egui::Event::PointerButton {
                        pos: idle.center(),
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                let _ = ctx.run_ui(
                    egui::RawInput {
                        events,
                        time: Some(time),
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show_inside(ui, |ui| {
                            let layer = ui.layer_id();
                            let start = ui.ctx().graphics(|g| {
                                g.get(layer).map_or(0, |list| list.all_entries().len())
                            });
                            let response = ui
                                .add_enabled_ui(!matches!(state, MatrixState::Disabled), |ui| {
                                    specimen(ui, state)
                                })
                                .inner;
                            rect = response.rect;
                            hovered = response.hovered();
                            down = response.is_pointer_button_down_on();
                            paint = ui.ctx().graphics(|g| {
                                g.get(layer).map_or_else(Vec::new, |list| {
                                    list.all_entries()
                                        .skip(start)
                                        .filter_map(|entry| {
                                            let bounds = entry
                                                .shape
                                                .visual_bounding_rect()
                                                .intersect(entry.clip_rect);
                                            bounds.is_positive().then_some(bounds)
                                        })
                                        .collect()
                                })
                            });
                        });
                    },
                );
                time += 0.1;
            }
            if matches!(state, MatrixState::Idle) {
                idle = rect;
            }
            assert_eq!(rect, idle, "state={state:?} ppp={ppp}");
            for bounds in &paint {
                assert!(
                    rect.expand(0.01).contains_rect(*bounds),
                    "paint outside rect: state={state:?} ppp={ppp} rect={rect:?} paint={bounds:?}"
                );
            }
            if matches!(state, MatrixState::Hover) {
                assert!(hovered, "hover was not injected at ppp={ppp}");
            }
            if matches!(state, MatrixState::Pressed) {
                assert!(down, "press was not injected at ppp={ppp}");
            }
        }
    }
}

#[test]
fn button_state_matrix() {
    let colors = crate::testing::HostHarness::new().app.colors;
    for specimen in crate::overlays::ui_gallery::kit_specimens() {
        assert_state_stable(|ui, state| {
            ui.add(
                KitButton::new(specimen.label, &colors)
                    .kind(specimen.kind)
                    .size(specimen.size)
                    .selected(matches!(state, MatrixState::Selected)),
            )
        });
    }
}

#[test]
fn popup_row_and_menu_item_state_matrix() {
    let colors = crate::testing::HostHarness::new().app.colors;
    assert_state_stable(|ui, state| {
        crate::ui::list::ListRow::new("Workspace")
            .dense()
            .selected(matches!(state, MatrixState::Selected))
            .show(ui, &colors)
            .response()
    });
    assert_state_stable(|ui, _state| crate::ui::button::menu_item(ui, "Rename", &colors));
}

#[test]
#[should_panic(expected = "state=Selected")]
fn state_guard_rejects_size_changing_control() {
    assert_state_stable(|ui, state| {
        let width = if matches!(state, MatrixState::Selected) {
            100.0
        } else {
            80.0
        };
        ui.allocate_response(egui::vec2(width, 30.0), egui::Sense::click())
    });
}
