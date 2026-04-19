Always confirm best practices by researching the docs.

## Current Queue

> This section is live. Items are added when scoped, deleted when shipped. Nothing here is permanent — it's just what's next. When an item ships, remove it and log the decision in DEV_LOG.

- [x] **Headless PNG renderer** — `src/headless_renderer.rs`. Done: 3 tests pass (rect pixel assertion, text no-panic, document blank frame). No egui dependency.
- [x] **HostModel rebuild** — Done: 26 host tests green (13 harness, 5 model, 8 pre-existing). Full command/effect set. No `todo!()`. Groups, capabilities, contexts, navigation all covered.
- [x] **Wire harness** — Done: `Harness::render_to_png` wired. `agent_dev_loop_produces_png` test spawns snake, renders frame, asserts valid PNG with visible pixels.
- [x] **inject_state + net.http brokering** — Done: 79 tests green. `PlexiEvent::InjectState`, `http_request`/`http_response` PGAP channel, `Harness::inject_state` + `mock_http` + pre-drain race fix, `emit.http_get()` in SDK. Wikipedia testable via inject_state (no key-pushing) and via mock_http (no network).
- [x] **Cutover slice 1: launch_app_by_id → HostModel** — Done: `PlexiApp` holds `host: HostModel`, launch path routes `HostCommand::OpenPane` through `HostModel` and reads `(share, placement)` from the returned `PaneOpened` effect. `pane_ops::split_with_new_pane` drops the 3:1 hardcode, takes a `ShareRatio`. Manifest `initial_share` field wired with per-app defaults. 58 Rust tests green.
- [x] **Cutover slices 2/3/4: split + close + navigate → HostModel** — Done: every user-facing pane op (launch, split, close, navigate) now flows through `HostCommand`. Slice 2 consumes `SplitOpened.placement`; slices 3/4 are observation-only until ID reconciliation is done.

**V3 refactor plan** (`V3_REFACTOR_PLAN.md`) — 12 steps across 5 phases. Progress:
- [x] **Step 1 — Dead code sweep** (582 LOC deleted, scaffolder migrated, module allows annotated)
- [x] **Step 2 — Unify dual types** (single Direction, 9-cap Capability, TryFrom errors, InjectState/HttpResponse/Image/media variants added)
- [x] **Step 3 — Finish or delete stubs** (plexi_iq prompt/tools deleted, context simplified, video-player removed from ship set)
- [x] **Step 4 — Pane ID reconciliation** (HostModel owns alloc; PlexiApp::next_pane_id deleted; workspace seed/restore)
- [x] **Step 5 — HostServices trait objects** (fs/secrets/net/spawn + mocks; `HostServices::mock()` for Layer-2 tests; STEP-9 still owns production wiring of routing.rs through services.secrets)
- [x] **Step 6 — FileEventSink live** (`effects.jsonl` append-only per-effect log wired into `HostServices::new()`; consumer-side nav/close rewiring deferred to STEP-9)
- [x] **Step 7 — Capability enforcement complete** (install-time manifest validation; runtime checks on HttpRequest/PipeSend/AudioPlay/AudioCapture; 403 HttpResponse on net.http denial)
- [x] **Step 8 — Manifest schema freeze** (new `[launch]` section with structured `layout_hint = { side, split }`; install-time validator; 5 examples + scaffolders migrated)
- [x] **Step 9 (partial) — env isolation + bold text + AppSpawned SDK hook**; HTTP broker, PipeSend peer routing, RunUpdate round-trip, media brokers, FD CLOEXEC audit all deferred to a follow-up session
- [x] **Step 10 — Real Rust Layer-1 tests + uv runner** (5 `#[test]` fns in pgap_test_harness; pyproject.toml wires `uv run pytest`)
- [ ] Step 11 — CI gate that enforces
- [ ] Step 12 — Invariant enforcement

## North Star

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — vision, target architecture diagram, key invariants. Read first.
- [`docs/specs/releases/plexi-v3.0.md`](docs/specs/releases/plexi-v3.0.md) — the v3 spec. Single source of truth for the protocol, pane ADT, secrets invariant, media, Plexi IQ, example apps.
- [`docs/specs/README.md`](docs/specs/README.md) — spec index.
- [`docs/specs/subsystems/host-architecture.md`](docs/specs/subsystems/host-architecture.md) — HostModel state machine, renderer layer, security model, WASM path, multi-agent.
- [`docs/specs/subsystems/testing-infrastructure.md`](docs/specs/subsystems/testing-infrastructure.md) — three-layer test strategy: app protocol, host state machine, headless PNG renderer.
- [`docs/AGENTS.md`](docs/AGENTS.md) — agent development guide: build, test, install, commit rules.

