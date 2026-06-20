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

## Style

Document stable contracts, not history. If a rule here stops being true after a refactor, update it in the same change; otherwise leave it alone.
