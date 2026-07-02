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
| `src/config/` | Config loading/validation, CONFIG.md reference |
| `src/testing/` | Test infrastructure, TESTING.md reference, scene format |
| `src/process_app/` | PGAP lifecycle, capability gating, security model, shell execution inventory |
| `src/render/` | CLI renderer app contract |
| `sdk/python/` | SDK traps, SDK_V3.md reference |
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

Log file: `~/.plexi-<channel>/plexi.log`. Rotates at 10 MB. Level set in `config.toml`.

Every new feature must be instrumented. No new capability, command, or user-visible behavior ships without at least one `info`-level trace.

## Testing

**Full reference: [`src/testing/TESTING.md`](src/testing/TESTING.md).** Observable state → TOML scene. Return value or invariant → Rust `#[test]`. `cargo test --bin plexi` must be green before any push.

**`just pr-install <N>` must run from the relevant feature worktree.** The recipe runs pre-install tests against the current working tree before building. Running it from alpha/root validates and installs the wrong tree.

Test-first for host logic. Define done by the test, not the code. No partial merges.

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

## Architecture

**HostModel** is a pure state machine with zero egui dependency. Commands in, effects out. All business logic (pane lifecycle, permissions, events) lives here. The renderer (egui in prod, headless in CI) reads state and paints — it never owns logic.

## General Rules

- When the user reports a bug, fix what they asked for first.
- Never use `#[allow(dead_code)]` or `#[allow(unused)]`. Delete or wire up.
- Always run `cargo build` after work.
- **Failed PR reset:** close the PR, revert worktree, comment on the issue, re-label `ready`, start fresh.
