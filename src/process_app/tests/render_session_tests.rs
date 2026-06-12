use super::super::*;
use std::collections::HashSet;
use std::path::PathBuf;

fn make_app() -> Option<ProcessApp> {
    let sh = ["/bin/sh", "/usr/bin/sh"]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(PathBuf::from)?;
    let workspace_root = std::env::temp_dir();
    ProcessApp::launch(
        "test_render_session",
        "Test RenderSession",
        &sh,
        &workspace_root,
        &["-c".to_string(), "sleep 1".to_string()],
        workspace_root.clone(),
        HashSet::new(),
        false,
        None,
    )
    .ok()
}

#[test]
fn render_session_submit_produces_event() {
    let Some(mut app) = make_app() else {
        eprintln!("skipping: no /bin/sh available");
        return;
    };
    app.render_session
        .text_input_buffers
        .insert("x".to_string(), "hello".to_string());
    app.submit_text_input("x");
    let evt = app.outbound_events.pop_back().expect("event queued");
    match evt {
        crate::app_protocol::PlexiEvent::TextSubmitted { id, value } => {
            assert_eq!(id, "x");
            assert_eq!(value, "hello");
        }
        other => panic!("expected TextSubmitted, got {other:?}"),
    }
}

/// Recursively collect painted galley texts from an egui shape tree.
fn collect_shape_texts(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(ts) => out.push(ts.galley.text().to_string()),
        egui::Shape::Vec(v) => {
            for s in v {
                collect_shape_texts(s, out);
            }
        }
        _ => {}
    }
}

