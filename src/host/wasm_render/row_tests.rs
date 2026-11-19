//! Rows are self-describing nodes (stint 1636). The trees are the golden
//! fixtures the Python SDK's `tests/test_row_nodes.py` pins its encoding to,
//! so a press here exercises exactly what an app emits.

use super::*;

const TABLE_HEADERLESS: &str =
    include_str!("../../../sdk/python/tests/golden/rows/table_headerless.json");
const TABLE_HEADERED: &str =
    include_str!("../../../sdk/python/tests/golden/rows/table_headered.json");
const LIST_BUTTON_ROW: &str =
    include_str!("../../../sdk/python/tests/golden/rows/list_button_row.json");
const LIST_BUTTON_ROW_NO_HANDLER: &str =
    include_str!("../../../sdk/python/tests/golden/rows/list_button_row_no_handler.json");

fn decode(json: &str) -> UiTree {
    crate::host::wasm_python::decode_ui_tree(json).expect("golden tree decodes")
}

fn screen() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 320.0))
}

struct Frame {
    out: RenderResult,
    shapes: Vec<egui::epaint::ClippedShape>,
}

fn run_frame(ctx: &egui::Context, tree: &UiTree, events: Vec<egui::Event>) -> Frame {
    let colors =
        Colors::from_config(&crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"));
    let mut out = RenderResult::default();
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(screen()),
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show_inside(ui, |ui| {
                    out = render_ui_tree_into(
                        ui,
                        tree,
                        &colors,
                        None,
                        None,
                        None,
                        None,
                        None,
                        0,
                        RenderResult {
                            node_paint: Some(Vec::new()),
                            ..RenderResult::default()
                        },
                    );
                });
        },
    );
    Frame {
        out,
        shapes: output.shapes,
    }
}

fn new_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    crate::ui::theme::setup_fonts(&ctx);
    // Fonts bind on the next frame; lay nothing out until they are live.
    let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
    ctx
}

/// Actions a real pointer press and release at `pos` delivers, after a
/// warm-up frame so egui hit-tests against laid-out widgets.
fn press_at(tree: &UiTree, pos: impl Fn(&Frame) -> egui::Pos2) -> Vec<String> {
    let ctx = new_ctx();
    let warm = run_frame(&ctx, tree, Vec::new());
    let _ = run_frame(&ctx, tree, Vec::new());
    let at = pos(&warm);
    let events = vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        },
        egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        },
    ];
    run_frame(&ctx, tree, events).out.actions
}

/// Actions `plexi pane click --node <ref>` delivers for node `id`: the
/// activation the host derives from the tree (`node_ref::activation`).
fn node_click(tree: &UiTree, id: u32) -> Vec<String> {
    match crate::host::node_ref::activation(tree.root, &tree.nodes, id) {
        Ok(crate::host::node_ref::Activation::Action(action)) => vec![action],
        Ok(crate::host::node_ref::Activation::Focus(_)) | Err(_) => Vec::new(),
    }
}

fn painted_text_center(frame: &Frame, label: &str) -> egui::Pos2 {
    fn find(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, label)),
            _ => None,
        }
    }
    frame
        .shapes
        .iter()
        .find_map(|clipped| find(&clipped.shape, label))
        .unwrap_or_else(|| panic!("no painted text {label:?}"))
        .center()
}

fn node_rect(frame: &Frame, id: u32) -> egui::Rect {
    frame
        .out
        .node_layout
        .as_ref()
        .expect("layout recorded")
        .iter()
        .find(|fact| fact.id == id)
        .and_then(|fact| fact.rect)
        .unwrap_or_else(|| panic!("node {id} painted nothing"))
}

fn node_id_of(tree: &UiTree, pred: impl Fn(&UiNodeData) -> bool) -> u32 {
    tree.nodes
        .iter()
        .find(|node| pred(&node.data))
        .map(|node| node.id)
        .expect("node present")
}

fn table_row_id(tree: &UiTree, cell: &str) -> u32 {
    node_id_of(
        tree,
        |data| matches!(data, UiNodeData::TableRow(row) if row.cells == [cell.to_string()]),
    )
}

#[test]
fn table_press_on_painted_b_and_node_click_on_b_emit_pick_1() {
    for (name, json) in [
        ("headerless", TABLE_HEADERLESS),
        ("headered", TABLE_HEADERED),
    ] {
        let tree = decode(json);
        assert_eq!(
            press_at(&tree, |frame| painted_text_center(frame, "B")),
            vec!["pick:1".to_string()],
            "{name}: press on painted B"
        );
        assert_eq!(
            node_click(&tree, table_row_id(&tree, "B")),
            vec!["pick:1".to_string()],
            "{name}: --node on B's row"
        );
        assert_eq!(
            node_click(&tree, table_row_id(&tree, "A")),
            vec!["pick:0".to_string()],
            "{name}: --node on A's row"
        );
    }
}

#[test]
fn button_inside_list_row_emits_its_own_action_and_background_emits_the_row() {
    let tree = decode(LIST_BUTTON_ROW);
    let button = node_id_of(&tree, |data| matches!(data, UiNodeData::Button(_)));
    let row = node_id_of(&tree, |data| matches!(data, UiNodeData::ListItem(_)));
    assert_eq!(
        press_at(&tree, |frame| node_rect(frame, button).center()),
        vec!["run".to_string()],
        "press on the embedded button"
    );
    assert_eq!(
        press_at(&tree, |frame| {
            let rect = node_rect(frame, row);
            egui::pos2(rect.right() - 4.0, rect.center().y)
        }),
        vec!["pick:0".to_string()],
        "press on the row background"
    );
    assert_eq!(node_click(&tree, row), vec!["pick:0".to_string()]);
}

#[test]
fn list_row_without_handler_leaves_its_button_working_and_background_inert() {
    let tree = decode(LIST_BUTTON_ROW_NO_HANDLER);
    let button = node_id_of(&tree, |data| matches!(data, UiNodeData::Button(_)));
    let row = node_id_of(&tree, |data| matches!(data, UiNodeData::ListItem(_)));
    assert_eq!(
        press_at(&tree, |frame| node_rect(frame, button).center()),
        vec!["run".to_string()]
    );
    assert!(press_at(&tree, |frame| {
        let rect = node_rect(frame, row);
        egui::pos2(rect.right() - 4.0, rect.center().y)
    })
    .is_empty());
}

#[test]
fn a_row_outside_its_container_still_delivers_its_own_action() {
    let tree = crate::host::wasm_python::decode_ui_tree(
        r#"{"root":0,"nodes":[
            {"id":0,"key":"0","data":{"type":"Column","children":[1,3]}},
            {"id":1,"key":"0/0","data":{"type":"ListItem","child":2,"selected":false,"on_select":"item"}},
            {"id":2,"key":"0/0/0","data":{"type":"Text","text":"orphan item"}},
            {"id":3,"key":"0/1","data":{"type":"TableRow","cells":["orphan row"],"selected":false,"on_select":"row"}}
        ]}"#,
    )
    .expect("decode");
    assert_eq!(node_click(&tree, 1), vec!["item".to_string()]);
    assert_eq!(node_click(&tree, 3), vec!["row".to_string()]);
}
