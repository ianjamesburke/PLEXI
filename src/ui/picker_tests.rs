//! Stint 1391: edges of the picker's input adapter. The ordered whole-path
//! property lives in `app::tests::command_palette_oracle_tests`.

use super::*;
use crate::host::picker::{PickerEffect, PickerRow, RowAction, PALETTE_STOPS};

fn row_rect(index: usize) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(10.0, 100.0 + 20.0 * index as f32),
        egui::vec2(200.0, 18.0),
    )
}

fn picker_row(id: &str) -> PickerRow {
    PickerRow {
        key: RowKey::new("commands", id),
        title: id.to_string(),
        secondary: String::new(),
        chips: Vec::new(),
        group: "",
        group_rank: 0,
        score: None,
        pinned: false,
        unavailable: None,
        actions: vec![RowAction {
            id: "run",
            label: "run",
        }],
    }
}

/// The field painted by the production painter, parked away from the rows.
fn field_geometry() -> FieldGeometry {
    let ctx = egui::Context::default();
    let mut geometry = None;
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        geometry = Some(OrderedLineField::new("").show(
            ui,
            egui::Id::new("edge_field"),
            "",
            crate::ui::focus::SurfaceKey::Overlay(crate::app::input_owner::OverlaySurface::Layer(
                crate::app::FocusKind::CommandPalette,
            )),
            &Colors::from_config(&crate::config::ThemeConfig::default()),
        ));
    });
    let mut geometry = geometry.expect("field painted");
    geometry.rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(200.0, 24.0));
    geometry
}

/// An open picker answered with rows a, b, c, presented at `row_rect`.
struct Fixture {
    input: PickerInput,
    state: PickerState,
    activations: Vec<String>,
}

impl Fixture {
    fn new() -> Self {
        let mut fixture = Self {
            input: PickerInput::default(),
            state: PickerState::new(PALETTE_STOPS),
            activations: Vec::new(),
        };
        fixture.apply(PickerCommand::Open {
            stop: PALETTE_STOPS[0].id,
            query: String::new(),
        });
        let generation = fixture.state.generation();
        fixture.apply(PickerCommand::Rows {
            generation,
            rows: ["a", "b", "c"].iter().map(|id| picker_row(id)).collect(),
            done: true,
        });
        fixture.input.present(Presented {
            stamp: fixture.state.rows_stamp(),
            rows: (0..3)
                .map(|i| (row_rect(i), fixture.state.rows()[i].key.clone()))
                .collect(),
            modal: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0)),
            field: field_geometry(),
        });
        fixture
    }

    fn apply(&mut self, command: PickerCommand) {
        for effect in self.state.apply(command) {
            if let PickerEffect::Activate { row, .. } = effect {
                self.activations.push(row.stable_id().to_owned());
            }
        }
    }

    fn send(&mut self, events: Vec<egui::Event>) {
        for event in events {
            if let Routed::Command(command) = self.input.route(&event, &mut |_| {}) {
                self.apply(command);
            }
        }
    }
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// Review round 3: a press that leaves the click slop is a drag, even when
/// it returns to its row before release.
#[test]
fn command_palette_drag_out_and_back_is_no_click() {
    let mut fixture = Fixture::new();
    let b = row_rect(1).center();
    fixture.send(vec![
        button(b, true),
        egui::Event::PointerMoved(b + egui::vec2(0.0, 40.0)),
        egui::Event::PointerMoved(b),
        button(b, false),
    ]);
    assert!(fixture.activations.is_empty(), "a drag never clicks");
    assert!(fixture.state.is_open());
    fixture.send(vec![button(b, true), button(b, false)]);
    assert_eq!(
        fixture.activations,
        vec!["b"],
        "a still press and release clicks"
    );
}

/// Review round 3: a native macOS Cmd chord sets `mac_cmd` too; Cmd+J and
/// Cmd+K still move.
#[test]
fn command_palette_native_command_chords_move() {
    let mut fixture = Fixture::new();
    let native_cmd = egui::Modifiers {
        command: true,
        mac_cmd: cfg!(target_os = "macos"),
        ctrl: !cfg!(target_os = "macos"),
        ..Default::default()
    };
    fixture.send(vec![
        key(egui::Key::J, native_cmd),
        key(egui::Key::J, native_cmd),
        key(egui::Key::K, native_cmd),
        key(egui::Key::Enter, egui::Modifiers::NONE),
    ]);
    assert_eq!(fixture.activations, vec!["b"]);
}

/// A scrim click closes at its release; an Enter during IME composition
/// confirms the composition, never the row.
#[test]
fn command_palette_scrim_and_ime_edges() {
    let mut fixture = Fixture::new();
    fixture.send(vec![
        egui::Event::Ime(egui::ImeEvent::Enabled),
        egui::Event::Ime(egui::ImeEvent::Preedit("al".into())),
        key(egui::Key::Enter, egui::Modifiers::NONE),
    ]);
    assert!(
        fixture.activations.is_empty(),
        "Enter confirmed the composition"
    );
    assert_eq!(fixture.state.query(), "", "preedit is never committed text");

    let outside = egui::pos2(600.0, 600.0);
    fixture.send(vec![button(outside, true)]);
    assert!(fixture.state.is_open(), "the press alone keeps it open");
    fixture.send(vec![button(outside, false)]);
    assert!(
        !fixture.state.is_open(),
        "the scrim click closed the palette"
    );
    assert!(fixture.activations.is_empty());
}
