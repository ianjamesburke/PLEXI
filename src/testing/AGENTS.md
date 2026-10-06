# src/testing — Agent Contract

**Read before editing anything under src/testing/:** this file, plus the root AGENTS.md.

## Scope

Test infrastructure for the Plexi host. `HostHarness` (headless egui test harness) and `PlexiUiHarness` (headless wgpu Metal renderer for scenes).

## Reference

- [TESTING.md](TESTING.md) — mandatory self-validation contract for every coding agent: layers, scene format, coverage map, evidence workflow, and profile isolation.

## Rules

- **If you'd assert on observable state** (pane tree, app UI, pixels) → write a TOML scene in `tests/scenes/`. If you'd assert on a return value or internal invariant → write a Rust `#[test]`.
- **Test-first for host logic.** A new `AppRequest` or `HostEffect` gets a failing `HostHarness` test before implementation.
- **New host UI component or overlay** → add a scene in `tests/scenes/` (open → act → assert → shot).
- `cargo test --bin plexi` must be green before any push.
- Every `HostHarness::new()` creates a fresh tempdir for profile isolation. Tests never touch `$HOME`.
- PTY-dependent tests are tagged `#[ignore = "requires-pty"]`.

## Traps

- **`plexi pane key` must exercise real key handlers, never a parallel resolver path.** The host `KeyPane` handler routes by pane type: terminals get PTY bytes, Python (PGAP) apps get `PlexiEvent::Key`, and native (builtin/WASM) apps first get a synthesized `egui::InputState` through `App::handle_key` (`drive_native_pane_key` in `src/app/mod.rs`) — the same handler a physical keystroke reaches. `Consumed` stops there; `Passthrough` focuses the real pane and replays the synthesized events into the current production egui pass so widgets such as `TextEdit` can consume Enter/Tab during rendering. When adding pane-driving capabilities, extend these paths; never add a hidden CLI that bypasses an app's own keyboard flow. Native panes report `disposition` in the response file so drive-host validation can distinguish both paths.
- **Text input uses `plexi pane send`, not printable `pane key` calls.** `SendToPane` keeps PTY writes for terminals, but for app panes it focuses the real pane and appends one `egui::Event::Text` to the current production input pass. `App::handle_key` handles structured keys; an egui `TextEdit` consumes `Event::Text` during rendering, so per-character `pane key` calls cannot substitute for text entry.
- **`cargo test --lib` silently misses host tests.** `--lib` only runs the `protocol` lib target (~47 tests). Host tests — app_registry, HostHarness, wasm_python, workspace_secrets — live in the binary target. Always use `cargo test --bin plexi`.
- **`HostHarness::add_test_pane()` inserts a builtin app pane, not a Terminal.** Terminal-count assertions must not assume the initial pane is a Terminal; offset accordingly.
- **Harness context ids and launched pane ids are process-unique.** `GLOBAL_REGISTRY` is process-global, keyed by pane id and namespaced by `context_id`, and `cargo test` runs every `#[test]` in one process. `HostHarness` and `PlexiUiHarness` assign a fresh context via `reserve_test_context_id` and seed `HostModel`'s pane counter from `reserve_pane_id_block` (launches were still starting at 1). A second window that must share that workspace copies `windows[0].context_id`. Hardcoding context `1` or assuming the first launched pane is id 1 merges harnesses. `new_for_test` itself stays on context 1 and pane id 1 for tests that construct `PlexiApp` directly and assert those ids.
- **Test constructor sync.** When adding a field to any struct that has a `new_for_test()` constructor, update that constructor in the same commit. Run `cargo test --bin plexi` on the base branch first to distinguish pre-existing failures from regressions.
- **Shared test context id leaks same-named tools across parallel tests.** Every harness window starts on the context id baked into `PlexiApp::new_for_test`, and `tool_dispatch`'s process-global registry shows a tool to every viewer in that same context. Disjoint pane ids stop one test's `AppPane` drop from unregistering another's key; they do not stop a second live `chess.play` in another harness from withdrawing the bare name. A test that registers a tool name another concurrent test also registers must move the window `context_id` and the matching router context together before the app launches (`isolate_tool_context`). `PlexiApp::new_for_test` seeds `HostModel`'s pane counter from `reserve_pane_id_block` because launch paths allocate there, not on `HostHarness`'s direct-insert counter.
- **Never root a harness app at a real machine directory.** A `FileBrowserApp` (or any scanning app) pointed at `std::env::temp_dir()` renders whatever the dev machine's temp dir holds — ~50k entries drove test binaries past 30 GB RSS while CI stayed green on its clean temp. Root harness panes in the harness's own workspace tempdir (`HostHarness::_workspace_dir`, `PlexiUiHarness::workspace`) or a scoped `tempfile::tempdir()`. The same rule holds in the write direction, and it bites harder off macOS: `set_context_root` auto-inits a workspace, so rooting a test at the shared temp dir leaves a `/tmp/.plexi` behind, and from then on every `tempfile::tempdir()` in the suite resolves a workspace root by walking up into it — five unrelated tests fail on a *later* run, on a machine where nothing changed. macOS hides this because `std::env::temp_dir()` is a private per-user dir there; on Linux it is `/tmp`, the shared parent of every test fixture.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.
