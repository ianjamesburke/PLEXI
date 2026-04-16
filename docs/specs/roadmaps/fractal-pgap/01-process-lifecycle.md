# 01 — Process Lifecycle Foundation

**Goal:** Make external app and future embedded instance processes safe to suspend, resume, shut down, and reap.

---

## Scope

- Add `PlexiEvent::Suspend` and `PlexiEvent::Resume` as additive protocol events.
- Add process group creation for child app processes on Unix/macOS.
- Ensure shutdown kills the whole child process group when graceful shutdown fails.
- Keep behavior unchanged for apps that ignore `Suspend` and `Resume`.

---

## Relevant Files

- `src/app_protocol.rs`
- `src/process_app.rs`
- `src/app_trait.rs`
- `docs/specs/subsystems/app-infrastructure.md`
- `docs/specs/subsystems/fractal-pgap.md`

---

## Implementation Notes

- `Suspend` means "stop active work and render loops if possible." It is advisory for v1/v2 apps.
- `Resume` means "return to normal event processing."
- Process groups are a host responsibility. Apps should not need to know their process group ID.
- On Unix/macOS, use `std::os::unix::process::CommandExt::process_group(0)` when spawning child processes.
- On non-Unix targets, compile without process group behavior and keep graceful shutdown.

---

## Tests

- Unit test `PlexiEvent` serialization/deserialization for `suspend` and `resume`.
- Regression test that older event JSON without these events still deserializes normally.
- Process lifecycle test that launches a helper process which spawns a child, then verifies forced shutdown reaps both when supported by the platform.

---

## Manual Verification

1. Run an existing Python app.
2. Close the pane.
3. Confirm no app child process remains.
4. Check `~/.plexi-alpha/plexi.log` for shutdown errors.

---

## Done When

- Existing app tests still pass.
- Ignoring `Suspend` and `Resume` is safe.
- Forced shutdown can clean up a process tree on macOS.
