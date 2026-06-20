# scripts — Agent Contract

**Read before editing anything under scripts/:** this file, plus the root AGENTS.md.

## Scope

Build, install, release, and channel management scripts. Called from `justfile` recipes. Run `just --list` for a self-documented recipe reference.

## Reference

- [RELEASE_CHANNELS.md](RELEASE_CHANNELS.md) — channel table, feature gates, RC flow, bare CLI shim, stable release flow.

## Stability Ladder

`alpha` → `beta` → `main`. All work lands on alpha first. Never commit directly to `beta` or `main`.

## Rules

- **Channel-agnostic.** Every script must work identically on all build channels.
- **Never hardcode profile paths.** Derive from binary name or `config_dir()`.
- Scripts are the only place `just` recipes call into. Do not duplicate logic in the justfile.
- `default-config.toml` is the config template seeded on install. Keep in sync with `src/config/CONFIG.md`.
- **Bump at release boundaries, not after every PR.** Run `just bump` once at end of a batch or before promoting.

## Traps

- **`cargo metadata` failure → silent install.** `scripts/install.sh` derives the bundle path via `cargo metadata`; if it fails (Python 3 missing, not in a workspace), the script exits immediately. If you change `[build] target-dir` in `.cargo/config.toml`, `install.sh` adapts automatically — but verify `cargo metadata` still succeeds.
- **Unpushed alpha commits are silently lost when a ship agent rebases.** `implement-issue` runs `git pull --rebase origin alpha` at Phase 1. Commits on local alpha that haven't been pushed will conflict and can be dropped. Every direct commit to alpha must be followed immediately by `git push origin alpha`.
- **PR build GUI won't launch when `PLEXI_SOCKET` is set.** `open -a "Plexi PR<N>"` inside a Plexi pane silently no-ops — the binary detects `PLEXI_SOCKET` and exits. Test scripts that need the PR build GUI must either run outside Plexi or `unset PLEXI_SOCKET` before the `open` call.
- **Uncommitted bump on alpha.** If `Cargo.toml` shows a dirty version bump, `just bump` ran but failed to commit. Commit manually with `git commit -m "chore: bump alpha to X.Y.Z"` before creating a worktree — otherwise the feature branch diverges from origin at a bump commit that isn't on origin, and `gh pr merge` will fail.
- **Session CWD for git commands.** Sessions start inside `worktrees/alpha/`. Run git commands bare (`git`, `wtp`, `just`, `gh`) for alpha; use absolute paths for feature worktrees.
- **Worktree dir gone after `wtp remove`.** Finish all file edits and cd away before cleanup steps.
- **Skill file edits don't need `bump + install`.** When the only change is `.claude/skills/*.md` or non-Rust config, commit directly to alpha. `just bump && just install` is only needed when Rust code changes should be reflected in the running build.
- **`just pr-install` must run from the feature worktree.** `scripts/install.sh` derives `REPO_ROOT` from `${BASH_SOURCE[0]}/..`. Running from the repo root syncs alpha's `apps/dev/`, missing any apps that only exist on the feature branch.

## Child DOX Index

- `default-scripts/` — default app scripts bundled into new user profiles.

## Style

Document stable contracts, not history. Update in the same change that makes a rule obsolete.
