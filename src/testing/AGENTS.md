# src/testing — Agent Contract

**Read before editing anything under src/testing/:** this file, plus the root AGENTS.md.

## Scope

Test infrastructure for the Plexi host. `HostHarness` (headless egui test harness) and `PlexiUiHarness` (headless wgpu Metal renderer for scenes).

## Reference

- [TESTING.md](TESTING.md) — full testing guide: layers, scene format, conventions, profile isolation.

## Rules

- **If you'd assert on observable state** (pane tree, app UI, pixels) → write a TOML scene in `tests/scenes/`. If you'd assert on a return value or internal invariant → write a Rust `#[test]`.
- **Test-first for host logic.** A new `AppRequest` or `HostEffect` gets a failing `HostHarness` test before implementation.
- **New host UI component or overlay** → add a scene in `tests/scenes/` (open → act → assert → shot).
- `cargo test --bin plexi` must be green before any push.
- Every `HostHarness::new()` creates a fresh tempdir for profile isolation. Tests never touch `$HOME`.
- PTY-dependent tests are tagged `#[ignore = "requires-pty"]`.

## Traps

- **`plexi pane key` must exercise real key handlers, never a parallel resolver path.** The host `KeyPane` handler routes by pane type: terminals get PTY bytes, process/PGAP apps get `PlexiEvent::Key`, and native (builtin/WASM) apps get a synthesized `egui::InputState` driven through `App::handle_key` (`drive_native_pane_key` in `src/app/mod.rs`) — the same handler a physical keystroke reaches. When adding pane-driving capabilities, extend these paths; never add a hidden CLI that bypasses an app's own keyboard flow. Native panes report `disposition` (consumed/passthrough) in the response file so drive-host validation can detect ignored keys.
- **`cargo test --lib` silently misses host tests.** `--lib` only runs the `app_protocol` lib target (~47 tests). Host tests — app_registry, HostHarness, process_app, workspace_secrets — live in the binary target. Always use `cargo test --bin plexi`.
- **`HostHarness::add_test_pane()` inserts a `ProcessApp` pane, not a Terminal.** Terminal-count assertions must not assume the initial pane is a Terminal; offset accordingly.
- **Test constructor sync.** When adding a field to any struct that has a `new_for_test()` constructor, update that constructor in the same commit. Run `cargo test --bin plexi` on the base branch first to distinguish pre-existing failures from regressions.

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.
