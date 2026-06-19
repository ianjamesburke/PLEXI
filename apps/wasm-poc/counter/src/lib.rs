// Counter — typed-node interaction POC for the Plexi WASM runtime.

wit_bindgen::generate!({
    world: "plexi-app",
    path: "wit/world.wit",
});

use exports::plexi::platform::lifecycle::Guest;
use plexi::platform::host_log;
use plexi::platform::types::{
    Alignment, ButtonNode, ButtonStyle, ColumnNode, Effect, IndexedNode, InputEvent, StateSnapshot,
    TextNode, UiNodeData, UiTree,
};

const INCREMENT_HANDLER: &str = "increment";

struct Counter {
    count: u32,
    next_id: u32,
}

impl Counter {
    fn new() -> Self {
        Counter {
            count: 0,
            next_id: 0,
        }
    }

    fn node(&mut self, key: &str, data: UiNodeData) -> IndexedNode {
        let id = self.next_id;
        self.next_id += 1;
        IndexedNode {
            id,
            key: key.to_string(),
            data,
        }
    }

    fn handle(&mut self, event: InputEvent) -> Vec<Effect> {
        match event {
            InputEvent::UiAction(action) if action.handler_id == INCREMENT_HANDLER => {
                self.count += 1;
                vec![]
            }
            _ => vec![],
        }
    }

    fn build_tree(&mut self) -> UiTree {
        self.next_id = 0;
        let mut nodes = Vec::new();

        let title = self.node(
            "title",
            UiNodeData::Text(TextNode {
                text: "WASM Counter".to_string(),
                size: Some(18.0),
                bold: true,
                color: None,
                truncate: false,
                align: Alignment::Start,
            }),
        );
        let title_id = title.id;
        nodes.push(title);

        let count = self.node(
            "count",
            UiNodeData::Text(TextNode {
                text: format!("Count: {}", self.count),
                size: Some(14.0),
                bold: false,
                color: None,
                truncate: false,
                align: Alignment::Start,
            }),
        );
        let count_id = count.id;
        nodes.push(count);

        let button = self.node(
            "increment",
            UiNodeData::Button(ButtonNode {
                label: "Increment".to_string(),
                on_click: INCREMENT_HANDLER.to_string(),
                style: ButtonStyle::Primary,
                disabled: false,
            }),
        );
        let button_id = button.id;
        nodes.push(button);

        let root = self.node(
            "root",
            UiNodeData::Column(ColumnNode {
                children: vec![title_id, count_id, button_id],
                gap: 10.0,
                align: Alignment::Start,
                grow: true,
            }),
        );
        let root_id = root.id;
        nodes.push(root);

        UiTree {
            root: root_id,
            nodes,
        }
    }
}

struct Component;

static mut APP: Option<Counter> = None;

fn app() -> &'static mut Counter {
    unsafe { (*core::ptr::addr_of_mut!(APP)).as_mut().unwrap() }
}

impl Guest for Component {
    fn init(_state: StateSnapshot, _size: (f32, f32), _args: Vec<String>) -> Vec<Effect> {
        host_log::info("counter: init");
        unsafe {
            APP = Some(Counter::new());
        }
        vec![]
    }

    fn update(event: InputEvent) -> Vec<Effect> {
        app().handle(event)
    }

    fn view() -> UiTree {
        app().build_tree()
    }
}

export!(Component);

#[cfg(test)]
mod tests {
    use super::*;
    use plexi::platform::types::{UiActionEvent, UiValueChangeEvent};

    fn tree_text(tree: &UiTree) -> String {
        tree.nodes
            .iter()
            .filter_map(|n| match &n.data {
                UiNodeData::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn initial_view_shows_zero() {
        let mut app = Counter::new();
        assert!(tree_text(&app.build_tree()).contains("Count: 0"));
    }

    #[test]
    fn increment_action_updates_view() {
        let mut app = Counter::new();
        let effects = app.handle(InputEvent::UiAction(UiActionEvent {
            handler_id: INCREMENT_HANDLER.to_string(),
        }));
        assert!(effects.is_empty());
        assert!(tree_text(&app.build_tree()).contains("Count: 1"));
    }

    #[test]
    fn unrelated_value_change_is_ignored() {
        let mut app = Counter::new();
        app.handle(InputEvent::UiValueChange(UiValueChangeEvent {
            handler_id: "name".to_string(),
            value: "Ada".to_string(),
        }));
        assert!(tree_text(&app.build_tree()).contains("Count: 0"));
    }
}
