---
name: testing
description: "Agent self-validation before push. Classifies the diff (host logic / host UI / PGAP app), runs the matching harness tests, generates headless render screenshots as evidence, and writes a Test Evidence block for the Ship Log that validate-pr uses to decide whether binary install can be skipped. Invoked inline by implement-issue and implement-stint after cargo build passes, before git push."
---

# Testing — Self-Validation Evidence

Run this after implementation compiles, before pushing. Output is a `**Test evidence:**` block appended to the issue Ship Log entry (or the PR body when there is no linked issue). validate-pr reads that block to decide whether the PR needs a binary install or diff review only.

All commands run from the feature worktree.

## Step 1 — Classify the Diff

```bash
git diff --name-only origin/alpha...HEAD
```

| Touched | Layer | Evidence required |
|---|---|---|
| `src/` host logic (commands, effects, pane ops, CLI) | host | scoped `cargo test --bin plexi` |
| `src/` egui layer (overlays, widgets, render, style) | host-ui | PlexiUiHarness test + screenshot |
| `apps/`, `sdk/python/` | pgap | real-process headless screenshot |
| docs, skills, scripts, config only | none | no evidence block needed — state "docs-only" |

A diff can hit multiple layers; produce evidence for each.

## Step 2 — Host Logic Evidence

Run the test modules nearest the touched code, then the full bin suite:

```bash
cargo test --bin plexi <module_filter>
cargo test --bin plexi
```

Record pass/fail counts and the module filters used. New `AppRequest`/`HostEffect` handlers must have a `HostHarness` test (`src/testing/mod.rs`) — written first, per repo discipline.

## Step 3 — Host UI Evidence (screenshots are headless)

`PlexiUiHarness` (`src/ui_tests.rs`) renders fully headless via wgpu Metal — no display, works in any terminal session. For a new or changed overlay/widget/pane type:

1. **Write a committed test** in `src/ui_tests.rs` — open → step → assert → `save_screenshot()`. Tests are permanent regression coverage, never throwaway blocks that get reverted.
2. Use `PlexiUiHarness::new_sized(1280.0, 800.0)` for screenshot tests so chrome is legible.
3. Save evidence PNGs to `/tmp/plexi-render-<issue>-<name>.png`.

Existing visual smoke tests to reuse:

```bash
# Host app + portal (run in the default suite):
cargo test --bin plexi shot_file_browser
cargo test --bin plexi shot_subcontext_portal
```

Harness drivers available on `PlexiUiHarness`:

- `new_sized(w, h)` — explicit surface size for screenshots
- `open_file_browser(cwd)` — built-in file browser host app
- `push_focused_pane_to_subcontext(name)` — create a subcontext portal
- `open_app_at(app_dir)` — spawn a **real PGAP app process** (manifest + Python child + IPC), returns the pane id
- `wait_for_app_frame(pane_id, timeout)` — block until the app commits its first real frame; errors carry the app's stderr
- `save_screenshot(path)` / `render()` — headless PNG

## Step 4 — PGAP App Evidence (real process, no code change)

Any app directory with a `manifest.toml` can be screenshotted headlessly without writing a test:

```bash
PLEXI_SHOT_APP=apps/<app> PLEXI_SHOT_OUT=/tmp/plexi-render-<issue>-<app>.png \
  cargo test --bin plexi shot_app_from_env -- --ignored --nocapture
```

This spawns the actual Python process through the production launch path (`launch_app_by_path_with_layout`), waits for its first committed frame, and renders. The repo SDK at `sdk/python` is exported automatically via `PLEXI_SDK_PATH`.

Reference example: `cargo test --bin plexi shot_balls_app -- --ignored --nocapture`.

If the app crashes before the first frame, the failure message contains its stderr — fix the app, don't screenshot a fallback state.

## Step 5 — Inspect the Screenshots

Read every generated PNG with the Read tool and confirm it shows the intended state — not an empty pane, error tile, or default fallback. A screenshot nobody looked at is not evidence.

## Step 6 — Write the Evidence Block

Append to the Ship Log entry for this attempt (issue body), or the PR description when there is no linked issue:

```markdown
**Test evidence (attempt <N>):**
- cargo test: <passed> passed, <failed> failed — filters: <module list or "full bin suite">
- PlexiUiHarness render: /tmp/plexi-render-<issue>-<name>.png — <one line: what it shows>
- Conclusion: install skippable — full coverage | binary install required — <why>
```

Conclusion rules:
- `install skippable — full coverage`: every touched layer has green tests AND any visual change has an inspected screenshot.
- `binary install required`: PTY/terminal interaction, keyboard-capture flows, anything `#[ignore = "requires-pty"]`, or behavior only observable in the installed bundle (menus, dock, file associations).

## Guards

- `git status --short` must show no unintended test scaffolding before commit. Committed harness tests are the norm; if you wrote a one-off you don't want to keep, you should have used `shot_app_from_env` instead.
- Never claim `install skippable` for a layer you didn't actually test.
- Screenshot PNGs live in `/tmp` and die with the machine — the evidence block's one-line description is the durable record; keep it specific.
