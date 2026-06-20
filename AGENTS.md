## Purpose

`CLAUDE.md` is a symlink to this file. Cross-cutting rules for all agents. Domain-specific contracts live in each directory's own `AGENTS.md`.

Before editing any file, read the `AGENTS.md` in its directory if one exists. Child rules add to this file; they never override it.

## Source of Truth

- **What shipped** → `git log --oneline -20` and `GOTCHAS.md`
- **Product direction** → `NORTH_STAR.md`, `GLOSSARY.md`
- **Feature specs** → `docs/prm/*.md` (PRMs are the planning source of truth)
- **Sprint graph** → `.stint/` (`stint next`, `stint status`)
- **Implementation tickets** → GitHub issues

Do not track in-progress work or completion status in this file.

## Website

The product website is **`plexiapp.com`**. Never write `plexiapp.dev` or `plexi.app`.

## Stint Time Tracking

When work begins: `stint start <task-id>`. When done: `stint done <task-id>`. Use UTC timestamps. If abandoned, leave `started_at` in place, do not set `completed_at`.

## Child DOX Index

| Directory | Owns |
|---|---|
| `src/cli/` | CLI rules, channel-agnostic enforcement, namespace design, pane naming |
| `src/ui/` | Host UI kit primitives, design tokens, overlay layout widgets |
| `src/config/` | Config loading/validation, CONFIG.md reference |
| `src/testing/` | Test infrastructure, TESTING.md reference, scene format |
| `src/process_app/` | PGAP lifecycle, capability gating, security model, shell execution inventory |
| `src/render/` | CLI renderer app contract |
| `sdk/python/` | SDK traps, SDK_QUICKSTART.md, SDK_V2.md reference |
| `apps/` | App rules, Core 9 policy, design philosophy |
| `scripts/` | Build channels, branch workflow, releases, install, RELEASE_CHANNELS.md |
| `registry/` | CLI descriptor guide, embedded descriptor registry |
| `docs/` | Forward-looking PRMs only |

## Branches

`alpha` is the starting branch. Every feature branch, worktree, and PR originates from alpha. Never branch from `main` or `beta`. Feature branch naming: `feature/<issue-number>-short-description`.

## Git Rules

Never add `Co-Authored-By: Claude ...` trailers. Never push directly to `main` or `beta`. Never pass `--delete-branch` to `gh pr merge`.

## GitHub Issues

Always use the `/create-issue` skill. It owns labels, priority, area, and triage state. Never create issues manually.

## Planning

Read the relevant PRM first. Use `stint next` for the next claimable task. GitHub issues are implementation tickets. Pipeline labels (`pipeline:implement`, `pipeline:open-pr`, `pipeline:validate`, `pipeline:merge`) are the live work state.

## Logging

Log file: `~/.plexi-<channel>/plexi.log`. Rotates at 10 MB. Level set in `config.toml`.

Every new feature must be instrumented. No new capability, command, or user-visible behavior ships without at least one `info`-level trace.

## Testing

**Full reference: [`src/testing/TESTING.md`](src/testing/TESTING.md).** Observable state → TOML scene. Return value or invariant → Rust `#[test]`. `cargo test --bin plexi` must be green before any push.

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
- **Ideas become issues, not tangents.**
- **Direct-to-alpha when user is watching.**
- **Own the build.** If your change breaks something, fix it.

## General Rules

- When the user reports a bug, fix what they asked for first.
- Never use `#[allow(dead_code)]` or `#[allow(unused)]`. Delete or wire up.
- Always run `cargo build` after work.
- **Failed PR reset:** close the PR, revert worktree, comment on the issue, re-label `ready`, start fresh.
