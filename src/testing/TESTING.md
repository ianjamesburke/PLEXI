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
open = { kind = "process", path = "apps/dev/balls", as = "balls", args = ["--state", '{"balls": 3}'] }

[[steps]]
wait_app_frame = { target = "balls", timeout_s = 15.0 }

[[steps]]
assert = { target = "balls", pane_count = 1, lifecycle = "running", tree_contains = "balls" }

[[steps]]
shot = "balls.png"                          # optional; written to the out dir
```

### Verbs

| Verb | Effect |
|---|---|
| `open = { kind = "process", path = "<dir>", as = "<handle>", args = [...] }` | Spawn a real PGAP app process from an app dir. Relative paths resolve from the repo root. Args surface as `ctx.args`. |
| `open = { kind = "wasm", path = "<file.wasm>", as = "<handle>", args = [...] }` | Open a reviewed raw WASM component. |
| `open = { kind = "builtin", id = "<id>", as = "<handle>", cwd = "<dir>" }` | Open a compiled-in host app by id. `cwd` is optional and defaults to a fresh harness-owned directory. |
| `text = { target = "<handle>", value = "hello" }` | Focus a pane and insert text through egui's normal text-input path. The report records the character count, not the text. |
| `key = { target = "<handle>", value = "enter" }` | Focus a pane and press a key combo (`cmd`/`ctrl`/`alt`/`shift` plus a key name). Use `target = "host"` for a whole-host shortcut. |
| `sidebar = true` | Show/hide the host sidebar. |
| `switch_context = 0` | Switch to the context at this router index. |
| `push_to_subcontext = "Name"` | Push the focused pane into a new subcontext (creates a portal). |
| `wait_app_frame = { target = "<handle>", timeout_s = N }` | Wait for a process app's first committed frame. Fails with the app's stderr on crash or timeout. |
| `run_steps = N` | Advance N harness frames. |
| `assert = { ... }` | Structured assertions — see below. |
| `assert_label = { target = "<handle>", label = "Settings" }` | Focus a pane and require an exact semantic label inside that pane's rendered rectangle. Use `target = "host"` for the whole accessibility tree. |
| `shot = "name.png"` | Headless screenshot to the out dir. |

Every `open` step binds one unique symbolic handle. Later pane steps reject missing handles, duplicate handles fail before an app opens, and `host` is reserved for explicit whole-host input. New verbs require a scene that needs them. Assertions are structured keys, never expression strings. `tests/scenes/assistant-settings.toml` exercises `open`, `text`, pane-targeted `key`, and `assert_label` against the native Assistant without starting an AI turn.

### Assertions

`pane_count`, `window_count`, `context_count`, `portal_count` (across all windows), and `sidebar` assert host state. `lifecycle` and `tree_contains` require `target = "<handle>"`; they inspect that process or WASM app's serialized L1 state. Native builtins use `assert_label`. Every present key must match.

### Running

```bash
just scene tests/scenes/balls.toml           # one scene → /tmp/plexi-scenes
just scene <file> /tmp/out 0                 # state-only: skip shot steps (no GPU)
cargo test --bin plexi scene_suite           # all committed suite scenes
```

`just scene` accepts absolute paths, so one-off scenes can live in `/tmp` and never touch the repo.

### SceneReport

Every run writes `<out>/<scene>.json` (`schema_version: 2`). It includes pass/fail per step, resolved handles, host state, and the last-opened process or WASM app's committed state. Successful action details and failures are structured objects; failures carry stable `code` and `message` fields. Prefer state and semantic-label assertions over pixel comparisons. Screenshots remain a human review artifact.

## Real App Processes in Tests

`open = { kind = "process", ... }` uses the production path: manifest load, `ProcessApp::launch`, real child process, IPC threads, and the L1 render pipeline. Outside an installed bundle the repo SDK is exported through `PLEXI_SDK_PATH=sdk/python` automatically. Python resolution comes from `src/app/python_env.rs`: per-app `.venv`, bundled python, then system `python3`, with Python 3.11 or newer required.

## Pre-Push Evidence

The `/testing` skill (`.agents/skills/testing/SKILL.md`) is the agent workflow: classify the diff, run the matching layer's tests/scenes, inspect screenshots, write the `**Test evidence:**` block into the issue Ship Log. validate-pr reads that block; `install skippable — full coverage` means diff-review-only validation.

## Conventions

- `cargo test --bin plexi` must be green before any push; the justfile exports `RUSTFLAGS="-D warnings"`, so warnings are build failures under `just test`.
- PTY-dependent tests are tagged `#[ignore = "requires-pty"]`.
- Test-first for host logic: a new `AppRequest`/`HostEffect` gets a failing `HostHarness` test before implementation.
- New host UI component or overlay → a scene in `tests/scenes/` (open → act → assert → shot).
- Coverage: `cargo llvm-cov --bin plexi` (cargo-llvm-cov installed at `~/.cargo/bin/`).

## Per-Test Profile Isolation

Every `HostHarness::new()` call creates a fresh `tempfile::TempDir` and installs a thread-local override so `config_dir()` and `config_path()` resolve inside that tempdir for the lifetime of the harness. The tempdir is deleted when the harness drops.

**What this means for test authors:**
- Tests that go through `HostHarness` never touch `$HOME` — no stray `~/.plexi-<hash>/` dirs.
- The override is thread-local, so worker threads spawned by the host (e.g. IPC handlers) won't see it. This is acceptable for dispatch tests; note it when writing tests that assert on paths written by background threads.
- For unit tests that call `config_dir()` directly (without HostHarness), use `set_test_profile_dir(tempdir.path().to_path_buf())` and hold the returned `TestProfileDirGuard` for the test duration.
- `set_test_channel()` remains for tests that specifically exercise channel-name derivation logic. Prefer `set_test_profile_dir` for everything else.
