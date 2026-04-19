# PlexiApp → HostModel Cutover — Six Hour Plan

**Goal:** Land the thinnest possible vertical slice of the HostModel cutover so that ONE command path (app launch) flows through HostModel end-to-end, and fix the "apps open in bottom 25%" bug as the first downstream payoff. Every other command path stays on the old system this session — subsequent sessions expand the pattern to `split_focused`, `close_pane`, `focus_next`, etc.

**Why this order:** The current code has two systems running in parallel — HostModel (tested, pure, spec-aligned, 57 tests green) and the old `src/app/mod.rs` + `src/pane_ops.rs` (shipping, coupled, pre-v3). Every bug you hit in production — apps at bottom 25%, layout hardcodes, capability double-prompts — lives in the old system. Fixing them there is wasted work because the cutover deletes those paths. One vertical slice proves the pattern; the rest is mechanical expansion.

---

## Pre-flight — read these first (30 min)

Before touching any code, load context:

- [ ] `ARCHITECTURE.md` — §0 vision, §Invariants (renderer reads HostModel state, not the other way around)
- [ ] `docs/specs/subsystems/host-architecture.md` — HostModel state machine, renderer layer, security model
- [ ] `src/host/model.rs` — the target API. Read `HostCommand`, `HostEffect`, `HostModel::handle`. This is what you're wiring into.
- [ ] `src/host/harness.rs` tests — shows the exact command → effect flow. Your PlexiApp integration should look like this but driving egui instead of assertions.
- [ ] `src/pane_ops.rs:72-73` — the hardcoded 3:1 share ratio you're fixing. `src/pane_ops.rs:89-190` — the two `open_*_app_pane` functions that HostModel replaces.
- [ ] `src/app/mod.rs` — 41k LOC. DO NOT read it all. Grep for `launch_app_by_id` and `focused_pane` and `self.registry`. You need to know where PlexiApp receives the "launch app" intent and where it currently calls into `pane_ops`.

---

## Phase A — Wire HostModel into PlexiApp as a non-consumed field (60 min)

Just instantiate it. Nothing consumes it yet. This phase should not change ANY behavior.

- [ ] In `src/app/mod.rs`, add a field `host: crate::host::HostModel` to `PlexiApp`
- [ ] Initialize in `PlexiApp::new`
- [ ] Remove `#[allow(dead_code)]` from `mod host;` in `src/main.rs:27`
- [ ] Remove `#[allow(dead_code)]` from `mod protocol;` once HostModel imports are live
- [ ] Keep all other `#[allow(dead_code)]` modules as-is — don't scope-creep
- [ ] `cargo build --release` — clean
- [ ] `cargo test` — 57+ green
- [ ] `just install-v3` — smoke test passes
- [ ] Commit: `host: instantiate HostModel in PlexiApp (no behavior change)`

**Gate:** Plexi launches, every existing feature works identically. If anything broke, the wiring is wrong — revert and retry.

---

## Phase B — Translate ONE command through HostModel (90 min)

Pick `launch_app_by_id` as the first target. It's concrete, high-value (powers every app launch including the file explorer), and the existing code is well-isolated in `src/pane_ops.rs:743-771`.

- [ ] In `PlexiApp`, add a helper `fn submit(&mut self, cmd: HostCommand) -> Vec<HostEffect>` that calls `self.host.handle(cmd)` and returns the effects
- [ ] In `launch_app_by_id`, BEFORE the existing code path, build a `HostCommand::OpenPane { request: OpenPaneRequest { ... } }` and call `self.submit(cmd)` — just to prove the handle path works. Log the returned effects. Do NOT apply them yet; the old code path still handles the real work
- [ ] Launch file explorer, verify log shows `HostEffect::PaneOpened { ... }`
- [ ] Now SWAP: delete the old `open_process_app_pane` / `open_builtin_app_pane` body inside the file-explorer code path, and replace with `apply_effects(effects)` that dispatches on `HostEffect::PaneOpened` to call the egui_tiles insertion logic
- [ ] Pull the 3:1 share hardcode OUT of `pane_ops.rs:72-73`. Read `request.layout.share_fraction` from the OpenPaneRequest instead (add the field to `host::command::OpenPaneRequest` if missing). Default to 0.5.
- [ ] Run file-browser manually — it should now open at 50/50, not 75/25
- [ ] `cargo test` green (HostModel tests + the new integration path)
- [ ] `just install-v3` + smoke test pass
- [ ] Manual test: launch file browser, verify layout
- [ ] Commit: `host: route launch_app_by_id through HostModel; drop pane_ops 3:1 hardcode`

**Gate:** File browser opens at the default share (50/50). No other commands touched.

---

## Phase C — Manifest-declared layout share (90 min)

Fix the root cause: apps declare their layout in `manifest.toml`, not the host hardcoding it.

