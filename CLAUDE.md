Always confirm best practices by researching the docs.

## Source of Truth for Project State

**CLAUDE.md does not track in-progress work or completion status.** It goes stale immediately and will mislead future sessions.

- **What shipped and why** → `git log --oneline -20` and `GOTCHAS.md` for non-obvious discoveries
- **What's currently in flight** → `git status`
- **What's planned** → GitHub issues
- **What to dispatch next** → GitHub Project board #7 "Up Next" column (query: `gh project item-list 7 --owner ianjamesburke --format json | jq '.items[] | select(.status == "Up Next")'`)

Before reporting anything as "done" or "missing", verify against `git log`. Never trust a status list in this file.

## North Star

Before making architectural decisions, read [`NORTH_STAR.md`](NORTH_STAR.md) for product direction and [`GLOSSARY.md`](GLOSSARY.md) for shared vocabulary (pane, context, PGAP, capability, secret, etc.).

## Branches

**`alpha` is the starting branch for all changes.** Every feature branch, worktree, and PR originates from alpha. Never branch from `main` or `beta`.

- `alpha` — active development. All PRs land here.
- `beta` — staging/release channel. Promoted from alpha when ready. Used for rigorous testing before promotion to main.
- `main` — stable releases only.

Feature branch naming: `feature/<issue-number>-short-description`. Never push directly to `main` or `beta`.

## GitHub Issues

**Always use the `/create-issue` skill to create issues.** It owns the full labeling convention (type, priority, area, load, blocking relationships, triage state) and enforces North Star alignment. Never create issues manually or with ad-hoc labels.

## Dispatch Orchestration (GitHub Project Board #7)

The PLEXI project board has a **Status** field with these columns: `Idea → Backlog → Up Next → In Progress → In Review → Done`.

**"Up Next"** is the dispatch staging area — issues staged here are ready for parallel agent dispatch. At session start, query the board to see what's queued. Use the `/dispatch` skill — or directly: `bash .claude/skills/dispatch/scripts/open-lanes.sh <issue1> [issue2...]`.

