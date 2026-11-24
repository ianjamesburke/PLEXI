// ── Text-crispness gate (stint 0531) ────────────────────────────────────
//
// Freezes the pixel-grid and typography work (stints 0527–0530) as
// mechanical assertions on the composited host state: plugin-surface
// texture dims scale with ppp (0527), the editor's row metric is
// integer-physical after a real render (0529), chrome labels clear the
// WCAG AA contrast floor (0528), and hand-painted chrome galley origins
// land on the physical pixel grid (0530). Each calls the production code
// path; none re-implements the invariant.
//
// The `paint_surface_header` title+subtitle band is painted only from
// `wasm_render`; its two crispness invariants are frozen below: the
// subtitle color clears the contrast floor on `bg_toolbar`, and its
// `font_medium` galleys snap to the grid.

use super::*;
use crate::app::text_editor_view::TextEditorView;
use crate::host::wasm_pane::{SysinfoStats, WasmPane};
use crate::host::wasm_view::{StateSnapshot, StateStore, WasmView};

/// The two display scales every text surface must stay crisp at: 1.0
/// (integer) and 2.0 (HiDPI, where off-grid galleys and half-resolution
/// plugin surfaces surface as fuzz).
const GATE_PPP: [f32; 2] = [1.0, 2.0];

fn wasm_fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/wasm-fixtures")
        .join(name)
}

/// Reach the first `text-editor` pane in any window and run `f` against
/// it. Returns `None` if no editor pane is open.
fn with_editor<R>(
    h: &mut PlexiUiHarness,
    f: impl FnOnce(&mut TextEditorView) -> R,
) -> Option<R> {
    let mut f = Some(f);
    h.with_app_mut(|app| {
        for win in &mut app.windows {
            for pane in win.panes.values_mut() {
                if let Pane::View(a) = pane {
                    if let ViewRuntime::Builtin(b) = &mut a.runtime {
                        if let Some(ed) = b.as_any_mut().downcast_mut::<TextEditorView>() {
                            return Some((f.take().expect("f consumed once"))(ed));
                        }
                    }
                }
            }
        }
        None
    })
}

// ── Seeding ─────────────────────────────────────────────────────────

/// A Notes pane holding a multi-line document, parked at a fractional
/// mid-scroll offset so unsnapped rows would show off-grid.
fn seed_editor_mid_scroll(h: &mut PlexiUiHarness) {
    let path = h.workspace.path().join("crispness-note.md");
    let mut body = String::from("---\ntitle: \"Pixel Grid\"\n---\n");
    for i in 1..=60 {
        body.push_str(&format!(
            "Line {i:02}: the quick brown fox jumps over the lazy dog 0123456789\n"
        ));
    }
    std::fs::write(&path, &body).expect("seed note");
    h.with_app_mut(|app| {
        let pane_id = app.host.alloc_pane_id();
        let view_pane = ViewPane {
            incarnation: Default::default(),
            pip_status: None,
            id: pane_id,
            runtime: ViewRuntime::Builtin(Box::new(TextEditorView::new_for_test_note(
                path.clone(),
            ))),
            workspace_root: crate::testing::scratch_context_root("ui"),
            permissions: PluginPermissions::builtin(),
            manifest_id: "text-editor".to_string(),
            name: "Text Editor".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            agent: None,
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        };
        let win = &mut app.windows[app.active_window];
        win.panes
            .insert(pane_id, Pane::View(Box::new(view_pane)))
            .unwrap();
        let tile_id = win.tree.tiles.insert_pane(pane_id);
        if win.tree.root.is_none() {
            win.tree.root = Some(tile_id);
        }
        win.focused_pane = Some(tile_id);
    });
    // One pass so the widget lays out (sets line_height / viewport_height),
    // then park at a fractional mid-document offset and render again.
    h.run_steps(2);
    with_editor(h, |ed| {
        let line_height = ed.test_view_line_height().max(1.0);
        ed.test_set_scroll_y(line_height * 18.0 + 0.5);
    });
}

// ── Mechanical assertions on composited state ───────────────────────

