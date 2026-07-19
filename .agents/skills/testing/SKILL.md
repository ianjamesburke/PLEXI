---
name: testing
description: "Agent self-validation before push. Classifies the diff (host logic / host UI / PGAP app), runs the matching harness tests, generates headless render screenshots as evidence, and writes a Test Evidence block for the Ship Log that validate-pr uses to decide whether binary install can be skipped. Invoked inline by implement-issue and implement-stint after cargo build passes, before git push."
---

# Testing — Self-Validation Evidence

Run this after implementation compiles, before pushing. Output is a `**Test evidence:**` block appended to the issue Ship Log entry (or the PR body when there is no linked issue). validate-pr reads that block to decide whether the PR needs a binary install or diff review only.

All commands run from the feature worktree.

## Flaky Test Policy

**A failing test is never dismissed as "flaky" without proof.** The word "flaky" is a hypothesis, not a finding; treating it as a finding stops investigation prematurely.

Before a test can be called flaky:
1. It must pass twice in isolation: `cargo test --bin plexi <test_name> -- --exact`
2. If it passes in isolation but fails in the full suite → **it has a concurrency bug**. Find and fix the shared state.
3. Even after confirming flakiness, file a GitHub issue — don't shrug it off in the Ship Log.

**Ship Log rule:** never write "test passed in isolation, likely flaky" as a conclusion. Write either:
- `cargo test: N passed` (all green) — if the suite is clean
- `binary install required — <test name> fails under parallelism, issue #N filed` — if there is a genuine intermittent

**Concurrency interference was the root cause of issue #2229.** Tests shared `config_dir()` (a real $HOME subdir) across threads. The fix: HostHarness now auto-isolates every test into a per-test tempdir. Any new shared-state interference must be fixed the same way — not dismissed.

## Step 1 — Classify the Diff

```bash
git diff --name-only origin/alpha...HEAD
git diff --name-only HEAD
git ls-files --others --exclude-standard
```

The first command classifies committed branch changes; the latter two include
the current pre-push worktree. Do not classify from `origin/alpha...HEAD` alone
when implementation is still uncommitted.

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
Use `-- --exact` only with the fully-qualified test name; a basename plus
`--exact` can succeed after running zero tests. The full bin suite must be green
in the current attempt; do not substitute an earlier run without recording when
and against which diff it ran.

## Step 3 — UI & App Evidence: Scenes and AppHarness

### Host UI — TOML scenes

**TOML scene files are the one way to test observable host UI behavior** — overlays, widgets, portals, and real PGAP app processes through the production launch path. The runner (`src/scenes.rs`) executes them on `PlexiUiHarness`: fully headless wgpu Metal rendering.

```bash
just scene tests/scenes/<name>.toml              # run one scene; PNGs + SceneReport JSON to /tmp/plexi-scenes
just scene <file> /tmp/out 0                     # state-only: skip screenshot steps
cargo test --bin plexi scene_suite               # all committed scenes (suite = true)
```

For a new or changed overlay/widget/pane type: **add a committed scene file** under `tests/scenes/` — it is automatically a regression test. Scenes that spawn real app processes set `suite = false` and run via `just scene`.

```toml
size = [1280.0, 800.0]

[[steps]]
open = { kind = "process", path = "apps/dev/balls", as = "balls" }
[[steps]]
wait_app_frame = { target = "balls", timeout_s = 15.0 }
[[steps]]
assert = { target = "balls", pane_count = 1, lifecycle = "running", tree_contains = "balls" }
[[steps]]
shot = "balls.png"
```

For the canonical scene DSL reference, coverage map, assertions, and SceneReport
format, see `src/testing/TESTING.md`.

### Python PGAP apps — AppHarness PNG renders

**For `apps/` and `sdk/python/` changes, use `AppHarness` for headless PNG validation.** No running host or display required — it pipes draw commands to `plexi --render` via stdin and returns PNG bytes.

