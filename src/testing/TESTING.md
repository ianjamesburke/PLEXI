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
| `drop_file = { target = "<handle>", value = "<path-or-url>" }` | Deliver a file or image URL through the same pane drop handler used by native host drops. Rejection fails the scene. |
| `focus = "<handle>"` | Focus an opened pane through the production pane-navigation path (`plexi pane focus` live). |
| `close = "<handle>"` | Close an opened pane through the production pane lifecycle path (`plexi pane close` live). |
| `sidebar = true` | Show/hide the host sidebar. |
| `switch_context = 0` | Switch to the context at this router index. |
| `push_to_subcontext = "Name"` | Push the focused pane into a new subcontext (creates a portal). |
| `wait_app_frame = { target = "<handle>", timeout_s = N }` | Wait for a process app's first committed frame. Fails with the app's stderr on crash or timeout. |
| `run_steps = N` | Advance N harness frames. |
| `assert = { ... }` | Structured assertions — see below. |
| `expect = { target = "<handle>", after_key = "left", node_changes = "paddle", timeout_s = 2.0 }` | Deliver input normally and poll semantic state. On input timeout the stable failure code is `input_no_effect`; without input it is a normal eventual assertion. |
| `assert_label = { target = "<handle>", label = "Settings" }` | Focus a pane and require an exact semantic label inside that pane's rendered rectangle. Use `target = "host"` for the whole accessibility tree. |
| `shot = "name.png"` | Headless screenshot to the out dir. |

Every `open` step binds one unique symbolic handle. Later pane steps reject missing handles, duplicate handles fail before an app opens, and `host` is reserved for explicit whole-host input. New verbs require a scene that needs them. Assertions are structured keys, never expression strings. `tests/scenes/assistant-settings.toml` exercises `open`, `text`, pane-targeted `key`, and `assert_label` against the native Assistant without starting an AI turn.

### Assertions

`pane_count`, `window_count`, `context_count`, `portal_count` (across all windows), and `sidebar` assert host state. `exists`, `focused`, `lifecycle`, and `tree_contains` require `target = "<handle>"`; `fit = "fill"|"contain"` and `aspect = [width, height]` inspect committed canvas semantic bounds. `exists` and `focused` check pane lifecycle/focus while the latter inspect a process or WASM app's serialized L1 state. Native builtins use `assert_label`. Every present key must match. Prefer handle-scoped assertions in scenes meant for both backends: an installed profile may restore unrelated panes, so absolute global counts are intentionally profile-dependent.

### Running

```bash
just scene tests/scenes/balls.toml           # one scene → /tmp/plexi-scenes
just scene <file> /tmp/out 0                 # state-only: skip shot steps (no GPU)
just scene-live <file> pr-123                # same schema against installed host
cargo test --bin plexi scene_suite           # all committed suite scenes
```

`just scene` accepts absolute paths, so one-off scenes can live in `/tmp` and never touch the repo.

### SceneReport

Every run writes `<out>/<scene>.json` (`schema_version: 4`). It includes the selected backend and channel, pass/fail per step, resolved real pane ids, host state, teardown result, and the last-opened app state. Successful action details and failures are structured objects; failures carry stable `code` and `message` fields. Failed steps include a `failure_bundle` path with the semantic dump, log tail, manifest (including eventual-poll history), and the headless screenshot when available. Prefer state and semantic-label assertions over pixel comparisons. Screenshots remain a human review artifact.

`just scene-live` requires an explicit installed channel, boots that channel's host, drives generic app open/text/key/context switch/context push operations through the public CLI/IPC surface, observes `pane state` and context metadata, and always tears down a runner-owned host. Set `PLEXI_SCENE_ATTACH=1` only to attach deliberately; attached hosts are never stopped by the runner. The outer script writes a runner-only ownership marker, so SIGINT/SIGTERM/HUP cleanup can stop an owned host without touching an attached one. Unsupported live verbs fail with `unsupported_live_verb` rather than silently diverging from headless semantics. Live assertions use bounded eventual polling with a small poll interval and a stable-state barrier, never fixed workflow sleeps.

