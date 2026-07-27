## Purpose

`CLAUDE.md` is a symlink to this file. Cross-cutting rules for all agents. Domain-specific contracts live in each directory's own `AGENTS.md`.

Before editing any file, read the `AGENTS.md` in its directory if one exists. Child rules add to this file; they never override it.

## Source of Truth

- **What shipped** → `git log --oneline -20`
- **Product direction** → `NORTH_STAR.md`
- **Feature specs** → `docs/*.md` (active PRMs; see `docs/AGENTS.md` for lifecycle rules)
- **Sprint graph** → `.stint/` (`stint next`, `stint status`)
- **Implementation tickets** → `.stint/` tasks (GitHub issues are optional; stint is authoritative)

Do not track in-progress work or completion status in this file.

## Website

The product website is **`plexiapp.com`**. Never write `plexiapp.dev` or `plexi.app`.

## Stint Time Tracking

When work begins: `stint claim <task-id>`. Do not run or document `stint start`; the installed CLI does not have that command, and `claim` owns status plus `started_at`. When done: `stint done <task-id>`. Use UTC timestamps. If abandoned, leave `started_at` in place, do not set `completed_at`.

## Child DOX Index

| Directory | Owns |
|---|---|
| `src/cli/` | CLI rules, channel-agnostic enforcement, namespace design, pane naming |
| `src/ui/` | Host UI kit primitives, design tokens, overlay layout widgets |
| `src/config/` | Config loading/validation; reference is `docs/CONFIG.md` |
| `src/testing/` | Test infrastructure, TESTING.md reference, scene format |
| `src/render/` | CLI renderer app contract |
| `src/workspace/` | Workspace state, environment secrets resolver |
| `sdk/python/` | SDK traps; AUTHORING.md canonical app guide; SDK_V3.md design spec |
| `apps/` | App rules, maintained-set policy (`packs/core.toml`), design philosophy |
| `scripts/` | Build channels, branch workflow, releases, install, RELEASE_CHANNELS.md |
| `registry/` | CLI descriptor guide, embedded descriptor registry |
| `docs/` | Active PRMs; lifecycle rules in `docs/AGENTS.md` |

## Branches

`alpha` is the starting branch. Every feature branch, worktree, and PR originates from alpha. Never branch from `main` or `beta`. Feature branch naming: `feature/<issue-number>-short-description`.

## Git Rules

Never add `Co-Authored-By: Claude ...` trailers. Never push directly to `main` or `beta`. Never pass `--delete-branch` to `gh pr merge`.

## Tasks and Issues

Always use the `/create-stint` skill to create tasks. It owns the full flow: duplicate check, sizing, sprint placement, blocking, and optional GitHub issue creation. Never create stint tasks or GitHub issues manually.

`.stint/` is git-ignored by design. New or updated stint tasks may not appear in `git status`; that is OK. Validate task state with `stint check`, `stint list`, `stint show <id>`, and `stint status`.

## Planning

Read the relevant PRM first. Use `stint next` for the next claimable task. Stint tasks are the primary implementation tickets; GitHub issues are optional. Pipeline labels (`pipeline:implement`, `pipeline:open-pr`, `pipeline:validate`, `pipeline:merge`) are the live work state.

## Logging

Log file: `~/.plexi-<channel>/plexi.log`. Rotation is date-based, not size-based: on startup, a log last modified on a prior day is renamed to `plexi-<YYYY-MM-DD>.log` and dated archives older than `[log] retention_days` (default 30) are pruned — see `rotate_and_prune` in `src/platform/logging.rs`. Level set in `config.toml`.

Every new feature must be instrumented. No new capability, command, or user-visible behavior ships without at least one `info`-level trace.

## Testing

**Mandatory self-validation contract: [`src/testing/TESTING.md`](src/testing/TESTING.md).** Every coding agent follows it before push. Observable state → TOML scene. Return value or invariant → Rust `#[test]`. `cargo test --bin plexi` must be green before any push.

**`just pr-install <N>` is cwd-independent.** The recipe resolves the PR's head via `gh`, selects a worktree that provably contains it (the PR's clean feature worktree when present, else a detached canonical build tree), and runs pre-install tests and the build from that tree — never the caller's cwd. Provenance (PR, head sha, worktree) is echoed and appended to the PR profile's `install.log`.

Test-first for host logic. Define done by the test, not the code. No partial merges.

**Host UI changes: identify the screenshot-test surface before implementing, not after.** TESTING.md's visual-review step (render realistic seeded content, Read the PNG, delete it) is mandatory, not an optional postscript — a passing alignment/geometry assertion is not proof the pixels look right. Plan which harness test will prove the fix before writing the fix.

## Panic Discipline

`todo!()` and `unimplemented!()` are banned outside `#[cfg(test)]` (enforced by `#![deny(clippy::todo, clippy::unimplemented)]`). Factory-returned impls must never panic in trait methods.

## Error Handling

Try-catch all I/O, network, external API calls. Log where + what failed. Never swallow errors. Propagate unrecoverable failures.

## Issue Visibility Before Work