Vision (why we're building this, long-term direction) lives in `ARCHITECTURE.md §0`.

If a doc outside these contradicts them, the doc is wrong. Fix or delete it.

## Terminology

**PGAP** — Plexi Generic App Protocol. Newline-delimited JSON over stdin/stdout. `PlexiEvent` flows host→app, `DrawCommand` flows app→host. Binary data (audio PCM, video frames, raw bytes) travels on typed pipes, not stdio. PGAP is the isolation boundary — no shared memory, no inherited FDs.

## Branches

- `main` — stable releases.
- `beta` — staging.
- `alpha` — frozen v2.x tree, tagged `v2-last`, retired. Do not land new work here.
- `v3` — active development for the v3.0 clean cut. All feature branches cut from `v3`, worked in `.claude/worktrees/`, merged back via PR.

Feature branch naming: `feature/<issue-number>-short-description`. Sub-agent workflow: `isolation: "worktree"` off `v3`, PR back to `v3`. Never push directly to long-lived branches.

## GitHub Issue Labels

Every issue gets one **type**, one **priority**, one **version**.

- **type:** `bug` | `enhancement` | `idea`
- **priority:** `P1` (shipping blocker) | `P2` | `P3` | `P4`
- **version:** `v3.0` | `v3.1+` | `future`
- **status** (optional): `in-progress` | `ready` | `blocked`

## App Installation Paths

Build-specific, resolved at runtime by binary name:

| Build | Apps directory |
|---|---|
| Alpha (frozen) | `~/.plexi-alpha/apps/` |
| Beta | `~/.plexi-beta/apps/` |
| Stable | `~/.plexi/apps/` |
| v3 dev build | `~/.plexi-v3/apps/` |

Each app is a subdirectory with `manifest.toml` and an executable entry point. Installing to the wrong directory silently does nothing.

## Build & Install

`just install` runs `cargo bundle --release`, copies the `.app` to `/Applications`, extracts the binary to `/usr/local/bin/plexi`, then runs `lsregister -f <bundle>` and `pbs -update` to refresh macOS Services.

**After every completed code change, install for the active branch** before reporting the task complete:
- `v3` → `just install-v3`
- `main` → `just install`

## Logging

Build-specific log file:
- v3: `~/.plexi-v3/plexi.log`
- Stable: `~/.plexi/plexi.log`

Rotates to `plexi.log.1` at 10 MB. Level set in `config.toml` (`error | warn | info | debug`). Third-party crates clamped to `warn`.

App logs forward into the host log tagged `app::<app_id>`. Python SDK: `ctx.info/warn/error/debug(...)` inside a frame; `emit.info(...)` outside. App stderr forwards as `warn`-level `app::<app_id>` entries.

**When debugging, check the log file first.**

## Configuration Philosophy

Required fields have no defaults — fail fast with a clear error. Optional fields are clearly marked. Never paper over ambiguity with invisible magic. Prefer a verbose generated config with all options visible over a sparse one with hidden behavior.

## Python Tooling

Use `uv` for all Python projects. `pyproject.toml` with `requires-python = ">=3.11"`, `uv sync`, `uv run`. Bootstrap with `curl -LsSf https://astral.sh/uv/install.sh | sh` if absent. Never write manual venv creation loops.

## Error Handling

Try-catch on all I/O, network, external API calls, and anything that can reasonably fail. Every catch logs where + what failed with enough context to diagnose. Never swallow errors silently. If a failure can't be meaningfully recovered from, propagate or re-throw.

## Implementation Discipline (no half-refactors)

**Define done by the test, not the code.** Before writing any new module or refactoring an existing one, write the test that must pass when the work is complete. A PR is done when `cargo test` is green — not when the code looks right.

**Test-first for host logic.** Any new `HostCommand` or `HostEffect` gets a `HostHarness` test written before the implementation. The test failing is the starting state; making it pass is the work. This prevents stubs: a stub that makes the test pass is an implementation.

**No partial merges.** A PR that adds a new capability, module, or feature must be complete end-to-end. If it's too large to complete in one pass, scope it down — don't merge half of it. Split at natural seams where each piece is independently testable and independently useful.

## Panic Discipline (stubs must not crash the host)

`todo!()` and `unimplemented!()` are **banned outside `#[cfg(test)]`** — enforced by `#![deny(clippy::todo, clippy::unimplemented)]` in `src/main.rs`. They compile clean but panic at runtime, and a panic on the UI thread freezes the whole GUI.

**Factory rule:** any impl returned from a factory function (e.g. `audio_device()`, `video_decoder()`) must never panic in a trait method. Unimplemented methods return `Err(NotImplemented)` / `None` / noop — never `todo!()`. When you add a new prod stub, add a `prod_stub_tests` unit test that calls every trait method and asserts no panic.

**Post-install smoke test:** `just install-v3` runs `scripts/smoke-test.sh`, which (1) feeds a PGAP Init to every installed app and asserts `ready` appears within 3s, (2) launches the host for 2s and scans the log for panics. If the smoke test fails, the install is broken — do not report the task complete.

## Lessons Carried Into v3

- **Python version in GUI app bundles:** macOS GUI bundles do NOT inherit shell PATH. `#!/usr/bin/env python3` → Apple's frozen `/usr/bin/python3` 3.9.6. Always add `from __future__ import annotations` as the first line of every app Python file so `str | None` is safe on 3.7+.
- **Install doesn't chmod:** `just install-*` syncs files but doesn't set executable bits. Run `chmod +x ~/.plexi-*/apps/*/*.py` after install, or fix the recipe.
- **Coupled state:** When adding state that derives from or shadows existing state, grep every mutation site of the original and update each one.
- **Fallback chain audit:** When a value looks correct on the surface but behavior is stale, enumerate every fallback source in priority order (cookies, env vars, caches, defaults). Fix the chain, not the surface.
- **Model ID verification:** Never guess versioned model IDs. Use only confirmed-current family IDs. A 400/404 surfaces only at call time.

## General Rules

- Before SSH/networking setup, ask if machines are on the same LAN or remote. Before any multi-step infra task, clarify topology first.
- When the user reports a bug, fix what they asked for first. Don't pivot to QA, refactoring, or tangential improvements until the primary request is resolved.
- When the user provides multiple distinct ideas, file them separately. Don't combine unrelated concepts.
- never write allow dead of code. alway do the work to clean the code base.
- always run cargo build after work to make sure it passes.

