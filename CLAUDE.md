## Source of Truth for Project State

`AGENTS.md` is a symlink to this file. Keep project-agent instructions here so both entry points stay identical.

**CLAUDE.md does not track in-progress work or completion status.** It goes stale immediately and will mislead future sessions.

- **What shipped and why** → `git log --oneline -20` and `GOTCHAS.md` for non-obvious discoveries
- **What's currently in flight** → `git status`, open PRs, and issue pipeline labels
- **Product direction** → `NORTH_STAR.md`
- **App-framework + marketplace plan** → `docs/prm/app-framework-marketplace.md`
- **Host UI kit plan** → `docs/prm/host-ui-kit.md`
- **File Explorer overhaul PRD** → `docs/prm/file-explorer-overhaul.md`
- **Implementation backlog** → GitHub issues, used as work tickets only

Before reporting anything as "done" or "missing", verify against `git log`.

## Planning Source

Plexi no longer uses GitHub Project board #7, `NEXT.md`, or generated dispatch snapshots to decide what happens next. Do not query, update, or trust the Project board for planning.

For app-framework, packaging, marketplace, MCPUI, WASM/WASI, and Bevy work, read `docs/prm/app-framework-marketplace.md` first. That PRM is the local source of truth and resolves conflicts with older roadmap fragments.

For host-level UI chrome work (modals, overlays, palettes, rows, text fields, buttons, hint bars, and overlay focus handling), read `docs/prm/host-ui-kit.md` first. That PRM is the local source of truth for the host UI kit sequence and points to the ordered implementation issues.

For File Explorer overhaul work, read `docs/prm/file-explorer-overhaul.md` after `docs/prm/host-ui-kit.md`. The File Explorer PRD is intentionally queued behind the Host UI Kit rework; do not rebuild File Explorer-specific row, table, modal, button, text field, or hint primitives that belong in the shared host UI kit.

When a PRD has a `Progress` table, update the relevant row in the same change that finishes or supersedes an issue. Do not make future agents infer PRD completion only from closed GitHub issues.

GitHub issues are implementation tickets. To choose the next dispatch, match open issues against the first unfinished milestone in the PRM, skip anything blocked or in progress, then choose parallel lanes whose `area:*` labels do not overlap. If the PRM calls for work that has no issue yet, create or triage the issue before dispatching.

## North Star

Before making architectural decisions, read [`NORTH_STAR.md`](NORTH_STAR.md) for product direction and [`GLOSSARY.md`](GLOSSARY.md) for shared vocabulary (pane, context, PGAP, capability, secret, etc.).

## Branches

**`alpha` is the starting branch for all changes.** Every feature branch, worktree, and PR originates from alpha. Never branch from `main` or `beta`.

Feature branch naming: `feature/<issue-number>-short-description`. Never push directly to `main` or `beta`.

## GitHub Issues

**Always use the `/create-issue` skill to create issues.** It owns the full labeling convention (type, priority, area, load, blocking relationships, triage state) and enforces North Star / PRM alignment. Never create issues manually or with ad-hoc labels.

## Dispatch Orchestration

Dispatch works from explicit issue numbers:

```
/dispatch <issue1> [issue2...]
```

There is no separate "Up Next" staging area. Before dispatching, read the marketplace PRM, audit the relevant open issues, and pick issues that match the next unfinished milestone.

**Pipeline labels are the live work state:** `pipeline:implement`, `pipeline:open-pr`, `pipeline:validate`, `pipeline:merge`, and `in progress`. The Project board is retired.

**Parallelizability rule:** Two issues are parallelizable if they do not touch the same source files. Check `area:*` labels as a proxy — same area = potential conflict.

## Milestones

GitHub milestones are optional release buckets, not the planning source. Do not introduce milestone bookkeeping unless the user explicitly asks for a release collection. The PRM's prose milestones are enough for marketplace sequencing.

## Build Channels & Isolated Profiles

Each build channel is a **fully isolated instance** — its own binary, app bundle, config dir, log file, secrets index, and apps. The channel is detected at runtime from the binary name (e.g. `plexi-pr-783`).