- [ ] Add `initial_share: Option<f32>` to `AppManifest.capabilities` in `src/app_registry.rs:79` (right beside `layout_hint`)
- [ ] Add `share_for(&self, app_id: &str) -> Option<f32>` method on registry (mirrors `layout_hint_for`)
- [ ] In `launch_app_by_id_with_layout`, include the share in the `OpenPaneRequest`
- [ ] Update `~/.plexi-v3/apps/file-browser/manifest.toml` (or wherever the built-in file browser declares itself) with `initial_share = 0.5`
- [ ] Update `examples/*/manifest.toml` for the in-repo examples with sensible shares: quick-note 0.3, snake 0.5, wikipedia 0.6, todo 0.4, audio-recorder 0.3, video-player 0.7
- [ ] Spec entry in `docs/specs/releases/plexi-v3.0.md` — add `initial_share` to the manifest capabilities section. Reference it in the protocol spec
- [ ] `cargo test` green
- [ ] `just install-v3` + smoke test
- [ ] Manual test: each example app opens at its declared share
- [ ] Commit: `manifest: initial_share field; plumb through HostModel OpenPaneRequest`

**Gate:** File browser + every example app opens at its manifest-declared share.

---

## Phase D — Close the loop: one host test + DEV_LOG + final verify (60 min)

- [ ] Add a HostModel harness test: submit `OpenPane { share_fraction: 0.4 }`, assert the emitted `PaneOpened` effect carries that share
- [ ] DEV_LOG entry tagged `[CHANGED]` with `Breaks if:` — concrete diagnostic: "file explorer opens at 75/25 again (pane_ops 3:1 regression)" and "examples in `examples/*/manifest.toml` open ignoring `initial_share`"
- [ ] Update `CLAUDE.md` Current Queue — check off cutover slice 1, add follow-ups (next commands to route through HostModel: `split_focused`, `close_pane`, `focus_next`)
- [ ] `cargo test` green
- [ ] `pytest` green (the widget tests from 2026-04-18 should still pass unaffected)
- [ ] `just install-v3` + smoke test
- [ ] Launch every example app manually, verify correct shares
- [ ] Final commit (if anything still unstaged): `dev-log: cutover slice 1 [host: launch_app_by_id]`

**Gate:** Six hours done. The pattern is proven. Every future command migration is "do the same thing I did for launch_app_by_id."

---

## What NOT to do this session

- **Do NOT migrate `split_focused`, `close_pane`, `focus_next`, or any other command.** The pattern is proven with one. Scope creep kills the cutover.
- **Do NOT delete `src/app/mod.rs` state or any `pane_ops.rs` functions that other command paths still use.** Only remove the dead branches inside `open_process_app_pane` / `open_builtin_app_pane` that are now unreachable because HostModel handles them.
- **Do NOT touch `src/process_app/`, `src/protocol/`, `src/host/` core files.** HostModel works. Don't refactor what isn't broken.
- **Do NOT merge to `main`.** This stays on `v3` until the full cutover lands across multiple sessions.

---

## If stuck

Common traps and recovery:

- **"HostEffect::PaneOpened doesn't carry enough info to drive egui_tiles."** — Add fields to it. The effect is allowed to grow; it's pure data. Write the harness test first for the new shape.
- **"`src/app/mod.rs` structure is impenetrable."** — Grep for exactly `launch_app_by_id` and work outward. Do not try to understand the whole file. Make the smallest change that routes that one call through HostModel.
- **"The 3:1 hardcode is used by split_focused too."** — Good. Leave that code path alone. This session only touches app-launch. `split_focused` migrates next session.
- **"Smoke test panics after my change."** — `~/.plexi-v3/plexi.log` shows the panic. Fix it or revert — never ship a panic.
- **"Running out of time in Phase C."** — Skip updating every example manifest. Just do file-browser. The fix is still valid; the other manifests stay on the default.

---

## Success criteria (end of session)

1. `cargo test` green (57 Rust + HostModel tests).
2. `pytest sdk/python/tests/` green (81 widget tests).
3. `just install-v3` green with smoke test passing.
4. Launching file-browser opens it at manifest-declared share (not 75/25).
5. At least one other example app verified to open at its declared share.
6. One to four commits on `v3`, all clean.
7. DEV_LOG entry with `Breaks if:` landed.
8. `CUTOVER_PLAN.md` items checked off; any deferred items moved to a `CUTOVER_PLAN_PART_2.md` (or just `## Next session` section appended here).

---

## Resume instructions for a fresh session

1. Read `CLAUDE.md` Current Queue first — confirms where this slice fits.
2. Read this file; check off items as completed.
3. Check `git log --oneline -5` on `v3` — see where the last commit landed.
4. Run `cargo test && just install-v3` to confirm baseline is clean before starting.
5. Start at the first unchecked box.
6. The memory at `~/.claude/projects/-Users-ianburke-Documents-GitHub-PLEXI/memory/feedback_subagent_orchestration.md` is load-bearing: orchestrator owns the ship gate and the commit. If using sub-agents, stage only — do not let sub-agents commit.
