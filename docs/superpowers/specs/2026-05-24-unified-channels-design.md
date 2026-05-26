# Unified Channel Primitive

Replace the ad-hoc channel handling (hardcoded match arms in Rust, separate PR vs non-PR scripts, split justfile recipes) with a single channel primitive that works for any build: alpha, beta, main, PR, gpui, or any future long-lived branch.

Also renames the "stable" channel to "main" so that every branch name IS the channel name with zero translation.

## Channel definition

A channel is a named build variant. Every channel produces exactly three artifacts, derived from its name:

| Artifact | Formula | `main` channel (bare name) |
|---|---|---|
| Binary | `/usr/local/bin/plexi-{name}` | `/usr/local/bin/plexi` |
| App bundle | `/Applications/Plexi {Name}.app` | `/Applications/Plexi.app` |
| Profile dir | `~/.plexi-{name}/` | `~/.plexi/` |

No other state. No registry file. Discovery is a filesystem scan of `/usr/local/bin/plexi*`.

**Rename: "stable" -> "main".** The channel name matches the branch name. No translation layer. `main` is the channel that gets the bare (unsuffixed) binary, app bundle, and profile dir.

## Channel tiers

Tier is inferred from the channel name by exact match. Not substring, not contains.

| Tier | Names | Bare symlink | Completions | Auto-clean |
|---|---|---|---|---|
| production | `main`, `alpha`, `beta` (exact) | main only | yes | never |
| development | any other name | never | no | manual |
| ephemeral | `pr-*` | never | no | `channel-clean-merged` |

The bare `/usr/local/bin/plexi` symlink always points to `main`. If `main` is not installed, the symlink does not exist.

## Justfile commands

```
just channel-install [name]     # Build + install. Auto-detects from branch if omitted.
just channel-clean <name>       # Remove one channel (app, binary, profile).
just channel-clean-merged       # Remove ephemeral channels whose GitHub PR is closed.
just channel-list               # Show all installed channels with tier, binary path, profile dir.
just channel-uninstall          # Remove ALL channels. Interactive confirm.
```

**Aliases for muscle memory:**
- `just install` = `just channel-install` (auto-detect from branch)
- `just pr-install <N>` = `just channel-install pr-<N>`
- `just pr-clean <N>` = `just channel-clean pr-<N>`

**Branch-to-channel auto-detection** (`just channel-install` with no argument):
- `main` -> `main`
- `alpha` -> `alpha`
- `beta` -> `beta`
- Any other branch -> error with message: "Cannot auto-detect channel from branch '{name}'. Use: just channel-install <channel-name>"

Every branch name IS the channel name. Zero translation. This prevents nonsensical channels like `plexi-fix/crash`.

## Rust changes

### config_dir_name() (src/config.rs)

Replace the hardcoded match cascade with a generic suffix extractor:

1. Extract the **basename** of `current_exe()` (not the full path).
2. If the basename starts with `plexi-`, the suffix is everything after `plexi-`. Profile dir = `~/.plexi-{suffix}/`.
3. If the basename is exactly `plexi`, profile dir = `~/.plexi/`.
4. `--profile <name>` override remains highest priority.

This handles any channel name without code changes. The `v3` match arm is dead code and gets deleted.

### Basename-first is load-bearing

The current code runs `contains()` on the full binary path. A user with `/Users/alpha/bin/plexi` would resolve to `.plexi-alpha`. The generic extractor MUST extract the filename before matching.

## Script changes

### install.sh

- Bare symlink: only captured when `channel == "main"` (currently captured for any non-PR channel).
- Completions: installed only for production tier (exact match on main/alpha/beta).
- Everything else already works generically.

### pr-clean.sh -> channel-clean.sh

Generalize to accept any channel name, not just PR numbers. Same three-artifact removal: app bundle, binary, profile dir. The script derives all three paths from the channel name using the same formula as install.sh.

### uninstall.sh

