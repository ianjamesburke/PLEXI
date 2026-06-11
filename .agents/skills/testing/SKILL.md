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

## Step 3 — UI & App Evidence: Scenes

**TOML scene files are the one way to test observable behavior** — host UI, host apps, portals, and real PGAP app processes. The runner (`src/scenes.rs`) executes them on `PlexiUiHarness`: fully headless wgpu Metal rendering, real child processes through the production launch path.

```bash
just scene tests/scenes/<name>.toml              # run one scene; PNGs + SceneReport JSON to /tmp/plexi-scenes
just scene <file> /tmp/out 0                     # state-only: skip screenshot steps
cargo test --bin plexi scene_suite               # all committed scenes (suite = true)
```

For a new or changed overlay/widget/pane type/app: **add a committed scene file** under `tests/scenes/` — it is automatically a regression test. Scenes that spawn real app processes set `suite = false` and run via `just scene`.

```toml
size = [1280.0, 800.0]

[[steps]]
open_app = "apps/balls"          # real Python process; args = [...] surface as ctx.args
[[steps]]
wait_app_frame = { timeout_s = 15.0 }
[[steps]]
assert = { pane_count = 1, lifecycle = "running", tree_contains = "balls" }
[[steps]]
shot = "balls.png"
```

Verbs: `open_app` (+`args`), `open_file_browser`, `key`, `sidebar`, `switch_context`, `push_to_subcontext`, `wait_app_frame`, `run_steps`, `assert` (structured keys: `pane_count`, `window_count`, `context_count`, `portal_count`, `sidebar`, `lifecycle`, `tree_contains`), `shot`.

The `SceneReport` JSON (`schema_version` field) contains per-step results, a host state snapshot, and the app's committed L1 render tree — assert on state, not pixels, whenever possible. If an app crashes before its first frame, the report carries its stderr — fix the app, don't screenshot a fallback state.

## Step 4 — Inspect the Screenshots

Read every generated PNG with the Read tool and confirm it shows the intended state — not an empty pane, error tile, or default fallback. A screenshot nobody looked at is not evidence.

## Step 5 — Write the Evidence Block

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

- `git status --short` must show no unintended test scaffolding before commit. Committed scene files are the norm; a one-off you don't want to keep belongs outside the repo (`just scene /tmp/<file>.toml` works on absolute paths).
- Never claim `install skippable` for a layer you didn't actually test.
- Screenshot PNGs live in `/tmp` and die with the machine — the evidence block's one-line description is the durable record; keep it specific.
