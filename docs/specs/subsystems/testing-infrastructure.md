# Testing Infrastructure

**Status:** Active (v3.0)
**Last updated:** 2026-04-18

---

## 1. Three Layers

Every feature in Plexi has a testable surface at three distinct levels. Tests at all three run via `cargo test` — no GUI, no browser, no display required.

```
Layer 3: Visual assertions   — draw commands → PNG via headless renderer
Layer 2: Host state machine  — HostModel commands/effects via HostHarness
Layer 1: App protocol        — PGAP events/draw commands via pgap_test_harness
```

Each layer is independent. Layer 1 tests have no dependency on the host. Layer 2 tests have no dependency on real apps. Layer 3 tests have no dependency on egui.

---

## 2. Layer 1 — App Protocol (pgap_test_harness)

**What it tests:** An individual app's protocol behavior in isolation. No host process. No egui.

**How it works:**
- Spawns the app as a subprocess with piped stdio
- Sends `Init` JSON, asserts `Ready`
- Sends `Render`, collects `DrawCommand` sequence up to `FrameDone`
- Sends `Key`, `PathChanged`, `Shutdown` events
- Asserts on the command stream (types, content, ordering)

**File:** `src/pgap_test_harness.rs`

**Running:**
```sh
cargo test --lib pgap_test_harness
```

**Example test — app renders expected content:**
```rust
let mut h = python_harness(&examples_dir().join("todo"), "todo.py").unwrap();
h.init_and_expect_ready("todo", &workspace_root);
let cmds = h.render_frame(1, 800.0, 600.0);
assert!(cmds.iter().any(|v| v["type"] == "text" && v["text"] == "Todo"));
```

**Example test — app reacts to PathChanged:**
```rust
h.send_path_changed(&new_cwd);
let cmds = h.render_frame(2, 800.0, 600.0);
assert!(cmds.iter().any(|v| v["text"] == new_cwd.display().to_string()));
```

### 2.1 Mocking app-level system access

Apps that call `net.http`, `fs.read`, etc. will hit real systems in Layer 1 tests unless mocked. For deterministic tests, use env vars or test fixtures:

- Inject `PLEXI_AUDIO=mock://in.wav,out.wav` and `PLEXI_VIDEO=mock://fixture.mp4` for media apps
- For net/fs apps: provide a test server or fixture directory via env var passed to the subprocess

### 2.2 Packaging for app developers

`src/pgap_test_harness.rs` is internal to the Plexi repo today. The intent is to extract it as a `plexi-test` crate that app developers can depend on. App devs write:

```toml
[dev-dependencies]
plexi-test = { git = "https://github.com/ianjamesburke/PLEXI" }
```

Then in their app's test suite:
```rust
use plexi_test::{AppHarness, mock_env};

#[test]
fn my_app_renders_header() {
    let mut h = AppHarness::python("path/to/my_app.py", mock_env()).unwrap();
    h.init("my-app", "/tmp");
    let frame = h.render(800.0, 600.0);
    frame.assert_text("My App");
}
```

---

## 3. Layer 2 — Host State Machine (HostHarness)

**What it tests:** Host-side business logic — pane routing, focus, splits, pane groups, capability decisions, event bus output. No egui. No subprocess spawning.

**How it works:**
- Constructs `HostModel` directly (no GUI)
- Submits `HostCommand`s
- Collects `HostEffect`s
- Asserts on effects and model state
- `HostServices` is wired with mock impls that return injected data

**Files:** `src/host/harness.rs`, `src/host/model.rs`

**Running:**
```sh
cargo test --lib host
```

**Example test — key routes to focused app:**
```rust
let mut h = HostHarness::new();
h.launch_app("todo");
h.send_key("j");
h.assert_effect(HostEffect::AppKeyDispatched { pane_id: h.focused_pane_id(), key: "j".into() });
```

**Example test — PathChanged broadcasts to group:**
```rust
let mut h = HostHarness::new();
let term_id = h.focused_pane_id();
let explorer_id = h.launch_app("explorer");
h.join_group(term_id, "cwd");
h.join_group(explorer_id, "cwd");
h.path_changed(term_id, "/tmp/project");
h.assert_effect(HostEffect::PathBroadcasted {
    group: "cwd".into(),
    cwd: "/tmp/project".into(),
    recipient_pane_ids: vec![explorer_id],  // terminal does not receive its own broadcast
});
```

**Example test — capability prompt on undecided capability:**
```rust
let mut h = HostHarness::new();
let wiki_id = h.launch_app_with_capabilities("wikipedia", &[Capability::NetHttp]);
h.check_capability(wiki_id, Capability::NetHttp);
h.assert_effect(HostEffect::CapabilityPromptRequired { pane_id: wiki_id, capability: Capability::NetHttp });
```

**Example test — mock filesystem data:**
```rust
let mut h = HostHarness::new();
h.services.fs.inject("/tmp/project/.git/HEAD", "ref: refs/heads/main\n");
let git_id = h.launch_app("git-log");
// app requests fs.read, gets injected data — no real git repo needed
```

