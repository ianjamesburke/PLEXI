Always confirm best practices by researching the docs.

## Source of Truth for Project State

**CLAUDE.md does not track in-progress work or completion status.** It goes stale immediately and will mislead future sessions.

- **What shipped and why** → `DEV_LOG.md` (read the first 100 lines at session start)
- **What's currently in flight** → `git log --oneline -20` and `git status`
- **What's planned** → GitHub issues

Before reporting anything as "done" or "missing", verify against `git log`. Never trust a status list in this file.

## North Star

- [`GLOSSARY.md`](GLOSSARY.md) — shared vocabulary. Refer here when terms like "pane," "context," "PGAP," "capability," "secret" are new to you.

## Terminology

See [`GLOSSARY.md`](GLOSSARY.md) for the full shared vocabulary — context, pane, PGAP, capability, secret, etc.

When a DEV_LOG entry introduces or significantly changes terminology, update `GLOSSARY.md` in the same commit. Keep it brief — a one-line addition is enough.

## Branches

**`alpha` is the starting branch for all changes.** Every feature branch, worktree, and PR originates from alpha. Never branch from `main` or `beta`.

- `alpha` — active development. All PRs land here.
- `beta` — staging/release channel. Promoted from alpha when ready. Used for rigorous testing before promotion to main.
- `main` — stable releases only.

Feature branch naming: `feature/<issue-number>-short-description`. Never push directly to `main` or `beta`.

## GitHub Issue Labels

Every issue gets one **type**, one **priority**, one **version**.

- **type:** `bug` | `enhancement` | `idea`
- **priority:** `mit` → `P1` → `P2` → `P3` → `P4`
- **version:** `v3.0` | `v3.1+` | `future`
- **status** (optional): `in progress` | `testing` | `ready` | `blocked`

**Priority definitions:**
- `mit` — Most Important Thing. The single highest-priority issue at this moment. Only one `mit` exists at a time. Usually also tagged `P1`. When `/ship mit` is invoked, this is what gets picked.
- `P1` — Shipping blocker or severe user-facing bug. Must be resolved before the next release. `mit` issues typically carry this too.
- `P2` — Important but not blocking a release. Should happen in the near term.
- `P3` — Nice to have. Polish, ergonomics, or low-urgency improvements.
- `P4` — Backlog. Good ideas that aren't a current priority.

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

Three channels, each more stable than the last:

- `alpha` — active development. All work lands here first.
- `beta` — staging. Promoted from alpha when a batch of work is stable enough to share.
- `main` — production. Promoted from beta when ready to release.

Never commit directly to `beta` or `main`. All work flows through alpha.

### Feature branch → alpha

All changes, no matter how small, follow this cycle:

0. **Label the issue** `in progress`: `gh issue edit <number> --add-label "in progress"`
1. **Create a worktree** from inside `worktrees/alpha/`: `wtp add -b <branch-name>`
2. **Implement** and commit inside that worktree
3. **Open a PR** targeting `alpha`: `gh pr create --base alpha`
4. **Wait for user approval** — do not merge unilaterally
5. **Squash-merge**: `gh pr merge <number> --squash` — lands one clean commit on `origin/alpha`. **Never pass `--delete-branch`** — git refuses to delete a branch checked out by a worktree.
6. **Sync alpha**: `git pull` from inside `worktrees/alpha/`
7. **Close related issue(s)**: `gh issue close <number> --comment "Closed by PR #<pr>"`
8. **Update DEV_LOG.md** and commit the update — this must happen before bump-and-install so alpha stays clean at the end
9. **Bump and install**: `just bump-and-install` from inside `worktrees/alpha/`
10. **Remove the feature worktree**: `wtp remove <branch-name>`
11. **Delete the remote branch**: `git push origin --delete <branch-name>`

**Always run `wtp add` from inside `worktrees/alpha/`**, not the repo root. This ensures the branch forks from alpha's current HEAD so PRs merge cleanly. Cutting from main silently orphans in-flight work.

**Verify the base immediately after `wtp add`** — before delegating any work to a subagent or writing any code, confirm the new worktree is on the right commit:
```bash
git -C worktrees/<new-branch> log --oneline -1   # must match ↓
git -C worktrees/alpha log --oneline -1
```
If they don't match, delete the worktree and branch immediately and redo from inside `worktrees/alpha/`. Discovering the wrong base after a subagent has run wastes the entire run.

**Before creating a worktree:** run `git status` AND `git diff --stat` in `worktrees/alpha`. Alpha must be clean (no uncommitted changes, no unstaged diffs) before branching. If it's dirty, stop and ask: commit first, or is this work meant to be carried into the new branch? Never silently proceed on a dirty base.

**Small-changes batch PR:** Multiple small related fixes may land in one PR if they share a single commit message and a single `Breaks if:` line. Unrelated changes go in separate PRs.

