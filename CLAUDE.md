Always confirm best practices by researching the docs.

## Source of Truth for Project State

**CLAUDE.md does not track in-progress work or completion status.** It goes stale immediately and will mislead future sessions.

- **What shipped and why** → `DEV_LOG.md` (read the first 100 lines at session start)
- **What's currently in flight** → `git log --oneline -20` and `git status`
- **What's planned** → `.plexi/backlog`

Before reporting anything as "done" or "missing", verify against `git log`. Never trust a status list in this file.

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

- `alpha` — active development. All PRs land here.
- `beta` — staging/release channel. Promoted from alpha when ready.
- `main` — stable releases only.

Feature branch naming: `feature/<issue-number>-short-description`. Sub-agent workflow: `isolation: "worktree"` off `alpha`, PR back to `alpha`. Never push directly to `main` or `beta`.

## GitHub Issue Labels

Every issue gets one **type**, one **priority**, one **version**.

- **type:** `bug` | `enhancement` | `idea`
- **priority:** `P1` (shipping blocker) | `P2` | `P3` | `P4`
- **version:** `v3.0` | `v3.1+` | `future`
- **status** (optional): `in-progress` | `ready` | `blocked`

## Milestones

Milestones (`v3.1`, `v3.2`, `v3.3`, …) define the actual release collections — the specific dot releases that ship work. They are distinct from version era labels:

- **Version era label** (`v3.1+`) — "this belongs in the v3.x era." Accepted but not yet slotted.
- **Milestone** (`v3.2`) — "this is committed to that specific release sprint."

Issues without a milestone are accepted into an era but unslotted. Assign a milestone when the work is actively being planned for a release. An issue can carry `v3.1+` as its version label and a `v3.4` milestone simultaneously — the label is the era, the milestone is the slot.

## App Installation Paths

Build-specific, resolved at runtime by binary name:

| Build | Apps directory |
|---|---|
| Alpha (frozen) | `~/.plexi-alpha/apps/` |
| Beta | `~/.plexi-beta/apps/` |
| Stable | `~/.plexi/apps/` |
| v3 dev build | `~/.plexi-v3/apps/` |

Each app is a subdirectory with `manifest.toml` and an executable entry point. Installing to the wrong directory silently does nothing.

## Branch Workflow

- `alpha` — active development. All PRs land here. Use `just install-alpha` to test.
- `beta` — staging/release channel. Promoted from alpha when ready. Use `just install-beta` to test.
- `main` — stable releases only.

Feature branches are cut from `alpha`, worked in `worktrees/`, and merged back to `alpha` via PR. Never commit directly to `main` or `beta`.

**Always run `wtp add` from inside `worktrees/alpha/`**, not from the repo root or any other worktree. This ensures the new branch forks from alpha's current HEAD — including uncommitted changes — so PRs merge cleanly back to alpha. Cutting from main silently orphans in-flight work.

**Before creating a worktree:** check `git status` in `worktrees/alpha`. If there are uncommitted changes, ask the user whether to commit them to alpha first or carry the dirty HEAD into the new branch. Never silently drop uncommitted work.

**Full done cycle (every PR, no exceptions):**
1. Commit and push the feature branch
2. Open a PR targeting `alpha`
3. Wait for user approval — do not merge unilaterally
4. Squash-merge the PR into `alpha` via `gh pr merge <number> --squash --delete-branch`
5. Pull the merged commit into `worktrees/alpha`: `git pull` from inside `worktrees/alpha/`
6. Run `just install-alpha` from inside `worktrees/alpha/` to confirm the build is clean
7. Remove the feature worktree: `wtp remove <branch-name>`

Steps 4–7 are mandatory. Skipping them is how uncommitted work gets silently lost on the next merge.

**Small-changes batch PR:** When a session produces multiple small, related fixes that each touch different areas but share a single theme (e.g. "polish pass", "welcome screen tweaks"), they may land in one PR. Rule of thumb: if they can share one commit message and one `Breaks if:` line, they belong together. If they're unrelated or could conflict, keep them separate.

Worktrees:
- `worktrees/alpha` — alpha branch
- `worktrees/beta` — beta branch

## Releases

Before tagging a release (`just bump` + `just release`):
1. Update `CHANGELOG.md` at the repo root — add a new `## [x.y.z] — YYYY-MM-DD` section with a brief summary of what changed (features, fixes, breaking changes).
2. Entries are newest-first. Keep them user-facing (not internal refactor detail).

If `CHANGELOG.md` doesn't exist yet, create it with a header comment and the first entry.

## Build & Install

`just install` runs `cargo bundle --release`, copies the `.app` to `/Applications`, extracts the binary to `/usr/local/bin/plexi`, then runs `lsregister -f <bundle>` and `pbs -update` to refresh macOS Services.

**After every completed code change, install for the active branch** before reporting the task complete:
- `alpha` → `just install-alpha` (run from inside `worktrees/alpha/`, not the repo root — the recipe builds from CWD, so running from the wrong directory installs from the wrong branch)
- `main` → `just install`

**Never claim a task complete based on an install from a feature worktree.** `just install-alpha` builds from source files in CWD — uncommitted changes compile and install successfully, making the task appear done when nothing has been committed. Installing from a feature worktree is only valid during development iteration. The full done cycle is: commit → push → PR → squash-merge to alpha → `git pull` in `worktrees/alpha/` → `just install-alpha` from `worktrees/alpha/`.

## Logging

Build-specific log file:
- Alpha: `~/.plexi-alpha/plexi.log`
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

## Host UI Systems — Reuse Before Rolling Your Own

Before writing any keyboard shortcut display, badge, chip, or inline label widget, check `src/widgets.rs` and `src/style.rs`. These modules contain the canonical, already-tested implementations. Re-rolling them inline produces visual inconsistency and duplicated sizing logic.

**`src/style.rs`** — design tokens: spacing scale (`SPACE_SM/MD/XL`), typography scale (`TEXT_HINT/CAPTION/BODY/TITLE_XL`), corner radii (`RADIUS_MD/LG`), modal widths, button heights, overlay chrome. Use these constants everywhere — never hard-code magic numbers.

**`src/widgets.rs`** — reusable egui widgets:
- `key_chip(ui, label, colors)` — renders a single keyboard key as a styled rounded-rect chip (`bg_active` fill, `border` stroke, `TEXT_HINT`-size monospace text).
- `key_combo(ui, keys, colors)` — renders a sequence of `key_chip`s with `INTRA_COMBO_GAP` between them (e.g. `["⌘", "N"]` → `[⌘][N]`).
- `key_combo_list(ui, combos, trailing, colors)` — renders multiple combos inline with `INTER_COMBO_GAP` between them and an optional dim description label at the end. This is the standard pattern for keyboard shortcut hint rows.

**Use `key_combo_list` for any shortcut hint row.** Do not render key shortcuts as plain `Label` text — it produces a visually inconsistent result that requires a separate pass to fix.

## General Rules

- Before SSH/networking setup, ask if machines are on the same LAN or remote. Before any multi-step infra task, clarify topology first.
- When the user reports a bug, fix what they asked for first. Don't pivot to QA, refactoring, or tangential improvements until the primary request is resolved.
- When the user provides multiple distinct ideas, file them separately. Don't combine unrelated concepts.
- Never use `#[allow(dead_code)]` or `#[allow(unused)]`. Always do the work: delete unused code, wire it up, or move it to a feature-flagged module. If fixing a warning takes a long time, that's the job — do not paper over it with an allow attribute.
- always run cargo build after work to make sure it passes.

