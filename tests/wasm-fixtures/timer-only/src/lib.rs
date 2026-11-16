//! Timer-only restore fixture. Regenerate from the repository root with:
//! `cargo +stable build --manifest-path tests/wasm-fixtures/timer-only/Cargo.toml --locked --release --target wasm32-wasip2 --target-dir target`
//! Copy `target/wasm32-wasip2/release/timer_only.wasm` to `tests/wasm-fixtures/timer-only.wasm`.

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
