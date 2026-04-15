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

Two parallel additions in one release:

**App composition primitive** — `DrawCommand::SpawnApp` and matching `Emitter` /
`RenderContext` helpers. One app can now ask Plexi to launch another app
and place it in a layout slot relative to itself, with lifecycle bonding
(`cascade` / `orphan` / `prompt`) and optional pre-wired typed-pipe
channels. See `docs/specs/subsystems/app-infrastructure.md#app-spawning` for the full contract.

- New outbound `DrawCommand::SpawnApp` variant and mirror types
  `SpawnParent`, `SpawnLayout`, `SpawnLifecycle`.
- New `Emitter::spawn_app` + `RenderContext::spawn_app` helpers.

**Breakpoint dispatcher + min-size primitive** — declare `[app.layout]` with
`min_width` / `min_height` in your manifest. When the pane is smaller than
the floor, the SDK draws a built-in "too small" frame (background + label +
directional arrow + current size) and skips `on_render`. Apps with multiple
size strategies can use the new `BreakpointSet` builder to pick a render
path by pane size.

- New `BreakpointSet` builder + `pick_breakpoint` free function.
- New `App::min_size` trait method.
- New `load_manifest_layout()` helper that reads `[app.layout]` without
  pulling in a TOML dep (hand-rolled mini-parser).
- `PlexiEvent` and the existing manifest surface are unchanged; both
  additions are purely additive.

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

## Breakpoints and minimum size

The SDK provides two first-class primitives for handling pane resizing:
**breakpoints** (pick a render function by pane size) and **auto min-size
fallback** (draw a built-in "too small" frame when the pane is below a
declared floor).

### Declaring a minimum size

Add an `[app.layout]` table to your `manifest.toml`:

```toml
[app]
id    = "my-app"
name  = "My App"
entry = "my-app"

[app.layout]
min_width  = 400   # logical pixels, default 0 (no floor)
min_height = 200   # logical pixels, default 0 (no floor)
```

When the pane is smaller than the declared floor on either axis, the SDK
draws a built-in frame (dark background + centered `min size: 400 x 200`
label + directional arrow + dim `current: w x h` subtitle) and bypasses
`on_render` entirely.

You can also override the floor programmatically from the trait:

```rust
impl App for MyApp {
    fn min_size(&self) -> (f32, f32) { (400.0, 200.0) }
    // ...
}
```

A non-zero `min_size()` return wins over the manifest; `(0.0, 0.0)` (the
default) defers to the manifest.

### Breakpoint dispatchers

Two patterns are supported. Pick whichever suits your app's state model.

**Stateless closures via `BreakpointSet`** (simple drawing-only branches):

```rust
use plexi_sdk::{BreakpointSet, RenderContext};

let mut breakpoints = BreakpointSet::new()
    .breakpoint(800.0, 500.0, |ctx: &mut RenderContext| {
        ctx.rect(0.0, 0.0, ctx.width, ctx.height, "#1e1e2e");
        ctx.text_bold(20.0, 20.0, "Dashboard (full)", 18.0, "#cdd6f4");
    })
    .breakpoint(400.0, 0.0, |ctx: &mut RenderContext| {
        ctx.rect(0.0, 0.0, ctx.width, ctx.height, "#1e1e2e");
        ctx.text(20.0, 20.0, "Dashboard (compact)", 14.0, "#cdd6f4");
    })
    .fallback(|ctx: &mut RenderContext| {
        ctx.rect(0.0, 0.0, ctx.width, ctx.height, "#1e1e2e");
        ctx.text(10.0, 10.0, "·", 12.0, "#6c7086");
    });

// Inside on_render:
breakpoints.dispatch(ctx);
```

`BreakpointSet::dispatch` walks entries sorted by `min_width * min_height`
descending and fires the first one whose bounds fit the pane. If none match,
a `(0, 0)` fallback entry is used if present; otherwise `dispatch` returns
`false`.

**Stateful dispatch via `pick_breakpoint`** (when breakpoints need `&mut
self`):

```rust
use plexi_sdk::{pick_breakpoint, App, RenderContext};

struct Dashboard { /* ... */ }

impl Dashboard {
    fn render_full(&mut self, ctx: &mut RenderContext) { /* ... */ }
    fn render_compact(&mut self, ctx: &mut RenderContext) { /* ... */ }
    fn render_minimal(&mut self, ctx: &mut RenderContext) { /* ... */ }
}

impl App for Dashboard {
    fn on_render(&mut self, ctx: &mut RenderContext) {
        const BPS: &[(f32, f32)] = &[(800.0, 500.0), (400.0, 0.0), (0.0, 0.0)];
        match pick_breakpoint(ctx.width, ctx.height, BPS).unwrap_or(BPS.len() - 1) {
            0 => self.render_full(ctx),
            1 => self.render_compact(ctx),
            _ => self.render_minimal(ctx),
        }
    }

    fn min_size(&self) -> (f32, f32) { (320.0, 180.0) }
}
```

`pick_breakpoint` takes raw scalars and returns the winning index, so the
caller dispatches to their own `&mut self` methods without borrow-check
friction.

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
[`docs/specs/subsystems/app-infrastructure.md`](https://github.com/ianjamesburke/PLEXI/blob/main/docs/specs/subsystems/app-infrastructure.md).

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
