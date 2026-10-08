# scripts — Agent Contract

**Read before editing anything under scripts/:** this file, plus the root AGENTS.md.

## Scope

Build, install, release, and channel management scripts. Called from `justfile` recipes. Run `just --list` for a self-documented recipe reference.

## Reference

- [DISTRIBUTION.md](DISTRIBUTION.md) — package, installation and publication contract.
- [RELEASE_CHANNELS.md](RELEASE_CHANNELS.md) — channel table, feature gates, RC flow, bare CLI shim, stable release flow.
- `folder-secrets-e2e.sh` — installed-binary check for folder-scoped secrets. The contract is `src/workspace/AGENTS.md`. On macOS it sets `PLEXI_KEYCHAIN_PATH` to `$WORK/test.keychain-db` and `PLEXI_KEYCHAIN_PASSWORD`, and it compares read-only `security default-keychain` and `security list-keychains` from before the run with the same commands after. It does not pass `-s` or a keychain path. The EXIT trap deletes that file with `rm`. On Linux it sets `PLEXI_FOLDER_SECRETS_BACKEND=encrypted-file-fallback` and starts a private session bus with an unlocked gnome-keyring so the permission MAC can be sealed. Folder values stay on the encrypted-file backend. It does not set a skip-panes variable. When `DISPLAY` and `WAYLAND_DISPLAY` are unset on Linux and Xvfb is installed, the script starts Xvfb so the pane checks still run. From a pane in folder A it requires `secret exec --cwd` of folder B, `env -u PLEXI_PANE_ID` of that exec, clearing `PLEXI_PANE_ID` and `PLEXI_SOCKET` on a channel-named binary, and `secret grant` and `secret read` of folder B to print `permission_denied` with no value and no pending id. A nested `pane new --cwd` of folder B must omit folder B's secret. The pane env must not contain `PLEXI_TERMINAL_ENV_VALUE_`. `secret grant` from the tester shell is the same refusal, and the following read does not return the value. `secret --help` must state that same-user native processes are not isolated. The pending read is approved by opening the assistant beside the terminal, sourcing `scripts/e2e/human.sh`, and calling `HUMAN_APPROVE` once; the next read is refused. `human.sh` places that click with `xwininfo`'s absolute window origin plus the accesskit rect. The binary comes from `PLEXI_BIN`, then `$1`, then `target/release/plexi`. The audit file is the channel profile of the binary under the private HOME (`channel_suffix_from_basename`: `plexi` → `.plexi`, `plexi-pr-<N>` → `.plexi-pr-<N>`).

## Stability Ladder

`alpha` → `beta` → `main`. All work lands on alpha first. Never commit directly to `beta` or `main`.

## Rules

- **Channel-agnostic.** Every script must work identically on all build channels.
- **Never hardcode profile paths.** Derive from binary name or `config_dir()`.
- Scripts are the only place `just` recipes call into. Do not duplicate logic in the justfile.
- `default-config.toml` is the config template seeded on install. Keep in sync with `docs/CONFIG.md`.
- **Bump at release boundaries, not after every PR.** Run `just bump` once at end of a batch or before promoting.
- **CHANGELOG is self-building.** `just bump` runs **git-cliff** (`cliff.toml`) to prepend unreleased conventional commits into `CHANGELOG.md`. Preview with `just changelog`. Do not hand-write release notes into CHANGELOG for a cut — fix commit messages if the cliff output is wrong. Full tag/promote flow: `RELEASE_CHANNELS.md`.
- **App seeding is channel-gated.** `packs/core.toml` is the single source of truth for the maintained/core app set (owned by `apps/AGENTS.md`). `package.py` includes maintained top-level app dirs on `alpha`/`pr-*`, discovered by `manifest.toml`; it must not flatten `apps/dev/` into the user-visible app registry. On `beta`/`main` it seeds exactly the canonical set through the host's own pack applier (refresh keyed by compiled build identity; examples on a fresh profile) so no app list is duplicated here. Never enumerate app names in this script.

## Traps

- **Unpushed alpha commits are silently lost when a ship agent rebases.** `implement-issue` runs `git pull --rebase origin alpha` at Phase 1. Commits on local alpha that haven't been pushed will conflict and can be dropped. Every direct commit to alpha must be followed immediately by `git push origin alpha`.
- **PR build GUI won't launch when `PLEXI_SOCKET` is set.** `open -a "Plexi PR<N>"` inside a Plexi pane silently no-ops — the binary detects `PLEXI_SOCKET` and exits. Test scripts that need the PR build GUI must either run outside Plexi or `unset PLEXI_SOCKET` before the `open` call.
- **Uncommitted bump on alpha.** If `Cargo.toml` shows a dirty version bump, `just bump` ran but failed to commit. Commit manually with `git commit -m "chore: bump alpha to X.Y.Z"` before creating a worktree — otherwise the feature branch diverges from origin at a bump commit that isn't on origin, and `gh pr merge` will fail.
- **Session CWD for git commands.** Sessions start inside `worktrees/alpha/`. Run git commands bare (`git`, `wtp`, `just`, `gh`) for alpha; use absolute paths for feature worktrees.
- **Worktree dir gone after `wtp remove`.** Finish all file edits and cd away before cleanup steps.
- **Skill file edits don't need `bump + install`.** When the only change is `.claude/skills/*.md` or non-Rust config, commit directly to alpha. `just bump && just install` is only needed when Rust code changes should be reflected in the running build.
- **`scripts/install.sh` derives `REPO_ROOT` from `${BASH_SOURCE[0]}/..`** — it installs whatever tree it lives in. Never call it directly for a PR build; `just pr-install <N>` resolves the PR's head into the right worktree and runs the script from there (safe from any cwd).
- **Login bash does not read `~/.bashrc`.** Host panes exec `bash -i -l` (`apply_initial_cmd`). Bash completion registration has to land on the login file in `prepare_path_registration`; a `.bashrc`-only snippet never reaches a pane. See `scripts/DISTRIBUTION.md`.
- **`just merge-pr` must run from the canonical alpha checkout.** Stint state lives in ignored `.stint/` files that feature worktrees may not have. If a PR body references stint IDs, running merge closeout from a feature worktree can fail before merge with missing `.stint/tasks`; rerun from `/Users/ianburke/Documents/GitHub/PLEXI` on `alpha`.

## Child DOX Index

- `default-scripts/` — default app scripts bundled into new user profiles.

## Style

Document stable contracts, not history. Update in the same change that makes a rule obsolete.