| Channel | Binary | Profile dir | App bundle |
|---|---|---|---|
| Main | `plexi` | `~/.plexi/` | `Plexi.app` |
| Beta | `plexi-beta` | `~/.plexi-beta/` | `Plexi Beta.app` |
| Alpha | `plexi-alpha` | `~/.plexi-alpha/` | `Plexi Alpha.app` |
| PR build | `plexi-pr-<N>` | `~/.plexi-pr-<N>/` | `Plexi PR<N>.app` |

**PR builds** are ephemeral isolated instances installed by `just pr-install <N>` from inside the feature worktree. They never capture the bare `plexi` symlink. Remove them after merge with `just channel-clean pr-<N>`.

**Alpha config stays default.** `~/.plexi-alpha/config.toml` is reset to the default template on every `just install`. Never customize it — use beta or main for personal config. PR builds seed from the alpha config, so a customized alpha would pollute every PR channel.

**Beta config is the staging ground.** `~/.plexi-beta/config.toml` is your personal staging config — customize it freely for migration testing and advanced feature exploration. It is NOT reset on install.

**Workspace** (`.plexi/workspace.toml`) is a separate per-project concept — the directory a user initializes with `plexi workspace init` inside their project root. It is not the same as the profile dir. Never run `workspace init` from `~` — it would create `~/.plexi/workspace.toml`, colliding with the main channel profile dir.

**When writing test instructions for a PR build:** use `plexi-pr-<N>` (not `plexi`), and if the feature requires workspace context, direct the user to `cd` into a real project dir first.

## Branch Workflow

Three channels, each more stable than the last:

- `alpha` — active development. All work lands here first.
- `beta` — staging. Promoted from alpha when a batch of work is stable enough to share.
- `main` — production. Promoted from beta when ready to release.

Never commit directly to `beta` or `main`. All work flows through alpha. Feature branch naming: `feature/<issue-number>-short-description`. Never pass `--delete-branch` to `gh pr merge`.

**Full ship cycle:** `/dispatch N` → opens one pane per issue → each pane runs implement-issue → open-pr → validate-pr (notifies you, waits) → merge-pr inline. No PM needed for the happy path. Labels track state for recovery.

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

**Bump and install after Rust code commits to alpha.** Only needed when Rust source changes — skip for commits that only touch docs, skills, scripts, or config. Every Rust commit to alpha must end with `just bump && just install`. If unrelated uncommitted changes block `just bump`, stash them first and pop after.

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

- **Platform behavior validation:** Before implementing any macOS-specific behavior (menu lifecycle, bundle naming, eframe/winit callback order), add a throwaway `log::info!()` to observe the actual runtime value on the first frame. Never assume which callback fires when or what a property returns — observe first, then code.
- **Command self-containment:** Any data a command handler needs must be in the command's own fields — never looked up from ambient state (like a queue or map) at dispatch time. By dispatch, that state may have been mutated or cleared by an earlier step in the same frame.
- **Test constructor sync:** When adding a field to any struct that has a `new_for_test()` constructor, update that constructor in the same commit. Before running `cargo test --bin plexi` on a fresh worktree, run it once on the base branch first to distinguish pre-existing failures from regressions.
- **Issue-referenced code validation:** When an issue names specific functions or code paths, grep for them in alpha before implementing — the function may have been removed or moved since the issue was filed.
- **HostHarness initial state:** `add_test_pane()` inserts a `ProcessApp` pane — not a Terminal. Terminal-count assertions in tests must not assume the initial pane is a Terminal; offset accordingly.
- **Shell suffix construction:** when appending a stay-alive or exec suffix to a user command string, use the absolute shell path from `settings.shell` (already resolved) rather than `$SHELL`, and `trim_end_matches([';', ' '])` the user command before appending to prevent `;;` syntax errors.
- **cfg(unix) propagation on removal:** When removing a `#[cfg(unix)]` block or executable-bit check, grep for `set_mode`, `PermissionsExt`, and `0o755` across all test functions in the same file before staging. The helper function is never the only site.

## Host UI Systems — Reuse Before Rolling Your Own

For new host-level UI chrome work, read [`docs/prm/host-ui-kit.md`](docs/prm/host-ui-kit.md) before editing overlays or shared widgets. It defines the host UI kit plan for modals, palettes, pickers, rows, text fields, buttons, hint bars, and overlay focus handling.