### 3.1 HostHarness convenience API

```rust
h.launch_app(app_id)                          // OpenPane with App kind
h.launch_app_with_capabilities(app_id, caps) // OpenPane + pre-declare capabilities
h.split_h(app_id)                             // SplitHorizontal
h.split_v(app_id)                             // SplitVertical
h.navigate(Direction::Right)                  // Navigate
h.send_key(key)                               // SendKeyToFocusedApp
h.path_changed(pane_id, cwd)                  // SimulatePathChanged
h.join_group(pane_id, group)                  // add pane to named group
h.grant_capability(pane_id, capability)       // pre-decide capability
h.check_capability(pane_id, capability)       // CheckCapability command
h.events()                                    // Vec<HostEvent> from VecEventSink
h.assert_effect(effect)                       // assert effect is in collected effects
h.assert_event(event)                         // assert event is in event sink
h.focused_pane_id()                           // current focused pane
h.pane_count()                                // total panes in active context
```

---

## 4. Layer 3 — Visual Assertions (Headless Renderer)

**What it tests:** That the app's draw commands produce the expected visual output. Used in the agent dev loop for visual feedback without a running GUI.

**How it works:**
- Collects `Vec<DrawCommand>` from Layer 1's `render_frame()`
- Passes them to the headless renderer (`src/headless_renderer.rs`)
- Gets back a PNG as `Vec<u8>`
- Tests can save the PNG, diff it against a reference, or pass it to an agent for inspection

**File:** `src/headless_renderer.rs` (built on `tiny-skia`)

**Activated by:** `PLEXI_RENDER=headless` env flag

**Running:**
```sh
PLEXI_RENDER=headless cargo test --lib headless_renderer
```

**Example test — render snake initial frame:**
```rust
let mut h = python_harness(&examples_dir().join("snake"), "snake.py").unwrap();
h.init_and_expect_ready("snake", &workspace_root);
let cmds = h.render_frame(1, 640.0, 480.0);
let png = render_to_png(&cmds, 640, 480);
// save for visual review, or diff against reference
std::fs::write("test-output/snake-frame-1.png", &png).unwrap();
```

**Determinism:** Apps must use `frame_timestamp` from the `Render` event for any time-dependent rendering (animations, clocks). Never use `time.time()` or `random` in a render function — this breaks reproducibility. The harness always passes `frame_timestamp = 0` in test mode.

**What the headless renderer does NOT render:**
- `VideoPlayer` — skipped (replaced with a placeholder rect labeled "[video]")
- `AudioMeter` — skipped (replaced with a placeholder rect labeled "[audio]")

These require real or mock media devices and are covered by the mock device tests in the audio/video subsystem, not the headless renderer.

---

## 5. The Agent Dev Loop

The three layers compose into a development loop where an agent can build and verify a Plexi app without ever needing a running GUI:

```
1. Agent writes app.py + manifest.toml
2. Layer 1: pgap_test_harness spawns app, sends Init + Render
3. App emits DrawCommands
4. Layer 3: headless renderer produces frame.png
5. Agent inspects PNG (visually or via assertions on pixel content)
6. Agent sends Key event, renders again, asserts new state
7. Repeat until behavior is correct
8. Layer 2: HostHarness exercises host-side behavior (routing, groups, capabilities)
9. smoke-test.sh confirms app handshake works with the real host binary
10. Install
```

This loop works entirely offline, requires no display, and produces reproducible output.

### 5.1 Best practices for testable apps

- Use `frame_timestamp` from the `Render` event, not `time.time()` or `random`
- Keep render logic pure: same state → same draw commands
- Use `init.workspace_root` for all file paths rather than hardcoding
- Register stubs for any external data sources (see §2.1)

---

## 6. CI Gate

The CI gate for v3.0 requires all three layers green:

```sh
# Layer 2: host state machine
cargo test --lib host

# Layer 1 + Layer 3: app protocol + visual
cargo test --lib

# Smoke: real binary, all installed apps
scripts/smoke-test.sh
```

No test is allowed to launch a GUI window, load a browser, or require a real audio/video device. `PLEXI_AUDIO=mock://` and `PLEXI_VIDEO=mock://` are always set in CI.

---

## 7. Adding Tests for a New Feature

When you add a feature that changes host behavior (new command, new routing, new capability):

1. **Write the Layer 2 test first** — add a test to `src/host/harness.rs` that drives the new `HostCommand` and asserts the expected `HostEffect`s. This test should pass before you touch any egui code.
2. **Write the Layer 1 test** — if the feature involves an app (new PGAP event, new draw command), add a test to `src/pgap_test_harness.rs` that exercises the app's response.
3. **Add a Layer 3 test if visual** — if the feature produces visible output, add a headless render test and save a reference PNG.
4. **Run smoke-test after `just install-v3`** — never report a task complete before smoke passes.