Replace the hardcoded case statement with filesystem discovery. Scan `/usr/local/bin/plexi*` to find all installed channels, then call `uninstall_channel()` for each. The `uninstall_channel()` function is already generic.

### pr-clean-merged.sh -> channel-clean-merged.sh

Stays ephemeral-only (pr-* channels). Must check GitHub PR state with `gh pr view <N> --json state`, not git branch existence.

### channel-list.sh (new)

Scan `/usr/local/bin/plexi*` and `/Applications/Plexi*.app`. For each, derive the channel name and tier. Output a table:

```
Channel    Tier         Binary                        Profile
main       production   /usr/local/bin/plexi          ~/.plexi/
alpha      production   /usr/local/bin/plexi-alpha    ~/.plexi-alpha/
gpui       development  /usr/local/bin/plexi-gpui     ~/.plexi-gpui/
pr-817     ephemeral    /usr/local/bin/plexi-pr-817   ~/.plexi-pr-817/
```

## Constraints

- Tier inference is exact-match only. `beta-2` is development tier, not production.
- No registry file. All state is derived from filesystem artifacts.
- `channel-clean main` removes the bare symlink.
- `channel-clean-merged` checks `gh pr view`, not git branch existence.
- The `just install` alias preserves existing behavior for alpha/beta/main branches. No surprises.

## Stable -> main migration

The rename from "stable" to "main" touches every reference to the word "stable" as a channel name:

### What changes
- `config_dir_name()`: remove `"stable"` concept entirely (generic extractor replaces it)
- `install.sh`: `_git_channel()` returns `"main"` instead of `"stable"` for branch `main`; bare symlink condition changes from `channel == "stable"` to `channel == "main"`
- `uninstall.sh`: replace `stable` case arm with `main`
- `promote.sh`: any references to "stable" channel naming
- `CLAUDE.md`: all references to "Stable" channel in the Build Channels table and prose
- `justfile`: update comments referencing "stable"
- `scripts/default-config.toml`: if it references "stable" anywhere
- `clear-apps.sh`: update help text

### What does NOT change
- The `main` git branch name stays the same
- `~/.plexi/` (no suffix) stays the same, it's just the `main` channel now instead of `stable`
- `/Applications/Plexi.app` stays the same
- `/usr/local/bin/plexi` stays the same
- The promotion flow (`just promote main`) stays the same

### Testing

Before the rename ships, these must be verified:

**Rust unit tests (add to src/config.rs):**
- `config_dir_name()` with binary basename `"plexi"` -> `".plexi"`
- `config_dir_name()` with binary basename `"plexi-alpha"` -> `".plexi-alpha"`
- `config_dir_name()` with binary basename `"plexi-gpui"` -> `".plexi-gpui"`
- `config_dir_name()` with binary basename `"plexi-pr-817"` -> `".plexi-pr-817"`
- `config_dir_name()` with binary at `/Users/alpha/bin/plexi` -> `".plexi"` (not `.plexi-alpha`)
- `config_dir_name()` with `--profile` override -> `".plexi-{override}"` regardless of binary name

**Script integration tests (shell, run in CI or manually):**
- `just channel-install` from `main` branch -> installs bare `plexi` binary, `Plexi.app`, `~/.plexi/`
- `just channel-install` from `alpha` branch -> installs `plexi-alpha`, `Plexi Alpha.app`, `~/.plexi-alpha/`
- `just channel-install gpui` -> installs `plexi-gpui`, `Plexi Gpui.app`, `~/.plexi-gpui/`
- `just channel-install pr-99` -> installs `plexi-pr-99`, no bare symlink, no completions
- `just channel-list` -> shows all installed channels with correct tiers
- `just channel-clean gpui` -> removes all three artifacts
- `just channel-clean main` -> removes bare symlink too
- `just install` from unrecognized branch -> errors with helpful message

**Grep audit before merging:**
- `grep -rn "stable" scripts/ src/ justfile CLAUDE.md` -> zero hits referencing "stable" as a channel name (the word "stable" in prose like "more stable" is fine)
