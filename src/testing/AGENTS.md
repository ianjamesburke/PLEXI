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
- **`cargo test --lib` silently misses host tests.** `--lib` only runs the `app_protocol` lib target (~47 tests). Host tests — app_registry, HostHarness, wasm_python, workspace_secrets — live in the binary target. Always use `cargo test --bin plexi`.
- **`HostHarness::add_test_pane()` inserts a builtin app pane, not a Terminal.** Terminal-count assertions must not assume the initial pane is a Terminal; offset accordingly.
- **Test constructor sync.** When adding a field to any struct that has a `new_for_test()` constructor, update that constructor in the same commit. Run `cargo test --bin plexi` on the base branch first to distinguish pre-existing failures from regressions.
- **Never root a harness app at a real machine directory.** A `FileBrowserApp` (or any scanning app) pointed at `std::env::temp_dir()` renders whatever the dev machine's temp dir holds — ~50k entries drove test binaries past 30 GB RSS while CI stayed green on its clean temp. Root harness panes in the harness's own workspace tempdir (`HostHarness::_workspace_dir`, `PlexiUiHarness::workspace`) or a scoped `tempfile::tempdir()`.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.
