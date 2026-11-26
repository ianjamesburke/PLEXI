---
name: testing
description: "Agent self-validation before commit. Classifies the diff (host logic / host UI / PGAP app), runs the tests that cover it, and generates headless render screenshots for visible changes. Used by implement-stint-v2 before a PR is opened."
---

# Testing — Self-Validation

Use [TESTING.md — The Gate](../../../src/testing/TESTING.md#the-gate): run the tests that cover what you changed, add one if nothing does, look at the screenshot for visible UI changes. The beta release runs the full suite once.

All commands run from the feature worktree.

## Step 0 — Build and Clippy when Rust changed

```bash
just build
just clippy
```

`just build` is the release build under `-D warnings`, the path `just pr-install` takes. `cargo test` omits `cfg(not(test))` paths and bare `cargo build` does not reject dead-code warnings (root `AGENTS.md` Traps). `just clippy` catches what `just build` does not (`too_many_arguments`, `type_complexity`, `needless_borrow`, ...). Fix the code, never `#[allow]`.

## Flaky Test Policy

**A failing test is never dismissed as "flaky" without proof.** The word "flaky" is a hypothesis, not a finding; treating it as a finding stops investigation prematurely.

Before a test can be called flaky:
1. It must pass twice in isolation: `cargo test --bin plexi <test_name> -- --exact`
2. If it passes in isolation but fails in the full suite → **it has a concurrency bug**. Find and fix the shared state.
3. Even after confirming flakiness, file a stint — don't shrug it off.

Never report "passed in isolation, likely flaky" as a result. Report either a green suite, or the test name and that it fails under parallelism with the stint filed.

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

| Touched | Layer | Check |
|---|---|---|
| `src/` host logic (commands, effects, pane ops, CLI) | host | scoped `cargo test --bin plexi` |
| `src/` egui layer (overlays, widgets, render, style) | host-ui | PlexiUiHarness test + screenshot |
| `plugins/`, `sdk/python/` | pgap | relevant app/SDK checks; inspected render for visible changes |
| docs or skills | docs | relevant documentation checks |
| scripts or config | infrastructure | the script's own self-test or a focused run |

A diff can hit multiple layers; check each.

**Some markdown is code.** Markdown reachable from `include_str!` anywhere under `src/`, shipped or `#[cfg(test)]`, is compiled into the artifact or asserted on by a doc-sync test, so it takes the layer of its including module. Decide with `python3 scripts/docs_gate_scope.py <paths>`: exit 0 means no build or test applies; exit 1 prints one reason per path.

## Step 2 — Host Logic Tests

Run the filters `scripts/scoped-tests.sh` selects, using `just test-host`, which shares test-profile flags with `just scene`. An empty selection skips only the Rust test run; script-specific checks and PGAP/UI checks still apply.

Verify each selected surface has actual matches; zero tests is not a pass. New `AppRequest`/`HostEffect` handlers must have a `HostHarness` test written first. Use `-- --exact` only with the fully-qualified test name.

## Step 3 — UI & App: Scenes and AppHarness

### Host UI — TOML scenes

**TOML scene files are the one way to test observable host UI behavior** — overlays, widgets, portals, and real PGAP app processes through the production launch path. The runner (`src/scenes.rs`) executes them on `PlexiUiHarness`: fully headless wgpu Metal rendering.

```bash
just scene tests/scenes/<name>.toml              # run one scene; PNGs + SceneReport JSON to /tmp/plexi-scenes
just scene <file> /tmp/out 0                     # state-only: skip screenshot steps
bash scripts/cargo-with-lease.sh cargo test --bin plexi scene_suite               # all committed scenes (suite = true)
```

For affected observable behavior, select and run relevant scenes individually; justify when none applies. Escalate to the suite for broadly shared changes as described in TESTING.md.

For a new or changed overlay/widget/pane type: **add a committed scene file** under `tests/scenes/` — it is automatically a regression test. Scenes that spawn real app processes set `suite = false` and run via `just scene`.

```toml
size = [1280.0, 800.0]

[[steps]]
open = { kind = "process", path = "plugins/dev/balls", as = "balls" }
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

**For visible `plugins/` and `sdk/python/` changes, use `AppHarness` for headless PNG validation.** No running host or display required — it pipes draw commands to `plexi --render` via stdin and returns PNG bytes.

```python
from plexi_sdk.testing import AppHarness, render_draw_commands

# Full app test — spawns the app subprocess, drives it over PGAP protocol
with AppHarness("plugins/my_app/app.py", width=800, height=600) as h:
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

**Read every generated PNG with the Read tool** and confirm it shows the intended state (not an empty pane, error tile, or clipped element). A screenshot nobody looked at proves nothing.

## Step 4 — Inspect the Screenshots

Read every generated PNG with the Read tool and confirm it shows the intended state — not an empty pane, error tile, or default fallback. A screenshot nobody looked at proves nothing.

Render realistic seeded content, not an empty state — an empty overlay or a
zero-turn assistant pane cannot show the bug you fixed. `screenshot_assistant_conversation_bubbles`
in `src/ui_tests/tests/assistant.rs` is the pattern: build real model state, open through the
real pane path (`open_assistant_built` / equivalent), screenshot, inspect.

A `LiveBackend` change needs `just scene-live` against an installed PR channel, and `plugins/` changes are only exercised by a real install. If the change needs a person to try it, open a PR and run `just pr-install <N>`; show screenshots in the PR body when they help.

## Guards

- `git status --short` must show no unintended test scaffolding before commit. Committed scene files are the norm; a one-off you don't want to keep belongs outside the repo (`just scene /tmp/<file>.toml` works on absolute paths).
- Never claim a layer was tested when it was not.