Before writing any keyboard shortcut display, badge, chip, or inline label widget, check `src/widgets.rs` and `src/style.rs`. These modules contain the canonical, already-tested implementations. Re-rolling them inline produces visual inconsistency and duplicated sizing logic.

**`src/style.rs`** — design tokens: spacing scale (`SPACE_SM/MD/XL`), typography scale (`TEXT_HINT/CAPTION/BODY/TITLE_XL`), corner radii (`RADIUS_MD/LG`), modal widths, button heights, overlay chrome. Use these constants everywhere — never hard-code magic numbers.

**`src/widgets.rs`** — reusable egui widgets:
- `key_chip(ui, label, colors)` — renders a single keyboard key as a styled rounded-rect chip (`bg_active` fill, `border` stroke, `TEXT_HINT`-size monospace text).
- `key_combo(ui, keys, colors)` — renders a sequence of `key_chip`s with `INTRA_COMBO_GAP` between them (e.g. `["⌘", "N"]` → `[⌘][N]`).
- `key_combo_list(ui, combos, trailing, colors)` — renders multiple combos inline with `INTER_COMBO_GAP` between them and an optional dim description label at the end. This is the standard pattern for keyboard shortcut hint rows.

**Use `key_combo_list` for any shortcut hint row.** Do not render key shortcuts as plain `Label` text — it produces a visually inconsistent result that requires a separate pass to fix.

**Overlay layout primitives** — four shared widgets every overlay should use instead of inlining:
- `section_header(ui, label, is_active, colors)` — group/context label at `TEXT_CAPTION` weight; `is_active` switches color from `text_dim` to `accent`.
- `pane_type_badge(ui, kind, colors)` — renders `"term"` for Terminal, `"app"` for App as a `key_chip`. Compact vs. full word.
- `status_chip(ui, status, colors)` — centralized status color mapping: `"busy"`/`"running"` → `accent`; `"crashed"`/`"hung"`/`"error"`/`"exited"` → `danger`; everything else → `text_dim`.
- `description_label(ui, text, colors)` — single-line `TEXT_HINT` label with `truncate()`. **Always wrap in `ui.scope()` and set `ui.set_max_width(n)` inside the scope** — setting it on a shared `Ui` corrupts layout of other widgets in the same row.

## Channel-Agnostic CLI Rule

Every CLI command and feature must work identically on alpha, beta, main, and PR builds. This is non-negotiable — the release channel is an implementation detail, not something callers should need to know.

**How it works:**
- `PLEXI_SOCKET` (set inside a Plexi pane) routes **host commands** (pane, notify, context, open, etc.) to the correct running instance — but it does NOT re-route the binary itself. Typing `plexi` inside a PR817 pane still runs the main/alpha binary; to target a specific channel you must use the full binary name (`plexi-alpha`, `plexi-pr-817`, etc.).
- When `PLEXI_SOCKET` is not set, commands fall back to channel-specific mechanisms (spawn-queue, config_dir) derived from the running binary name.
- `/usr/local/bin/plexi` (the bare `plexi` command) is reserved for the `main` channel binary. If `main` is not installed, the bare symlink should not exist. PR builds never capture the bare name.

**Enforcement:** Never hardcode a profile directory path (e.g. `~/.plexi-alpha/`) in CLI code — always use `config_dir()`. Never route around `PLEXI_SOCKET` when it is set. Any new CLI command that communicates with a running instance must follow the socket-first pattern in `open_cli()`.

**Channel-aware workspace paths:** Never hardcode `.plexi/` as a workspace directory name when joining from a workspace root. Always use `crate::config::workspace_channel_dir()` or a helper built on it, such as `workspace_config_path()`. This returns `.plexi`, `.plexi-alpha`, `.plexi-beta`, `.plexi-pr-N`, etc. based on the running binary/profile. Literal `.plexi` is only acceptable for docs, tests that explicitly cover the main channel, or profile-dir class checks that intentionally match `.plexi*`.

**Testing completions on PR builds:** `just pr-install` intentionally skips completion installation — all channels share a single completion file path (e.g. `$(brew --prefix)/share/zsh/site-functions/_plexi`) and a PR build overwriting it would corrupt the active channel's completions. To test a completion change on a PR build, manually run `plexi-pr-<N> completions zsh > <completions-path>` after install and restore the previous file afterward. Completion changes that don't require interactive testing can be merged to alpha and verified there.

