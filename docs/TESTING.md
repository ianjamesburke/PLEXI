# Testing

How Plexi is tested, which layer owns what, and how to add coverage.

## The Rule

**If you'd assert on observable state (pane tree, app UI, pixels) → write a TOML scene. If you'd assert on a return value or internal invariant → write a Rust test.** Two layers, no overlap.

| Layer | Tool | Lives in | Runs |
|---|---|---|---|
| Pure logic | `#[test]` unit tests | next to the code | `cargo test --bin plexi` |
| Host logic (AppRequest/HostEffect dispatch) | `HostHarness` (`src/testing/mod.rs`) | `src/*/tests/` | `cargo test --bin plexi` |
| Observable behavior — host UI, host apps, portals, real PGAP apps | **TOML scenes** (`tests/scenes/*.toml`) | `tests/scenes/` | `scene_suite` + `just scene` |

`PlexiUiHarness` (`src/ui_tests.rs`) is the engine under scenes: PlexiApp inside egui_kittest with fully headless wgpu Metal rendering (no display needed). Direct PlexiUiHarness tests in `src/ui_tests.rs` are for egui-input edge cases scenes can't express (e.g. rename-commit-on-Enter inside a draw pass).

## Scenes

A scene file is a list of steps: setup, actions, structured assertions, optional screenshots. **The scene file IS the test** — `scene_suite` globs `tests/scenes/*.toml`, so adding a file adds a regression test. Runner: `src/scenes.rs`.

```toml
size = [1280.0, 800.0]      # optional, default 1280x800
suite = false               # optional, default true. false = excluded from
                            # scene_suite (use for scenes spawning real app
                            # processes); run them via `just scene`.

[[steps]]
open_app = "apps/balls"                     # real Python process, production launch path
args = ["--state", '{"balls": 3}']          # optional; surfaces as ctx.args in the SDK

[[steps]]
wait_app_frame = { timeout_s = 15.0 }       # block until first committed frame

[[steps]]
assert = { pane_count = 1, lifecycle = "running", tree_contains = "balls" }

[[steps]]
shot = "balls.png"                          # optional; written to the out dir
```

### Verbs

| Verb | Effect |
|---|---|
| `open_app = "<dir>"` (+ optional `args = [...]`) | Spawn a real PGAP app process from an app dir (relative paths resolve from the repo root). Args surface as `ctx.args`. |
| `open_file_browser = "<dir>"` | Open the built-in file browser host app at a dir. |
| `key = "cmd+b"` | Press a key combo (`cmd`/`ctrl`/`alt`/`shift` + key name). |
| `sidebar = true` | Show/hide the host sidebar. |
| `switch_context = 0` | Switch to the context at this router index. |
| `push_to_subcontext = "Name"` | Push the focused pane into a new subcontext (creates a portal). |
| `wait_app_frame = { timeout_s = N }` | Wait for the last-opened app's first committed frame. Fails with the app's stderr on crash/timeout. |
| `run_steps = N` | Advance N harness frames. |
| `assert = { ... }` | Structured assertions — see below. |
| `shot = "name.png"` | Headless screenshot to the out dir. |

New verbs require a scene that needs them — no speculative DSL growth. Assertions are structured keys, never expression strings.

### Assertions

`pane_count`, `window_count`, `context_count`, `portal_count` (across all windows), `sidebar`, `lifecycle` (last-opened app, lowercase, e.g. `"running"`), `tree_contains` (substring match against the app's serialized L1 render tree). Every present key must match.

### Running

```bash
just scene tests/scenes/balls.toml           # one scene → /tmp/plexi-scenes
just scene <file> /tmp/out 0                 # state-only: skip shot steps (no GPU)
cargo test --bin plexi scene_suite           # all committed suite scenes
```

`just scene` accepts absolute paths, so one-off scenes can live in `/tmp` and never touch the repo.

### SceneReport

Every run writes `<out>/<scene>.json` (`schema_version: 1`): pass/fail per step, host state snapshot (context/window/pane/portal counts, sidebar), and the last-opened app's `lifecycle` plus its full committed L1 render tree as JSON. **Prefer asserting on state over comparing pixels** — the tree is the app's UI state; screenshots are an artifact for human review.

## Real App Processes in Tests

`open_app` (and `PlexiUiHarness::open_app_at`) uses the production path: manifest load → `ProcessApp::launch` → real child process, IPC threads, L1 render pipeline. Outside an installed bundle the repo SDK is exported via `PLEXI_SDK_PATH=sdk/python` automatically. Python resolution follows production order: per-app `.venv` → bundled python → system `python3`.

## Pre-Push Evidence

The `/testing` skill (`.agents/skills/testing/SKILL.md`) is the agent workflow: classify the diff, run the matching layer's tests/scenes, inspect screenshots, write the `**Test evidence:**` block into the issue Ship Log. validate-pr reads that block; `install skippable — full coverage` means diff-review-only validation.

## Conventions

- `cargo test --bin plexi` must be green before any push; the justfile exports `RUSTFLAGS="-D warnings"`, so warnings are build failures under `just test`.
- PTY-dependent tests are tagged `#[ignore = "requires-pty"]`.
- Test-first for host logic: a new `AppRequest`/`HostEffect` gets a failing `HostHarness` test before implementation.
- New host UI component or overlay → a scene in `tests/scenes/` (open → act → assert → shot).
- Coverage: `cargo llvm-cov --bin plexi` (cargo-llvm-cov installed at `~/.cargo/bin/`).