**Status transitions:**
- Triage → **Up Next**: issue is ready, unblocked, and parallelizable (doesn't conflict with other Up Next issues)
- **Up Next** → **In Progress**: agent dispatched
- **In Progress** → **Done**: PR merged and issue closed

**Parallelizability rule:** Two issues are parallelizable if they don't touch the same source files. Check `area:*` labels as a proxy — same area = potential conflict.

## Milestones

Milestones define release collections. Assign a milestone when work is actively planned for a specific release sprint. Issues without a milestone are accepted but unslotted.

## Build Channels & Isolated Profiles

Each build channel is a **fully isolated instance** — its own binary, app bundle, config dir, log file, secrets index, and apps. The channel is detected at runtime from the binary name (e.g. `plexi-pr-783`).

| Channel | Binary | Profile dir | App bundle |
|---|---|---|---|
| Stable | `plexi` | `~/.plexi/` | `Plexi.app` |
| Beta | `plexi-beta` | `~/.plexi-beta/` | `Plexi Beta.app` |
| Alpha | `plexi-alpha` | `~/.plexi-alpha/` | `Plexi Alpha.app` |
| PR build | `plexi-pr-<N>` | `~/.plexi-pr-<N>/` | `Plexi PR<N>.app` |

**PR builds** are ephemeral isolated instances installed by `just pr-install <N>` from inside the feature worktree. They never capture the bare `plexi` symlink. Remove them after merge with `just pr-clean <N>`.

**Workspace** (`.plexi/workspace.toml`) is a separate per-project concept — the directory a user initializes with `plexi workspace init` inside their project root. It is not the same as the profile dir. Never run `workspace init` from `~` — it would create `~/.plexi/workspace.toml`, colliding with the stable profile dir.

**When writing test instructions for a PR build:** use `plexi-pr-<N>` (not `plexi`), and if the feature requires workspace context, direct the user to `cd` into a real project dir first.

## Branch Workflow

Three channels, each more stable than the last:

- `alpha` — active development. All work lands here first.
- `beta` — staging. Promoted from alpha when a batch of work is stable enough to share.
- `main` — production. Promoted from beta when ready to release.

Never commit directly to `beta` or `main`. All work flows through alpha. Feature branch naming: `feature/<issue-number>-short-description`. Never pass `--delete-branch` to `gh pr merge`.

**Dirty alpha during ship-issue:** The ship-issue skill auto-stashes uncommitted alpha changes at Phase 1 and auto-pops at every exit point. If changes disappear after a ship run, check `git stash list` — a stash named `ship-issue auto-stash before #<N>` may not have been popped (agent crash mid-cycle).

**Full ship cycle (label → worktree → PR → merge → install → cleanup) is defined in the `/ship-issue` skill.** Do not duplicate it here.

### alpha → beta → main (channel promotion)

When alpha has stabilised enough for broader testing:
```
git push origin alpha:beta
```
Run from the repo root (or anywhere with origin access). This fast-forwards beta to alpha's current HEAD. Then `just install` from `worktrees/beta/` to verify the beta build.

When beta is ready to ship as a release:
```
just promote main
```
This pushes beta→main, creates and pushes the version tag, and triggers the GitHub Actions release workflow.

Worktrees:
- `.` (repo root) — alpha branch
- `worktrees/beta` — beta branch
- `worktrees/main` — main branch
- `worktrees/feature/<branch>` — feature branches (created by `wtp add`)
- `worktrees/fix/<branch>` — fix branches (created by `wtp add`)

## Releases

Release flow:
1. `just bump [patch|minor|major]` — bumps version, generates CHANGELOG via git-cliff, commits `chore: release vX.Y.Z`
2. `just promote beta` — pushes alpha→beta, syncs beta worktree
3. Test on beta
4. `just promote main` — pushes beta→main, tags `vX.Y.Z`, triggers GitHub Actions release

## Build & Install

`just bump && just install` is the standard post-merge command — bumps the version and regenerates CHANGELOG via git-cliff, then builds and installs. Always run from the repo root.

`just install` alone is for re-installing without a version bump (e.g. after editing config or docs without a code change). Run from the repo root.

`just bump [minor|major]` without install is for explicit pre-promote version bumps when you need a minor or major release.

**Never claim a task complete based on an install from a feature worktree.** Uncommitted changes compile and install successfully, making the task appear done when nothing has been committed. The full done cycle is: commit → PR → squash-merge to alpha → `git pull` in the repo root → `just bump && just install` from the repo root.

## Logging

Build-specific log file:
- Alpha: `~/.plexi-alpha/plexi.log`
- Stable: `~/.plexi/plexi.log`

Rotates to `plexi.log.1` at 10 MB. Level set in `config.toml` (`error | warn | info | debug`). Third-party crates clamped to `warn`.

App logs forward into the host log tagged `app::<app_id>`. Python SDK: `ctx.info/warn/error/debug(...)` inside a frame; `emit.info(...)` outside. App stderr forwards as `warn`-level `app::<app_id>` entries.

**When debugging, check the log file first.**

**Every new feature must be instrumented.** Logging is not optional polish — it's the first diagnostic tool when something breaks:
- **Host (Rust):** `log::info!` at entry points for new `AppRequest`/`HostEffect`/`DrawCommand` handlers and any user-visible state change.
- **Apps/SDK:** `ctx.info()` or `emit.info()` at meaningful state transitions (init, key actions, errors).
- **CLI:** log the resolved command and any path it acted on.

No new capability, command, or user-visible behavior ships without at least one `info`-level trace that confirms it ran.

## Configuration Philosophy

Required fields have no defaults — fail fast with a clear error. Optional fields are clearly marked. Never paper over ambiguity with invisible magic. Prefer a verbose generated config with all options visible over a sparse one with hidden behavior.

## Python Tooling

Use `uv` for all Python projects. `pyproject.toml` with `requires-python = ">=3.11"`, `uv sync`, `uv run`. Bootstrap with `curl -LsSf https://astral.sh/uv/install.sh | sh` if absent. Never write manual venv creation loops.

## Error Handling

Try-catch on all I/O, network, external API calls, and anything that can reasonably fail. Every catch logs where + what failed with enough context to diagnose. Never swallow errors silently. If a failure can't be meaningfully recovered from, propagate or re-throw.

## Issue Visibility Before Work Begins

Before making any progress on a bug or issue, establish visibility of the problem. Never take a reporter's word alone — you need to see it yourself:

- **Preferred:** reproduce it in a `HostHarness` test that fails. This becomes the done condition.
- **Acceptable:** add a targeted `log::info!` or `log::warn!` that fires when the bad state occurs, then confirm it appears in `plexi.log` against the alpha build before writing any fix.

If you can't reproduce it or instrument it, stop and flag it. A fix written against an unconfirmed symptom is a guess. This check belongs at the triage step — before the issue is labeled `in progress` and before any worktree is created.

## Implementation Discipline (no half-refactors)

**Define done by the test, not the code.** Before writing any new module or refactoring an existing one, write the test that must pass when the work is complete. A PR is done when `cargo test --bin plexi` is green — not when the code looks right.

**Test-first for host logic.** Any new `AppRequest` or `HostEffect` gets a `HostHarness` test written before the implementation. The test failing is the starting state; making it pass is the work. This prevents stubs: a stub that makes the test pass is an implementation.

**No partial merges.** A PR that adds a new capability, module, or feature must be complete end-to-end. If it's too large to complete in one pass, scope it down — don't merge half of it. Split at natural seams where each piece is independently testable and independently useful.

## Panic Discipline (stubs must not crash the host)

`todo!()` and `unimplemented!()` are **banned outside `#[cfg(test)]`** — enforced by `#![deny(clippy::todo, clippy::unimplemented)]` in `src/main.rs`. They compile clean but panic at runtime, and a panic on the UI thread freezes the whole GUI.

**Factory rule:** any impl returned from a factory function (e.g. `audio_device()`, `video_decoder()`) must never panic in a trait method. Unimplemented methods return `Err(NotImplemented)` / `None` / noop — never `todo!()`. When you add a new prod stub, add a `prod_stub_tests` unit test that calls every trait method and asserts no panic.

## Lessons Carried Into v3

- **Coupled state:** When adding state that derives from or shadows existing state, grep every mutation site of the original and update each one.
- **Fallback chain audit:** When a value looks correct on the surface but behavior is stale, enumerate every fallback source in priority order (cookies, env vars, caches, defaults). Fix the chain, not the surface.
- **Model ID verification:** Never guess versioned model IDs. Use only confirmed-current family IDs. A 400/404 surfaces only at call time.
- **Platform behavior validation:** Before implementing any macOS-specific behavior (menu lifecycle, bundle naming, eframe/winit callback order), add a throwaway `log::info!()` to observe the actual runtime value on the first frame. Never assume which callback fires when or what a property returns — observe first, then code.
- **Command self-containment:** Any data a command handler needs must be in the command's own fields — never looked up from ambient state (like a queue or map) at dispatch time. By dispatch, that state may have been mutated or cleared by an earlier step in the same frame.
- **Test constructor sync:** When adding a field to any struct that has a `new_for_test()` constructor, update that constructor in the same commit. Before running `cargo test --bin plexi` on a fresh worktree, run it once on the base branch first to distinguish pre-existing failures from regressions.
- **Issue-referenced code validation:** When an issue names specific functions or code paths, grep for them in alpha before implementing — the function may have been removed or moved since the issue was filed.
- **git worktree operations:** Always use absolute paths with `git -C /absolute/worktree/path <command>` — relative paths (`git -C worktrees/<branch>`) fail when CWD is not the repo root. Applies to all operations: `add`, `rebase`, `push`, etc.
- **HostHarness initial state:** `add_test_pane()` inserts a `ProcessApp` pane — not a Terminal. Terminal-count assertions in tests must not assume the initial pane is a Terminal; offset accordingly.
- **Shell suffix construction:** when appending a stay-alive or exec suffix to a user command string, use the absolute shell path from `settings.shell` (already resolved) rather than `$SHELL`, and `trim_end_matches([';', ' '])` the user command before appending to prevent `;;` syntax errors.
- **cfg(unix) propagation on removal:** When removing a `#[cfg(unix)]` block or executable-bit check, grep for `set_mode`, `PermissionsExt`, and `0o755` across all test functions in the same file before staging. The helper function is never the only site.

## Host UI Systems — Reuse Before Rolling Your Own

Before writing any keyboard shortcut display, badge, chip, or inline label widget, check `src/widgets.rs` and `src/style.rs`. These modules contain the canonical, already-tested implementations. Re-rolling them inline produces visual inconsistency and duplicated sizing logic.

**`src/style.rs`** — design tokens: spacing scale (`SPACE_SM/MD/XL`), typography scale (`TEXT_HINT/CAPTION/BODY/TITLE_XL`), corner radii (`RADIUS_MD/LG`), modal widths, button heights, overlay chrome. Use these constants everywhere — never hard-code magic numbers.

**`src/widgets.rs`** — reusable egui widgets:
- `key_chip(ui, label, colors)` — renders a single keyboard key as a styled rounded-rect chip (`bg_active` fill, `border` stroke, `TEXT_HINT`-size monospace text).
- `key_combo(ui, keys, colors)` — renders a sequence of `key_chip`s with `INTRA_COMBO_GAP` between them (e.g. `["⌘", "N"]` → `[⌘][N]`).
- `key_combo_list(ui, combos, trailing, colors)` — renders multiple combos inline with `INTER_COMBO_GAP` between them and an optional dim description label at the end. This is the standard pattern for keyboard shortcut hint rows.

**Use `key_combo_list` for any shortcut hint row.** Do not render key shortcuts as plain `Label` text — it produces a visually inconsistent result that requires a separate pass to fix.

**Overlay layout primitives** — four shared widgets every overlay should use instead of inlining:
- `section_header(ui, label, is_active, colors)` — group/context label at `TEXT_CAPTION` weight; `is_active` switches color from `text_dim` to `accent`.
- `pane_type_badge(ui, kind, colors)` — renders `"T"` for Terminal, `"A"` for App (first letter of kind) as a `key_chip`. Saves horizontal space vs. full word.
- `status_chip(ui, status, colors)` — centralized status color mapping: `"busy"`/`"running"` → `accent`; `"crashed"`/`"hung"`/`"error"`/`"exited"` → `danger`; everything else → `text_dim`.
- `description_label(ui, text, colors)` — single-line `TEXT_HINT` label with `truncate()`. **Always wrap in `ui.scope()` and set `ui.set_max_width(n)` inside the scope** — setting it on a shared `Ui` corrupts layout of other widgets in the same row.

## Channel-Agnostic CLI Rule

Every CLI command and feature must work identically on alpha, beta, stable, and PR builds. This is non-negotiable — the release channel is an implementation detail, not something callers should need to know.

**How it works:**
- `PLEXI_SOCKET` (set inside a Plexi pane) routes **host commands** (pane, notify, context, open, etc.) to the correct running instance — but it does NOT re-route the binary itself. Typing `plexi` inside a PR817 pane still runs the stable/alpha binary; to target a specific channel you must use the full binary name (`plexi-alpha`, `plexi-pr-817`, etc.).
- When `PLEXI_SOCKET` is not set, commands fall back to channel-specific mechanisms (spawn-queue, config_dir) derived from the running binary name.
- `/usr/local/bin/plexi` (the bare `plexi` command) is kept as a symlink to the most recently installed non-PR channel binary by `scripts/install.sh`. PR builds never capture the bare name.

**Enforcement:** Never hardcode a profile directory path (e.g. `~/.plexi-alpha/`) in CLI code — always use `config_dir()`. Never route around `PLEXI_SOCKET` when it is set. Any new CLI command that communicates with a running instance must follow the socket-first pattern in `open_cli()`.

**Testing completions on PR builds:** `just pr-install` intentionally skips completion installation — all channels share a single completion file path (e.g. `$(brew --prefix)/share/zsh/site-functions/_plexi`) and a PR build overwriting it would corrupt the active channel's completions. To test a completion change on a PR build, manually run `plexi-pr-<N> completions zsh > <completions-path>` after install and restore the previous file afterward. Completion changes that don't require interactive testing can be merged to alpha and verified there.

## CLI Namespace Design

Before adding any new CLI command, verify it belongs in the right namespace — place it where the noun already lives, not at the top level. When in doubt, ask before implementing.

## General Rules

- Before SSH/networking setup, ask if machines are on the same LAN or remote. Before any multi-step infra task, clarify topology first.
- When the user reports a bug, fix what they asked for first. Don't pivot to QA, refactoring, or tangential improvements until the primary request is resolved.
- When the user provides multiple distinct ideas, file them separately. Don't combine unrelated concepts.
- Never use `#[allow(dead_code)]` or `#[allow(unused)]`. Always do the work: delete unused code, wire it up, or move it to a feature-flagged module. If fixing a warning takes a long time, that's the job — do not paper over it with an allow attribute.
- Always run `cargo build` after work to make sure it passes.
- **Failed PR reset:** If a PR fails its first test pass and the diff is under ~1000 lines: close the PR without merging, revert the worktree to clean, comment on the original issue (what was tried, what failed, why), re-label the issue `ready`, and start a fresh agent with only the updated issue as context. Don't patch a broken attempt — start clean.

## Issue Prior Attempts

When an issue tracks a feature or bug that has been attempted before, document the failure in the issue **body** under a `## Prior Attempts` section — not in comments. Comments are invisible to `gh issue view` without an explicit `--comments` flag and will be missed by agents reading the issue before implementing.

Format:
```markdown
## Prior Attempts

**Attempt N:** What was tried.
**Why it failed:** Root cause or observable symptom.
**What to try next:** Specific next investigation step.
```