/// Allocation-reduction regression test (#2024): a PGAP frame with many list
/// rows, elided/wrapped text, a TextInput, and a ComponentTree containing a
/// TextEdit and a Raw text node must render identically across frames and
/// emit no spurious events. Exercises:
/// - ListView primary-text galley reuse + elision
/// - Text `max_width`/`elide` and `max_lines` truncation paths
/// - TextInput visibility scratch-set rotation
/// - Raw-node persistent cache threading (`RawNodeCaches`)
/// - TextEdit buffer seeding without per-frame key clones
#[test]
fn render_session_pgap_frame_stable_output_no_spurious_events() {
    use crate::app_protocol::{
        ListViewItem, ListViewRowDescriptor, PlexiEvent, RenderCommand, UiNode,
    };
    use crate::protocol::commands::TextAlign;

    let ctx = egui::Context::default();
    ctx.set_fonts(egui::FontDefinitions::default());
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));

    let long: String = "very long row text segment ".repeat(20);
    let items: Vec<ListViewItem> = (0..50)
        .map(|i| {
            ListViewItem::Row(ListViewRowDescriptor {
                id: format!("row-{i}"),
                leading: None,
                primary: format!("Row title {i} {long}"),
                secondary: Some(format!("secondary {i}")),
                chips: vec![],
                trailing: None,
            })
        })
        .collect();

    let text_cmd = |y: f32, text: String, max_width: Option<f32>, max_lines: Option<u32>| {
        RenderCommand::Text {
            x: 0.0,
            y,
            text,
            size: 13.0,
            color: "#ffffff".to_string(),
            monospace: false,
            bold: false,
            align: TextAlign::LeftTop,
            max_width,
            elide: true,
            selectable: false,
            max_lines,
        }
    };

    let frame = vec![
        RenderCommand::ListView {
            id: "lv".to_string(),
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 400.0,
            items,
            selected: 3,
            loading: false,
            error: None,
        },
        // Overflowing single-line text → max_width elision path.
        text_cmd(420.0, long.clone(), Some(200.0), None),
        // Overflowing wrapped text → max_lines truncation path.
        text_cmd(440.0, long.clone(), None, Some(2)),
        // Fitting text → galley-reuse fast path.
        text_cmd(560.0, "short fits".to_string(), Some(400.0), None),
        RenderCommand::TextInput {
            id: "ti-1".to_string(),
            x: 0.0,
            y: 580.0,
            w: 200.0,
            h: 24.0,
            placeholder: "type here".to_string(),
            multiline: false,
            value: None,
        },
        RenderCommand::ComponentTree {
            root: UiNode::Column {
                children: vec![
                    UiNode::TextEdit {
                        node_id: "te-1".to_string(),
                        placeholder: "edit".to_string(),
                        value: "seeded value".to_string(),
                        multiline: false,
                        max_length: 0,
                    },
                    UiNode::Raw {
                        command: Box::new(text_cmd(
                            0.0,
                            "raw node text".to_string(),
                            None,
                            None,
                        )),
                    },
                ],
                gap: 4.0,
                padding_top: 0.0,
                padding: 0.0,
            },
        },
    ];

    let mut session = render_session::RenderSession::new();
    let colors = crate::ui::theme::Colors::from_config(&Default::default());
    let mut cm_cache = egui_commonmark::CommonMarkCache::default();
    let peaks: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
    let mut img_cache = image_cache::ImageCache::new();
    let app_dir = std::env::temp_dir();

    let mut texts_per_pass: Vec<Vec<String>> = Vec::new();
    for pass in 0..2 {
        let raw_input = egui::RawInput {
            screen_rect: Some(rect),
            ..Default::default()
        };
        let full_output = ctx.run(raw_input, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| {
                    session.render(
                        ui,
                        rect,
                        &frame,
                        &colors,
                        &mut cm_cache,
                        &peaks,
                        1,
                        &mut img_cache,
                        &app_dir,
                        false,
                        false,
                    );
                });
        });

        // A static frame with no user input must emit no events.
        let events: Vec<PlexiEvent> = session.drain_events().collect();
        assert!(
            events.is_empty(),
            "pass {pass}: expected no events from a static frame, got {events:?}"
        );

        let mut texts = Vec::new();
        for cs in &full_output.shapes {
            collect_shape_texts(&cs.shape, &mut texts);
        }
        texts_per_pass.push(texts);
    }

    let texts = &texts_per_pass[0];
    // Selected list row title is painted and elided (never the full string).
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("Row title 3") && t.ends_with('…')),
        "expected elided 'Row title 3 …' in painted texts"
    );
    assert!(
        !texts.iter().any(|t| t == &format!("Row title 3 {long}")),
        "overflowing row title must not paint at full length"
    );
    // Fitting text paints verbatim via the reused galley.
    assert!(
        texts.iter().any(|t| t == "short fits"),
        "fitting text must paint unmodified"
    );
    // max_lines text is truncated with an ellipsis.
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("very long row text segment")
                && t.ends_with('…')
                && t.len() < long.len()),
        "max_lines text must be truncated with an ellipsis"
    );
    // Raw node rendered through the threaded persistent caches.
    assert!(
        texts.iter().any(|t| t == "raw node text"),
        "Raw node text must render via threaded caches"
    );

    // TextEdit buffer seeded once from the app value (no per-frame reseed).
    assert_eq!(
        session.text_edit_buffers.get("te-1").map(String::as_str),
        Some("seeded value"),
        "TextEdit buffer must be seeded from the app value"
    );

    // Painted output must be identical across frames — cache/scratch reuse
    // must not change what is rendered.
    assert_eq!(
        texts_per_pass[0], texts_per_pass[1],
        "painted text output must be stable across frames"
    );
}

#[test]
fn render_session_process_app_has_no_text_input_fields() {
    // Compile-time proof: ProcessApp::render_session owns the state.
    // This test just exercises the field path — if mod.rs still had
    // text_input_buffers directly on ProcessApp this wouldn't compile.
    let Some(mut app) = make_app() else {
        return;
    };
    app.render_session
        .text_input_buffers
        .insert("k".to_string(), "v".to_string());
    assert!(app.render_session.text_input_buffers.contains_key("k"));
}