## CLI Namespace Design

Before adding any new CLI command, verify it belongs in the right namespace — place it where the noun already lives, not at the top level. When in doubt, ask before implementing.

## CLI Pane Naming

Always name panes after spawning them. Every `plexi terminal`, `plexi app open`, split, or new window should be followed by `plexi pane name <id> "descriptive name"`. Named panes make the UI scannable, help the project-manager read state from `plexi pane list`, and make dispatch lanes identifiable at a glance.

## CLI Tips

Contextual tips (e.g. keyboard shortcut hints) use `print_tip()` from `src/cli/mod.rs`. Never raw `eprintln!` a tip. `print_tip` checks `config.cli.tips` (default `true`, user can set `false` in `config.toml` under `[cli]`), respects `NO_COLOR`, and logs via `log::info!`.

## App Priority

**Core 9 only.** Do not fix, update, or improve `apps/dev/` apps unless the change is a POC demonstrating a new SDK/host capability (e.g. counter-tree for ComponentEvent). All dev effort goes to the Core 9 and the SDK/host. Dev apps are throwaway proofs-of-concept, not maintained software.

## App & SDK Design Philosophy

- **Obvious over clever.** Fight for the solution agents would naturally assume. If a design requires explanation, that's a signal to rethink it. "Simple" and "obvious" aren't always aligned — sometimes the obvious solution is more complex, and that's correct.
- **Simulate affordances, never lie about contracts.** Give apps familiar interfaces (filesystem-like, subprocess-like), but never obscure isolation, durability, security, or persistence boundaries. These must be explicit, never implied.
- **Build primitives, not features.** Resist building what developers' agents can trivially implement atop the platform. Feature creep in the core is maintenance debt that compounds. When scope is unclear, omit.
- **Design for agents, not humans browsing docs.** The SDK must be immediately legible to a coding agent. If it requires a README to be usable, the API is wrong.

## Session Velocity

High-throughput sessions share a pattern: strong initial analysis → user trust → fast feedback loops → features ship. These rules protect that flow.

**Orient from the document, not the issues.** When the user says "complete the roadmap" or "finish layer 1," read the roadmap/spec file. It IS the plan. Do not fetch individual GitHub issues to "understand" work that's already described in the document the user referenced. Issue bodies are 50-80 lines each; reading 6 of them serially burns 300-480 lines of context before any code is read.

**Never serialize issue reads.** If you genuinely need issue details, use `gh issue list --search "..." --json number,title,labels,state` with filters to get a summary in one call. Only `gh issue view` for the single issue you're about to implement. Reading issues "to orient" is almost always wrong.

**Context is a budget.** Every tool call that dumps text has a cost. Before fetching anything, ask: "do I already have enough to start?" If the user described the work, you probably do.

**Pipeline phases flow inline.** implement → open-pr → validate → merge. Never stop between phases to ask "should I proceed?" The user already said to ship. Invoke the next phase at the end of the current one.

**Match user energy.** When the user says "do it," gives screenshots, or provides directional feedback, they're in build mode. Stop asking, start building. Present specs as conversation (a table, a code block), get one "do it," then execute.

**Subagent for implementation, orchestrator for review.** Sonnet subagents handle multi-file coding (>3 files or multiple subsystems). The orchestrator always reviews diffs before committing, catches mistakes, owns the final commit. Never let a subagent commit directly.

**Ideas become issues, not tangents.** When the user pitches something adjacent mid-stream, file it as an issue and move on in the same breath. Don't explore it, don't ask if they want to explore it.

**Direct-to-alpha when user is watching.** The full pipeline (worktree, PR, validate, merge) is for dispatched/autonomous work. When the user is actively watching every build and giving screenshot feedback, direct commits to alpha are appropriate. The ceremony exists to catch mistakes when nobody's looking; when the user IS looking, ship fast.

**Screenshot-feedback loops.** The best sessions iterate in <5 min cycles: edit → `cargo build` → `just install` → user screenshots → targeted fix → repeat. Keep changes small enough that each cycle is one visual diff.

**Own the build.** If your change breaks something, fix it. If a previous commit broke something, fix that too. Never surface a build failure as a question to the user.

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