Changes to `LiveBackend` require a `just scene-live` run against the installed PR
channel. A passing headless scene does not exercise CLI command construction,
host startup/ownership, or eventual polling.

Whole-host `text`, `key`, and `assert_label` targets are headless-only because the installed host exposes no sanctioned generic host-input or host-semantic CLI/IPC seam. The live backend returns `unsupported_live_target` for `target = "host"`; pane targets retain the shared semantics.

## Host Surface Coverage

Use this map before extending the DSL. “Scene” means observable coverage through
the shared TOML schema; “logic” names the lower layer for invariants that are not
pixels. A blank scene cell is a deliberate gap, not permission to add an ad hoc
driver.

| Host surface | Scene coverage | Logic coverage | Live CLI surface |
|---|---|---|---|
| Open process, builtin, and WASM panes | `open`, app-specific scenes | launch and lifecycle tests | `plexi app open` |
| Pane focus, close, text, and native/process/WASM keys | `focus`, `close`, `text`, `key`; `pane-lifecycle.toml`, `assistant-settings.toml`, `wasm-sysmon.toml` | `HostHarness` IPC and native key tests | `plexi pane focus/close/send/key` |
| Pane file and URL drops | `drop_file`; `notes-agent-drive.toml` | Production dispatch, persistence, and observable acceptance or rejection tests | `plexi pane drop <id> <path-or-url>` |
| Canvas/pointer clicks on process and WASM app panes (stint 0398) | no scene verb yet — use `HostHarness::inject_click`/`AppRequest::ClickPane` for headless coverage | `HostHarness` `click_pane_delivers_canvas_space_coordinate_through_fit_contain_transform` (end-to-end, real process app, `fit="contain"`) + `wasm_render.rs` `canvas_click_inverts_fit_transform_to_canvas_space` (unit, transform math only) | `plexi pane click <id> <x> <y> [--button left]` — pane-pixel coordinates, delivered through the pane's live `canvas_transform` inversion, never OS automation |
| Node-targeted clicks on Button/TextInput/ListView nodes in WASM (incl. CPython-in-WASM) app panes (stint 0414) | no scene verb yet — use `HostHarness::inject_node_click`/`AppRequest::ClickPaneNode` for headless coverage | `HostHarness` `click_pane_node_activates_button_and_mutates_guest_view` (end-to-end, real process app, node resolved from `plexi pane state`, covers the fail-loud missing/non-interactive paths) + `pane.rs` `resolve_interactive_node_*` (unit, role validation) | `plexi pane click <id> --node <node_id> [--button left]` — targets a node by the id `plexi pane state` reports, never pixel geometry; fails loudly (named error, nonzero exit) when the id is absent from the current tree or not an interactive role |
| Split directions, tabs, overlays, and multi-window placement | host shortcuts can be driven with whole-host `key`; placement state is covered in focused Rust tests | pane layout, window cleanup, and spawn tests | `plexi pane new` / `plexi app open` placement flags |
| Context switch, subcontext push, and portals | `switch_context`, `push_to_subcontext`; `subcontext-portal.toml` | context and portal lifecycle tests | `plexi context zoom/push/list` |
| Sidebar and context drag/drop | `sidebar`; file-browser sidebar scene. Pointer drag remains a direct `PlexiUiHarness` edge-case test. | sidebar reorder/drop resolver tests | no generic pointer-injection command for drag; single clicks use `plexi pane click` |
| Command palette and host overlays | whole-host `key` plus semantic/pixel assertions; direct harness tests cover draw-pass focus edge cases | palette, notification, quick-note, and focus-stack tests | no generic whole-host key command; invoke the underlying pane/context/app CLI command |
| Agent panes and terminal PTY behavior | host state and pane chrome can be captured; PTY behavior is not simulated | agent-state and `#[ignore = "requires-pty"]` tests | `plexi pane new`, `plexi agent report`, `plexi pane capture` |
| Native, process, WASM, portal, sidebar, and overlay screenshots | `shot` captures the entire headless host framebuffer, so pane type does not require a separate capture path | renderer and semantic-state tests | live scenes intentionally skip `shot`; installed validation uses `drive-host` capture |

