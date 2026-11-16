//! Timer-only restore fixture. Regenerate with `just wasm-fixtures`.

use plexi_wasm_sdk::{Effect, InitContext, InputEvent, UiTree, View};

struct TimerOnly;

impl View for TimerOnly {
    fn init(&mut self, _context: InitContext) -> Vec<Effect> {
        vec![plexi_wasm_sdk::effects::set_timer(1, 1000, true)]
    }

    fn update(&mut self, _event: InputEvent) -> Vec<Effect> {
        Vec::new()
    }

    fn view(&self) -> UiTree {
        UiTree {
            root: 0,
            nodes: Vec::new(),
        }
    }
}

plexi_wasm_sdk::export_plugin!(TimerOnly);