```python
from plexi_sdk.testing import AppHarness, render_draw_commands

# Full app test — spawns the app subprocess, drives it over PGAP protocol
with AppHarness("apps/my_app/app.py", width=800, height=600) as h:
    h.run(1)                             # step one render frame
    h.key("enter")                       # inject a key event
    h.run(1)                             # re-render
    h.save_snapshot("/tmp/snap.png")     # write PNG to disk for visual inspection
    h.assert_pixel(10, 10, "#1e1e2e")   # pixel assertion with default tolerance=4
    h.assert_no_overlap()               # assert no rects clip each other

# Direct render — when you already have a draw command list
png = render_draw_commands(cmds, width=800, height=600)
```

**Every Python app ships with at least one `AppHarness` test** in `tests/apps/<app_name>_test.py`. It must: run one frame, call `save_snapshot`, and assert at least one pixel or `assert_no_overlap`. This is the done condition for visual layout — not "it ran without crashing."

**Read every generated PNG with the Read tool** and confirm it shows the intended state (not an empty pane, error tile, or clipped element). A screenshot nobody looked at is not evidence.

## Step 4 — Inspect the Screenshots

Read every generated PNG with the Read tool and confirm it shows the intended state — not an empty pane, error tile, or default fallback. A screenshot nobody looked at is not evidence.

Render realistic seeded content, not an empty state — an empty overlay or a
zero-turn assistant pane cannot show the bug you fixed. `screenshot_assistant_conversation_bubbles`
in `src/ui_tests.rs` is the pattern: build real model state, open through the
real pane path (`open_assistant_built` / equivalent), screenshot, inspect.

Once inspected, delete the PNG (`rm /tmp/plexi_*.png`). The screenshot is a
review artifact, not evidence to keep — the passing test is what future runs
re-check, and `/tmp` is not the durable record (see Guards).

## Step 5 — Write the Evidence Block

Append to the Ship Log entry for this attempt (issue body), or the PR description when there is no linked issue:

```markdown
**Test evidence (attempt <N>):**
- cargo test: <passed> passed, <failed> failed — filters: <module list or "full bin suite">
- PlexiUiHarness render: /tmp/plexi-render-<issue>-<name>.png — <one line: what it shows>
- Conclusion: install skippable — full coverage | binary install required — <why>
```

**These conclusion rules are the single source of truth for install-skip.** `/implement-stint` and `/validate-pr` consume the conclusion this block produces — they never re-derive it against their own criteria. If you are editing skip criteria anywhere else, you are editing the wrong file.

Conclusion rules:
- `install skippable — full coverage`: every touched layer has green tests AND any visual change has an inspected screenshot. This includes pure-logic and docs-only changes with direct `HostHarness`/`PlexiUiHarness`/scene coverage of the full user-visible behavior — coverage is the test, not cosmetic-ness.
- `binary install required`: PTY/terminal interaction, keyboard-capture flows, anything `#[ignore = "requires-pty"]`, or behavior only observable in the installed bundle (menus, dock, file associations). This is the default for visible UI, keyboard, app-launch, channel, filesystem, host/runtime, or interaction changes.
- `docs-only — no test evidence required`: markdown, comments, or skill/doc text with no code path touched.
- A `LiveBackend` change always requires `just scene-live` against the installed
  PR channel. Headless parity proves schema semantics, not CLI execution or
  eventual polling against a real host.
- `apps/` changes always require binary install regardless of conclusion — the app registry and bundle layout are only exercised by a real install.

## Guards

- `git status --short` must show no unintended test scaffolding before commit. Committed scene files are the norm; a one-off you don't want to keep belongs outside the repo (`just scene /tmp/<file>.toml` works on absolute paths).
- Never claim `install skippable` for a layer you didn't actually test.
- Screenshot PNGs live in `/tmp` and die with the machine — the evidence block's one-line description is the durable record; keep it specific.