Do not require scene and CLI syntax to be identical. They must converge on the
same production host operation. Pointer-only UI affordances and whole-host
overlays have no sanctioned live input seam; test their observable UI headlessly
and test the underlying mutation through its public CLI command.

## Real App Processes in Tests

`open = { kind = "process", ... }` uses the production path: manifest load, launch through the CPython-in-WASM adapter (`src/host/wasm_python.rs`, stint 0285), and the L1 render pipeline. Outside an installed bundle the repo SDK is exported through `PLEXI_SDK_PATH=sdk/python` automatically. Python resolution comes from `src/app/python_env.rs`: per-app `.venv`, bundled python, then system `python3`, with Python 3.11 or newer required.

## Pre-Push Evidence

This file is the self-validation contract for every coding agent. The `/testing`
skill (`.agents/skills/testing/SKILL.md`) is its executable workflow: classify the
diff, run every touched layer, inspect every generated screenshot, and write the
`**Test evidence:**` block into the issue Ship Log or PR body. validate-pr reads
that block; `install skippable — full coverage` means diff-review-only validation.

**Visual review is mandatory for any host UI change, not optional.** A change
touching layout, alignment, spacing, color, or any `egui::Ui`/render code is not
done when the assertions pass — a passing geometry or galley-halign test proves
the numbers are right, not that a human/agent looking at the pixels would agree.
Before calling a UI fix complete:

1. Write or reuse a `PlexiUiHarness::save_screenshot` test that renders the
   actual changed surface with realistic seeded content (not an empty state) —
   `screenshot_assistant_conversation_bubbles` in `src/ui_tests.rs` is the
   pattern: build real model state, open through the real pane path, screenshot.
2. Run it and **open the PNG with the Read tool and look at it** — confirm the
   change looks right, not just that the test exited 0.
3. Delete the screenshot from `/tmp` after review. Screenshots are a review
   artifact, not evidence to persist — the passing test is what future runs
   check; the PNG itself is not committed and not left on disk.
4. Note in the PR body that this loop ran (what you looked at, what you saw).

This applies whenever the diff includes host UI rendering — not only when the
task is explicitly "visual." A logic-only PR that happens to touch `render.rs`
still gets this pass before push.

## Editor Release Gate

Any diff touching `src/editor/` or `src/app/text_editor_app.rs` runs the editor
release gate before push, in this order:

1. **Core matrix + fuzz** — `cargo test --bin plexi editor::gate`. Runs the
   table-driven command matrix (`gate_cases` in `src/editor/gate.rs`), a long
   deterministic stress sequence with full undo/redo round trips, and seeded
   randomized command fuzzing with per-command invariant checks (grapheme-valid
   selections, semantic/text agreement, history consistency, monotonic
   revisions).
2. **Harness layer** — `cargo test --bin plexi editor_gate`. Drives a
   representative subset of the same matrix through the installed-host input
   paths (`SendToPane`/`KeyPane`/click/drop against a real builtin Notes pane)
   plus host-only surfaces: pointer caret placement, Live Preview toggling,
   save success/failure reporting, and drop accept/reject.
3. **Scenes headless** — the `editor-gate-*.toml` and `notes-*.toml` scenes run
   in `scene_suite`; iterate on one with `just scene tests/scenes/<file>`.
4. **Installed host** — `just editor-gate pr-<N>` re-runs the core
   qualification, then runs every editor scene against the installed PR
   channel and collects everything into `/tmp/plexi-editor-gate/pr-<N>/`:
   per-scene SceneReports and failure bundles (live runs are semantic-only;
   pixel evidence comes from the headless suite's shot steps), the core
   qualification artifact (`editor-gate-core.json`: per-case
   pass/duration/final semantic state, per-seed randomized results, totals),
   a channel `log-tail.txt`, and `summary.json`.

Randomized failures are replayable: the panic names a seed and writes a
minimized replay bundle to `$TMPDIR/plexi-editor-gate-failure-<seed>.json`;
rerun exactly that seed with `PLEXI_EDITOR_GATE_SEED=<seed>`.

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