Reproduce the bug before fixing it. Preferred: a failing `HostHarness` test. Acceptable: a targeted `log::info!` confirmed in `plexi.log`. If you can't reproduce or instrument it, stop and flag it.

## Issue Prior Attempts

Document failures in the issue **body** under `## Prior Attempts`, not in comments. Comments are invisible to `gh issue view` without `--comments`.

## Python Tooling

`uv` for all Python. `pyproject.toml` with `requires-python = ">=3.11"`, `uv sync`, `uv run`.

## Session Velocity

- **Orient from the document, not the issues.** The PRM IS the plan.
- **Never serialize issue reads.** Use `gh issue list --search` with filters.
- **Context is a budget.** Before fetching, ask: do I already have enough?
- **Pipeline phases flow inline.** implement → open-pr → validate → merge. No stopping to ask.
- **Match user energy.** When the user says "do it," start building.
- **Sequential sub-agents only.** Never parallel in one worktree.
- **Ideas become stint tasks, not tangents.**
- **Direct-to-alpha when user is watching.**
- **Own the build.** If your change breaks something, fix it.

## Documentation Rule

Every fact lives in exactly one place. Other files reference it; they never restate it. If you find yourself writing something that exists elsewhere, replace it with a pointer. Inline command help (justfile recipe comments) is exempt — it serves `just --list`, not agent orientation.

**No volatile numbers in docs.** Never cite a line number, a file offset, or a count of things (`foo.rs:40`, "line 292", "the 14 exemplar apps", "Core 9") in any `AGENTS.md`, PRM, or contract doc. They drift silently the moment code changes and send agents to the wrong place. Reference code by symbol name (`builtin_factory`, `decode_badge_color`) and let grep find it; describe sets by their defining source (`packs/core.toml`, the exemplar dirs under `apps/`), never by a frozen tally. Version identifiers, schema numbers, and dates are not counts and are fine.

**One progress tracker per unit of work.** Work lives in a stint task — never in a GitHub issue, never tracked inside a spec doc. A PRM describes destination state; it never tracks what is done. No checklists, no strikethrough, no status tables inside PRMs. The stint task is the single delete trigger for its PRM.

## Traps

Non-obvious discoveries with no single owning directory. When you discover a trap, add it to the `## Traps` section of the relevant child `AGENTS.md` file. If it spans multiple subsystems, add it here.

