use plexi_wasm_sdk::{effects, ui, Alignment, App, Effect, InputEvent, UiNodeData};

#[test]
fn ui_builder_covers_every_node_variant() {
    let mut tree = ui::Tree::new();
    let empty = tree.empty("empty");
    let text = tree.text("text", "Hello");
    let button = tree.button("button", "Run", "run");
    let input = tree.text_input("input", "", "Name", "change", "submit");
    let row = tree.row("row", [text, button]);
    let column = tree.column("column", [input, row]);
    let progress = tree.progress_bar("progress", 2.0, 10.0);
    let badge = tree.badge("badge", "Ready");
    let list = tree.list_view("list", [text], Some(0), Some("select"));
    let scroll = tree.scroll("scroll", list, false);
    let padding = tree.padding("padding", scroll, 4.0);
    let canvas = tree.canvas("canvas", 320.0, 200.0, []);
    let divider = tree.divider("divider");
    let space = tree.space("space", 8.0);
    let surface = tree.surface("surface", 640, 480, None);
    let root = tree.column(
        "root",
        [
            empty, column, progress, badge, padding, canvas, divider, space, surface,
        ],
    );
    let tree = tree.finish(root);

    assert_eq!(tree.nodes.len(), 16);
    assert!(tree
        .nodes
        .iter()
        .any(|node| matches!(node.data, UiNodeData::Surface(_))));
}

#[test]
fn effect_helpers_cover_host_effects() {
    let values = vec![
        effects::file_read("notes.txt"),
        effects::file_write("notes.txt", b"hello"),
        effects::http_fetch("https://example.com"),
        effects::set_timer(1, 100, true),
        effects::cancel_timer(1),
        effects::get_system_stats(),
        effects::set_title("Counter"),
        effects::set_status("Ready"),
        effects::close_self(),
        effects::request_capability("clipboard.write"),
        effects::clipboard_read(),
        effects::clipboard_write("hello"),
        effects::notify("Done", "Counter updated"),
        effects::spawn("files"),
    ];

    assert_eq!(values.len(), 14);
    assert!(matches!(values[6], Effect::SetTitle(_)));
}

#[derive(Default)]
struct Example;

impl App for Example {
    fn update(&mut self, _event: InputEvent) -> Vec<Effect> {
        Vec::new()
    }

    fn view(&self) -> plexi_wasm_sdk::UiTree {
        let mut tree = ui::Tree::new();
        let root = tree.text("root", "example");
        tree.finish(root)
    }
}

#[test]
fn ergonomic_app_trait_has_default_init() {
    let mut app = Example;
    let context = plexi_wasm_sdk::InitContext::new(
        plexi_wasm_sdk::StateSnapshot { entries: vec![] },
        (80.0, 24.0),
        vec![],
    );
    assert!(app.init(context).is_empty());
    assert_eq!(Alignment::Start, Alignment::Start);
}
