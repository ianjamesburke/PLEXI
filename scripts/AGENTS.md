# scripts — Agent Contract

**Read before editing anything under scripts/:** this file, plus the root AGENTS.md.

## Scope

Build, install, release, and channel management scripts. Called from `justfile` recipes.

## Reference

- [RELEASE_CHANNELS.md](RELEASE_CHANNELS.md) — channel table, feature gates, RC flow, bare CLI shim, stable release flow.

## Build Channels & Isolated Profiles

Each build channel is a fully isolated instance with its own binary, app bundle, config dir, log file, secrets index, and apps. Channel is detected at runtime from the binary name.

| Channel | Binary | Profile dir | App bundle |
|---|---|---|---|
| Main | `plexi` | `~/.plexi/` | `Plexi.app` |
| Beta | `plexi-beta` | `~/.plexi-beta/` | `Plexi Beta.app` |
| Alpha | `plexi-alpha` | `~/.plexi-alpha/` | `Plexi Alpha.app` |
| Release candidate | `plexi-rc-<version>` | `~/.plexi-rc-<version>/` | `Plexi Rc-<version>.app` |
| PR build | `plexi-pr-<N>` | `~/.plexi-pr-<N>/` | `Plexi PR<N>.app` |

- **RC builds:** local stable-tier release candidates installed with `just channel-install rc-010`. Release gates treat `rc-*` exactly like stable/main.
- **PR builds:** ephemeral isolated instances installed by `just pr-install <N>` from the feature worktree. Never capture the bare `plexi` shim. Remove after merge with `just channel-clean pr-<N>`.
- **Alpha config stays default.** Reset on every `just install`. Never customize it. PR builds seed from alpha.
- **Beta config is the staging ground.** NOT reset on install. Use for migration testing and personal overrides.
- **Workspace** (`.plexi/workspace.toml`) is a separate per-project concept. Not the same as the profile dir. Never run `workspace init` from `~`.
- **PR build test instructions:** use `plexi-pr-<N>` (not `plexi`), and `cd` into a real project dir for workspace context.

## Branch Workflow

Three channels, each more stable than the last:

- `alpha` — active development. All work lands here first.
- `beta` — staging. Promoted from alpha.
- `main` — production. Promoted from beta.

Never commit directly to `beta` or `main`. Feature branch naming: `feature/<issue-number>-short-description`. Never pass `--delete-branch` to `gh pr merge`.

**Full ship cycle:** `/dispatch N` → implement-issue → open-pr → validate-pr → merge-pr. Labels track state.

### Promotion

Alpha → beta: `git push origin alpha:beta`, then `just install` from `worktrees/beta/`.

Beta → main: `just promote main` (pushes beta→main, creates/pushes version tag, triggers release).

RC before promotion: `just channel-install rc-010`.

**Worktree base is always local `HEAD`.** `wtp add` branches from the last local commit. Unpushed commits are included. Never stop worktree creation for dirty working tree, only for in-progress merge/rebase.

Worktrees: `.` (alpha), `worktrees/beta`, `worktrees/main`, `worktrees/feature/<branch>`, `worktrees/fix/<branch>`.

## Releases

1. `just bump [patch|minor|major]` — bumps version, generates CHANGELOG, commits, creates local tag.
2. `just promote beta` — pushes alpha→beta, syncs worktree.
3. Test on beta.
4. `just promote main` — pushes beta→main, pushes tag, triggers release.

**Bump at release boundaries, not after every PR.** Run `just bump` once at end of batch or before promote.

## Build & Install

- `just install` is manual. Do not run automatically after PR merge.
- **Never claim done from a feature worktree install.** Full done cycle: commit → PR → squash-merge → `git pull`.

## Rules

- **Channel-agnostic.** Every script must work identically on all build channels.
- **Never hardcode profile paths.** Derive from binary name or `config_dir()`.
- Scripts are the only place `just` recipes call into. Do not duplicate logic in the justfile.
- `default-config.toml` is the config template seeded on install. Keep in sync with `src/config/CONFIG.md`.

## Child DOX Index

- `default-scripts/` — default app scripts bundled into new user profiles.

## Style

Document stable contracts, not history. Update in the same change that makes a rule obsolete.