/// Stint 0527: a GPU view pane allocates its surface at physical
/// resolution (`logical × ppp`), so at ppp 2.0 the texture is exactly
/// twice the ppp-1.0 texture in each dimension — full-res on HiDPI, no
/// bilinear upscaling. Drives the production `WasmPane` surface path.
#[test]
fn gate_wasm_view_surface_scales_with_ppp() {
    fn surface_at(ppp: f32) -> (u32, u32) {
        let app = WasmView::load_ephemeral_run(
            "pong-crispness",
            &wasm_fixture("pong.wasm"),
            StateStore::ephemeral(),
        )
        .expect("load pong fixture");
        let mut pane = WasmPane::new(app, Box::new(SysinfoStats::new()));
        pane.set_pixels_per_point(ppp);
        pane.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])
            .expect("init pong");
        pane.surface_size().expect("surface allocated after init")
    }
    let (w1, h1) = surface_at(1.0);
    let (w2, h2) = surface_at(2.0);
    assert!(w1 > 0 && h1 > 0, "ppp-1.0 surface must be non-empty");
    assert_eq!(
        (w2, h2),
        (w1 * 2, h1 * 2),
        "surface texture must scale 1:1 with display scale \
                 (ppp1={w1}x{h1}, ppp2={w2}x{h2})"
    );
}

/// Stint 0529: after a real host render the editor's row metric is
/// quantized to the physical pixel grid, so `line_height × ppp` is an
/// integer and every painted row top is uniformly spaced — at both
/// display scales, and mid-scroll.
#[test]
fn gate_editor_row_metric_is_integer_physical() {
    for ppp in GATE_PPP {
        let mut h = PlexiUiHarness::new_sized_ppp(820.0, 560.0, ppp);
        h.step();
        seed_editor_mid_scroll(&mut h);
        h.run_steps(3);
        let line_height =
            with_editor(&mut h, |ed| ed.test_view_line_height()).expect("editor pane open");
        let phys = line_height * ppp;
        assert!(
            line_height > 0.0 && (phys - phys.round()).abs() < 1e-2,
            "quantized line_height {line_height} not integer-physical at ppp {ppp} \
                     (phys {phys})"
        );
        // Uniform, on-grid row tops for the metric the widget committed.
        let view = crate::editor::ViewState {
            line_height,
            ..crate::editor::ViewState::default()
        };
        let expected = (line_height * ppp).round();
        let mut prev = view.line_top(0) * ppp;
        for line in 1..120 {
            let top = view.line_top(line) * ppp;
            let spacing = top - prev;
            assert!(
                (spacing - expected).abs() < 1e-2,
                "row spacing {spacing} != {expected} phys px (ppp {ppp}, line {line})"
            );
            prev = top;
        }
    }
}

/// Stint 0528: the labels the host hand-paints in `text_secondary`
/// (sidebar context names, surface-header subtitles, pane-name bars) clear the
/// WCAG AA 4.5:1 floor against every background they actually sit on —
/// mirroring the contrast check plugins get. Reuses the production WCAG
/// math and the default theme.
#[test]
fn gate_chrome_labels_meet_wcag_aa_floor() {
    use crate::ui::theme::{contrast, luminance};
    let colors =
        crate::ui::theme::colors_from_config(&crate::config::PlexiConfig::default());
    let backgrounds = [
        ("bg_sidebar", colors.bg_sidebar),
        ("bg_toolbar", colors.bg_toolbar),
        ("pane_header_bg", colors.pane_header_bg()),
        ("bg_darkest", colors.bg_darkest),
    ];
    for (name, bg) in backgrounds {
        let fg = colors.text_secondary(bg);
        let ratio = contrast(luminance(fg), luminance(bg));
        assert!(
            ratio >= 4.5,
            "chrome label on {name} has contrast {ratio:.2} < 4.5 AA floor"
        );
    }
}

/// Stint 0530: the hand-painted-text snapping helper lands every galley
/// origin on the physical pixel grid, for the whole UI text size range
/// and from fractional layout positions, at both display scales. Drives
/// the production `snap::text_snapped` path all chrome text routes
/// through.
#[test]
fn gate_chrome_galley_origins_snap_to_physical_grid() {
    for ppp in GATE_PPP {
        let ctx = egui::Context::default();
        // Bind the real UI faces so `font_medium` ("ui-medium") resolves.
        crate::ui::theme::setup_fonts(&ctx);
        ctx.set_pixels_per_point(ppp);
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter();
            for size in 8..=16 {
                let rect = crate::ui::snap::text_snapped(
                    painter,
                    egui::pos2(12.3, 9.7),
                    egui::Align2::LEFT_TOP,
                    "context / subtitle",
                    crate::ui::theme::font_medium(size as f32),
                    egui::Color32::WHITE,
                );
                for coord in [rect.min.x, rect.min.y] {
                    let phys = coord * ppp;
                    assert!(
                        (phys - phys.round()).abs() < 1e-3,
                        "galley origin {coord} off the grid at ppp {ppp} (size {size})"
                    );
                }
            }
        });
    }
}
