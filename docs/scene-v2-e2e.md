# Scene v2 — Eventual Asserts, Geometry Invariants, Failure Artifacts

Status: active
Stint: 0384

## Why

PR #2386 shipped through a full automated validation pass — 10 semantic checks, all green — while two user-visible regressions sailed through untouched: arrow keys in Breakout were completely dead, and Balls rendered at the wrong size inside its pane. Both facts were *present in `pane state` the whole time*. Nothing asserted them.

The scene layer today validates **what an app claims to render**, not **what a user experiences**. Playwright wins on exactly this: auto-waiting effect assertions, geometry-aware expectations, and a failure bundle that shows you what happened without re-running. Scene v2 closes those three gaps using the existing `pane state` seam. No new architecture.

## Destination state

Three additions to the scene runner (`src/scenes.rs`), each usable headless (`scene_suite`) and live (`just scene-live`):

### 1. Eventual asserts — input round-trip verification

A new step verb that injects input and **polls the semantic tree until an expected change appears**, failing with a before/after diff on timeout:

```toml
[[steps]]
expect = { target = "breakout", after_key = "left", node_changes = "paddle", timeout_s = 2.0 }
```

Semantics:
- Snapshot the target's semantic tree, inject the input (`after_key` / `after_text`), then poll (bounded interval, same eventual-polling machinery `scene-live` already has) until a node matching `node_changes` differs from the snapshot.
- On timeout: fail with code `input_no_effect`, carrying the unchanged node's before-state. Dead keyboard delivery becomes an automatic, named failure instead of an invisible pass.
- Plain `expect` without `after_*` is also valid: poll until `tree_contains` / node predicate holds (Playwright's `expect(locator)` equivalent), replacing fixed `run_steps`-then-`assert` timing guesses.
- Headless backend polls across harness frames; live backend reuses the existing bounded-eventual-poll + stable-state barrier. One schema, both backends — same rule as every other verb.

### 2. Geometry invariants

Structured assertion keys on canvas/pane geometry, evaluated against the semantic tree's committed dimensions:

```toml
[[steps]]
assert = { target = "balls", fit = "fill" }          # content rect covers the pane rect (within tolerance)

[[steps]]
assert = { target = "tetris", fit = "contain", aspect = [10, 20] }  # letterboxed, aspect preserved
```

- `fit = "fill"` — committed content dimensions cover the pane rect (tolerance for chrome/padding, single tolerance constant, not per-scene).
- `fit = "contain"` — content fits inside the pane with aspect ratio preserved; fails on non-uniform stretch.
- `aspect = [w, h]` — content aspect ratio matches within tolerance, independent of fit mode.
- These are exactly the two bugs of PR #2386 expressed as one-line invariants. Both Balls (`fill`) and Tetris/Snake (`contain`) get scenes asserting their mode, so a future transform change that flips one breaks CI, not the user.
- Requires the host canvas transform to expose its computed fit decision (or enough geometry to derive it) in `pane state` — extend the semantic state schema if the current fields are insufficient. Schema bump follows the existing versioning rule (schema v2 → v3), no compat shim.

### 3. On-failure artifact bundle

When any step fails, the runner writes a per-failure bundle next to the SceneReport:

```
<out>/<scene>.failure-<step_idx>/
  screenshot.png        # headless: framebuffer shot; live: skipped (no sanctioned live shot — note in manifest)
  semantic.json         # full semantic tree of every open handle at failure time
  log-tail.txt          # last 200 lines of the channel/harness log
  manifest.json         # step index, verb, failure code+message, poll history for eventual asserts
```

- Poll history in the manifest (each poll's timestamp + observed value) is the trace-viewer equivalent: you see *what the runner saw while waiting*, not just "timed out".
- SceneReport gains a `failure_bundle` path field per failed step. Schema version bumps.
- Bundles are artifacts, never assertions — the pixel-vs-state rule from TESTING.md stands.

## Non-goals

- Recorder/codegen (turning human interaction into scenes) — separate future PRM; do not start it here.
- Whole-host live input seam (`target = "host"` live) — separate gap, out of scope.
- Video capture, network-interception analogs, retry orchestration — not now.

## Acceptance (destination, not checklist)

- A scene with `expect.after_key` against Breakout fails with `input_no_effect` when key delivery is broken, passes when the paddle moves — verified both headless and via `scene-live` against an installed channel.
- Scenes asserting `fit = "fill"` (Balls) and `fit = "contain"` (Tetris, Snake) are committed to `tests/scenes/` and run in `scene_suite`.
- Any failing scene produces a failure bundle whose manifest includes poll history; `SceneReport` references it.
- `src/testing/TESTING.md` documents all three additions (verbs table, assertion keys, report shape). This PRM is deleted in the PR that closes its stint.