### alpha → beta → main (channel promotion)

When alpha has stabilised enough for broader testing:
```
git push origin alpha:beta
```
Run from `worktrees/alpha/` (or anywhere with origin access). This fast-forwards beta to alpha's current HEAD. Then `just install` from `worktrees/beta/` to verify the beta build.

When beta is ready to ship as a release:
```
git push origin beta:main
```
Then tag the release: `just bump` + `just release`. Update `CHANGELOG.md` before tagging.

Worktrees:
- `worktrees/alpha` — alpha branch
- `worktrees/beta` — beta branch

## Releases

Before tagging a release (`just bump` + `just release`):
1. Update `CHANGELOG.md` at the repo root — add a new `## [x.y.z] — YYYY-MM-DD` section with a brief summary of what changed (features, fixes, breaking changes).
2. Entries are newest-first. Keep them user-facing (not internal refactor detail).

If `CHANGELOG.md` doesn't exist yet, create it with a header comment and the first entry.

## Build & Install

`just bump-and-install` is the standard post-merge command — bumps the alpha version first, then builds and installs. Always run from inside `worktrees/alpha/`.

`just install` alone is for re-installing without a version bump (e.g. after editing CHANGELOG or config without a code change). Run from inside `worktrees/alpha/`.

**Never claim a task complete based on an install from a feature worktree.** Uncommitted changes compile and install successfully, making the task appear done when nothing has been committed. The full done cycle is: commit → PR → squash-merge to alpha → `git pull` in `worktrees/alpha/` → `just bump-and-install` from `worktrees/alpha/`.

## Logging

Build-specific log file:
- Alpha: `~/.plexi-alpha/plexi.log`
- Stable: `~/.plexi/plexi.log`

Rotates to `plexi.log.1` at 10 MB. Level set in `config.toml` (`error | warn | info | debug`). Third-party crates clamped to `warn`.

App logs forward into the host log tagged `app::<app_id>`. Python SDK: `ctx.info/warn/error/debug(...)` inside a frame; `emit.info(...)` outside. App stderr forwards as `warn`-level `app::<app_id>` entries.

**When debugging, check the log file first.**

**Every new feature must be instrumented.** Logging is not optional polish — it's the first diagnostic tool when something breaks:
- **Host (Rust):** `log::info!` at entry points for new `HostCommand`/`HostEffect`/`DrawCommand` handlers and any user-visible state change.
- **Apps/SDK:** `ctx.info()` or `emit.info()` at meaningful state transitions (init, key actions, errors).
- **CLI:** log the resolved command and any path it acted on.

No new capability, command, or user-visible behavior ships without at least one `info`-level trace that confirms it ran.

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

## Lessons Carried Into v3

- **Python version in GUI app bundles:** macOS GUI bundles do NOT inherit shell PATH. `#!/usr/bin/env python3` → Apple's frozen `/usr/bin/python3` 3.9.6. Always add `from __future__ import annotations` as the first line of every app Python file so `str | None` is safe on 3.7+.
- **Coupled state:** When adding state that derives from or shadows existing state, grep every mutation site of the original and update each one.
- **Fallback chain audit:** When a value looks correct on the surface but behavior is stale, enumerate every fallback source in priority order (cookies, env vars, caches, defaults). Fix the chain, not the surface.
- **Model ID verification:** Never guess versioned model IDs. Use only confirmed-current family IDs. A 400/404 surfaces only at call time.
- **Uncommitted bump on alpha:** When alpha shows a dirty Cargo.toml with a version change, `just bump` ran without its commit — commit manually as `chore: bump alpha to X.Y.Z` before creating a worktree.
- **Platform behavior validation:** Before implementing any macOS-specific behavior (menu lifecycle, bundle naming, eframe/winit callback order), add a throwaway `log::info!()` to observe the actual runtime value on the first frame. Never assume which callback fires when or what a property returns — observe first, then code.
- **Egui pointer state during macOS file drags:** `ui.rect_contains_pointer()` and `i.pointer.hover_pos()` are stale during macOS OS-level file drags — winit only updates them from its own events, not the drag-tracking run loop. Never gate drop-target detection on egui pointer checks; test for drop event presence alone. When the drop target is unambiguous (e.g. a single zoomed pane covering the whole overlay), drop the check entirely.

## PlexiApp State

`PlexiApp` fields are declared in `src/app/mod.rs`. There are exactly two struct-literal initialization blocks — both contain `renaming_window: None` and are the only places new fields need to be initialized.

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
- Always run `cargo build` after work to make sure it passes.
- **Failed PR reset:** If a PR fails its first test pass and the diff is under ~1000 lines: close the PR without merging, revert the worktree to clean, comment on the original issue (what was tried, what failed, why), re-label the issue `ready`, and start a fresh agent with only the updated issue as context. Don't patch a broken attempt — start clean.
