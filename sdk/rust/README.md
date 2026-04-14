# plexi-sdk

Rust SDK for building [Plexi](https://github.com/ianjamesburke/PLEXI) external
apps. Plexi is a terminal multiplexer and agent workspace where external apps
render custom UI panes by exchanging newline-delimited JSON messages with the
host over stdin/stdout.

## Installation

```toml
[dependencies]
plexi-sdk = "0.3"
```

## Changelog

### 0.3.0

App composition primitive: `DrawCommand::SpawnApp` and matching `Emitter` /
`RenderContext` helpers. One app can now ask Plexi to launch another app
and place it in a layout slot relative to itself, with lifecycle bonding
(`cascade` / `orphan` / `prompt`) and optional pre-wired typed-pipe
channels. See `docs/specs/app-infrastructure.md#app-spawning` in the repo
for the full contract.

- New outbound `DrawCommand::SpawnApp` variant and mirror types
  `SpawnParent`, `SpawnLayout`, `SpawnLifecycle`.
- New `Emitter::spawn_app` + `RenderContext::spawn_app` helpers.
- `PlexiEvent` and manifest surface unchanged; the new command is purely
  additive.

### 0.2.0

Feature parity with the Python SDK 0.2.0:

- New inbound events on `PlexiEvent`: `Scroll`, `MouseDown`, `MouseUp`,
  `MouseMove`, `Drop`, `GetState`, `SetState`. `Render` now carries
  `delta_time`.
- New outbound `DrawCommand` variants: `Image`, `VideoThumbnail`, `FileGrid`,
  `DropTarget`, `Log`, `State`, `CostReport`, `Notification`, `SetCursor`,
  `MouseTracking`.
- New `App` trait hooks (all default no-op): `on_scroll`, `on_mouse_down`,
  `on_mouse_up`, `on_mouse_move`, `on_drop`, `on_get_state`, `on_set_state`.
- New `Emitter` helpers: `log` / `info` / `warn` / `error` / `debug`,
  `cost_report`, `notification`, and the **client-side**
  `submit_feedback` (writes `feedback.jsonl` directly into the app's
  install directory — not a draw command).
- New `RenderContext` helpers mirroring the Python surface: `image`,
  `video_thumbnail`, `file_grid`, `drop_target`, `set_cursor`,
  `mouse_tracking`, `log`/`info`/`warn`/`error`/`debug`, `notification`.
- `Emitter` and `RenderContext` now read `PLEXI_APP_ID` from the
  environment so `cost_report` and `notification` are attributed correctly
  without ceremony.

## Quick start

Implement the `App` trait and call `run`. All event handlers have default
no-op implementations, so an app can override only what it needs.

```rust
use plexi_sdk::{App, Emitter, Modifiers, MouseButton, RenderContext, run};

struct Counter {
    count: i32,
}

impl App for Counter {
    fn on_render(&mut self, ctx: &mut RenderContext) {
        ctx.rect(0.0, 0.0, ctx.width, ctx.height, "#1e1e2e");
        ctx.text_bold(20.0, 20.0, "Counter", 18.0, "#cdd6f4");
        ctx.text(
            20.0,
            50.0,
            &format!("Count: {}", self.count),
            16.0,
            "#a6e3a1",
        );
        ctx.text(20.0, 80.0, "j/k to change", 12.0, "#6c7086");
    }

    fn on_key(&mut self, key: &str, _mods: &Modifiers, _emit: &mut Emitter) {
        match key {
            "J" | "ArrowDown" => self.count += 1,
            "K" | "ArrowUp" => self.count -= 1,
            _ => {}
        }
    }
}

fn main() {
    run(&mut Counter { count: 0 });
}
```

## Core concepts

- **`App` trait** — implement one or more of `on_render`, `on_key`, `on_click`,
  `on_command`, `on_resize`. Every method has a no-op default.
- **`RenderContext`** — accumulates draw commands during `on_render` and
  flushes them atomically at the end of the frame. Primitives: `rect`,
  `rect_rounded`, `text`, `text_mono`, `text_bold`, `line`, `list`.
- **`Emitter`** — sends commands (e.g. `run_in_terminal`, `cd`) from outside
  a render frame, such as inside `on_key` or `on_command`.
- **Events** — Plexi streams `Init`, `Render`, `Resize`, `Key`, `Click`,
  `Command`, `Shutdown` as newline-delimited JSON on stdin. The SDK parses
  them and dispatches to your `App` methods.
- **`manifest.toml`** — every installed Plexi app ships a manifest describing
  its id, entry point, and capabilities. See the protocol spec for the full
  schema.

The full wire protocol and manifest format are documented in
[`docs/specs/app-infrastructure.md`](https://github.com/ianjamesburke/PLEXI/blob/main/docs/specs/app-infrastructure.md).

## Runtime model

Rust Plexi apps are compiled to standalone binaries. After `cargo build
--release`, the binary is installed to
`~/.plexi-alpha/apps/<id>/<binary>` (or `~/.plexi/apps/<id>/<binary>` on the
stable build) alongside a `manifest.toml`. The SDK is linked statically at
build time, so installed apps have no runtime dependency on a system library
— the host only needs to be able to spawn the binary and talk to it over
stdio.

## License

MIT — see [`LICENSE`](LICENSE).