- **`proc_listchildpids(NULL, 0)` returns `EFAULT` on macOS 23.x (Sonoma).** Documented to return bytes needed; on Sonoma it returns -1. Use `pgrep -P <pid>` instead — exits 0 when children exist, 1 when idle, reliable across macOS versions.
- **`git status --porcelain` can show false-dirty files.** Index timestamps may be stale while `git diff HEAD` is empty. Run `git update-index --refresh` before treating the branch as dirty.
- **Observe macOS platform behavior before coding it.** Before implementing any macOS-specific behavior (menu lifecycle, bundle naming, eframe/winit callback order), add a throwaway `log::info!()` to observe the actual runtime value on the first frame. Never assume which callback fires when.
- **Command handler data must be self-contained.** Any data a command handler needs must be in the command's own fields, never looked up from ambient state at dispatch time. By dispatch, that state may have been mutated or cleared by an earlier step in the same frame.
- **`#[cfg(unix)]` removal — grep all sites.** When removing a `#[cfg(unix)]` block or executable-bit check, grep for `set_mode`, `PermissionsExt`, and `0o755` across all test functions in the same file before staging. The helper function is never the only site.
- **Issue-referenced code may no longer exist.** When an issue names specific functions or code paths, grep for them in alpha before implementing. The function may have been removed or moved since the issue was filed.
- **`create_page_at` takes an explicit `context_id`.** Never temporarily switch `active_window` or `router.active` to steer `create_page_at` into a context — pass `context_id: u64` directly. To get the caller-pane's context: `find_pane_in_any_window(from_pane_id)` → `self.windows[win_idx].context_id`.
- **Don't switch global state to thread data through a function.** When a helper reads from `router.active()` or `active_window`, the fix is to add an explicit parameter — not to temporarily mutate global focus state before calling it. Global-state mutation as a calling convention is always a hack.
- **`plexi` CLI is almost always running inside a Plexi pane.** Never assume an outside-terminal scenario unless the bug explicitly involves the spawn-queue or `PLEXI_SOCKET` being unset. User-reported issues are about in-pane behavior.
- **`PLEXI_CHANNEL` leaks into app tooling.** A pane launched under beta runs `plexi app check` / `plexi app render` against the beta profile SDK even when the app path is under `.plexi-alpha/`. For alpha validation, make the channel explicit with `env PLEXI_CHANNEL=alpha plexi ...` or use `plexi-alpha`. For PR builds, use `plexi-pr-<N>` directly or `env PLEXI_CHANNEL=pr-<N> plexi ...` so the shim selects the `~/.plexi-pr-<N>/` profile. Do not infer the runtime SDK/profile from the app path.
- **Parallel wave agents can independently fix the same latent bug.** Two same-wave worktrees both hit and fixed the same pre-existing defect in a shared file (`wasm_render.rs`'s PNG font-bootstrap ordering) because each agent's task happened to exercise it — a real rebase conflict, not a merge mistake. Resolve by reading both fixes and keeping the more complete one, not by mechanically taking "ours" or "theirs."
- **Host-spawned login/interactive shell probes must be setsid-isolated.** Give them their own session (no controlling TTY), stdin=/dev/null, and capture stdio — otherwise the user's profile chain can steal keystrokes or bleed sudo/tput noise onto the session Plexi was launched from. Shared helper: `run_login_shell_probe` in `src/host/shell.rs`.
- **egui advances delayed repaint requests by its predicted frame time.** A scheduler that passes its exact frame interval to `request_repaint_after` can turn a 60 Hz deadline into an immediate repaint loop. Add `InputState::predicted_dt` to the remaining wall-clock delay, and stop scheduling a deadline while the producer is still busy with an overdue frame. A literal zero-delay request also schedules an extra settling paint; use a nonzero delay when a background completion needs exactly one immediate host pass.
- **macOS App Nap silently defers an idle host's cross-thread wakeups.** A non-frontmost idle eframe host gets napped: `request_repaint()` posted from a background thread (notify-socket IPC) is not delivered until an unrelated event un-naps the app, so CLI requests time out and then drain in one late burst. Neither winit nor eframe exempts the process. The host holds a process-lifetime `NSProcessInfo` activity (`platform::app_nap::disable_app_nap`, `UserInitiatedAllowingIdleSystemSleep`) so it stays wakeable; any new long-lived headless/background process serving IPC on macOS needs the same exemption.
- **eframe skips `App::ui` entirely while the window is hidden.** `ViewportInfo::visible()` is false when the window is minimized *or fully occluded by other windows*; eframe then runs logic-only passes — `App::logic` fires, `App::ui` never does, and the pass still advances `cumulative_pass_nr`, so wakes "work" while nothing visible in `ui` executes. Every drain that services external clients (pane IPC, spawn queue, PTY events, shutdown, event-bus subscriptions, screenshot capture) must live in `App::logic`, never in `ui`. Regression guard: `pane_ipc_serviced_while_window_hidden`; harness pump: `HostHarness::hidden_frame`. This also applies to *issuing* work, not just draining it: `plexi host screenshot` sent its `ViewportCommand::Screenshot` from `ui`, so an occluded host never asked for a capture — the PNG arrived only once the window was uncovered, after the CLI had timed out (`service_pending_screenshots`, stints 0495/0504).

## Architecture

**HostModel** is a pure state machine with zero egui dependency. Commands in, effects out. All business logic (pane lifecycle, permissions, events) lives here. The renderer (egui in prod, headless in CI) reads state and paints — it never owns logic.

## Inter-Pane Communication

There is one comms model with three disjoint planes, each with a single sanctioned transport:

- **Structured app-to-app data → the event bus.** `DeclareEventStreams` / `EmitEvent` / `SubscribeAppEvents` (subscription-scoped, cross-window, `resource_id`-filterable). A directed pipe (`PipeOpenDirected`) is a thin alias over this bus for the one case that needs an exclusive `(sender, target)` duplex JSON channel. The legacy non-directed peer-broadcast JSON fan-out was removed in 0327 — do not reintroduce it.
- **Bulk binary data → typed pipes** (`src/host/typed_pipes.rs`): Unix-socket + ring buffer for audio/video/MIDI frames.
- **Human-trust PTY injection → `plexi pane send` / `RunInLinkedTerminal`.** This is a *human* affordance for driving a terminal a person is watching. **App-to-app data must never route through PTY injection** — it bypasses capability scoping, has no schema, and is unobservable to the event log. Use the event bus.

Later consolidation candidates (left untouched by 0327, not yet unified): notifications (`NotifyAction`), pane slots, `PathChanged` pane-group sync, poll-based "Phase C" delivery, and undo/rollback checkpoints on the event record.

## Capturing Host State

`plexi host screenshot [--pane <id>] [--output <path>]` captures the running host window as a PNG through the real render pipeline (`AppRequest::Screenshot` → `egui::ViewportCommand::Screenshot` → `src/app/screenshot.rs`) — the actual pixels on screen: chrome, terminals, apps, overlays. `--pane` crops to that pane's current rect. Works over the channel socket like every pane command (`PLEXI_SOCKET`/`PLEXI_CHANNEL` rules apply). **Never use macOS `screencapture` or any OS-level capture to inspect a Plexi host** — this command is the sanctioned path, works headless, and needs no screen-recording permission. For pane *semantics* (text, node bounds) use `plexi pane state <id>`; for a not-running-host render of an app, use `plexi app render . --png`.

## General Rules

- When the user reports a bug, fix what they asked for first.
- Never use `#[allow(dead_code)]` or `#[allow(unused)]`. Delete or wire up.
- Always run `cargo build` after work.
- **Failed PR reset:** close the PR, revert worktree, comment on the issue, re-label `ready`, start fresh.
