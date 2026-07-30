# Release Channels

Plexi uses one code version and multiple isolated runtime channels.

| Channel | Binary | Profile dir | Tier |
|---|---|---|---|
| Stable | `plexi` | `~/.plexi/` | stable |
| Release candidate | `plexi-rc-<version>` | `~/.plexi-rc-<version>/` | stable |
| Beta | `plexi-beta` | `~/.plexi-beta/` | beta |
| Alpha | `plexi-alpha` | `~/.plexi-alpha/` | alpha |
| PR | `plexi-pr-<N>` | `~/.plexi-pr-<N>/` | alpha |

## Versioning

Use one semantic version from `Cargo.toml`. Do not maintain separate
alpha/beta/stable version streams. Channels are isolated install targets and
feature policy tiers; the version identifies the commit that is being tested or
released.

For a stable `0.1.0` release, alpha, beta, RC, and main all report `0.1.0`
when they are built from the same release commit.

## Feature Gates

Release-gated features are centralized in `src/release.rs`.

- `main` and `rc-*` are stable tier.
- `beta` is beta tier.
- `alpha` and `pr-*` are alpha tier.
- Unknown named channels disable release-gated features.

Use release gates for product surface that is not part of stable v1, such as
Assistant, app wrappers, and marketplace/account flows. Use config sections for
stable user preferences, such as `[effects]`.

## Local RC Flow

Create an isolated stable-tier install without touching beta:

```sh
just channel-install rc-010
plexi-rc-010 --version
just channel-list
```

This creates:

- `/Applications/Plexi Rc-010.app`
- `/usr/local/bin/plexi-rc-010`
- `~/.plexi-rc-010/`

RC installs also install shell completions for the channel binary, for example
`_plexi-rc-010` for zsh.

After installing a new or updated channel, open a fresh Plexi pane before testing
completion behavior. Existing zsh sessions may keep an old completion cache; if
autocomplete still looks stale, reset it in that pane:

```sh
rm -f ~/.zcompdump*
autoload -Uz compinit
compinit
```

Workspace config for that RC lives under `.plexi-rc-010/` inside the project
root. For example:

```text
my-project/.plexi-rc-010/config.toml
```

That config affects preferences only. It does not change release tier. The
running binary name decides release-gated feature availability.

## Bare CLI Shim

On macOS, `/usr/local/bin/plexi` is a contextual shim, not a direct symlink to
the stable app-bundle binary. The real stable binary remains:

```text
/Applications/Plexi.app/Contents/MacOS/plexi
```

When `PLEXI_CHANNEL` is set inside a Plexi PTY and the matching
`/usr/local/bin/plexi-$PLEXI_CHANNEL` binary exists, the shim delegates to that
channel binary with the original arguments. Otherwise it falls back to the
stable app-bundle binary.

Examples:

```sh
# inside a beta PTY
plexi app browse      # runs plexi-beta app browse

# inside an rc-010 PTY
plexi app browse      # runs plexi-rc-010 app browse, stable-tier gates apply

# outside Plexi
plexi app browse      # runs stable Plexi
```

Host commands that use `PLEXI_SOCKET`, such as `plexi pane info`, route to the
running instance. Binary-local behavior, such as config paths, workspace paths,
update/install behavior, and release gates, comes from whichever binary the
shim executes.

## Release Tags

Three lanes publish source-build tags. The updater resolves these tags, checks
out the selected commit, and builds it locally. `.github/workflows/release.yml`
records tag metadata for that lookup. It does not publish binary assets.

| Lane | Tag scheme | Branch tagged from |
|---|---|---|
| Alpha | `vX.Y.Z-alpha.N` | `alpha` |
| Beta | `vX.Y.Z-beta.N` | `alpha` |
| Stable | `vX.Y.Z` | `main` |

The base `X.Y.Z` is the current `Cargo.toml` version. Prerelease tags do **not**
bump `Cargo.toml` or touch the changelog — they pin a commit on `alpha` for
testing the next version. The stable bump (`just bump`) sets the base version and
regenerates the changelog.

SemVer ordering: `vX.Y.Z-alpha.1 < alpha.2 < beta.1 < beta.2 < vX.Y.Z`.
The Python SDK publishes only on stable tags; prerelease tags skip
`publish-sdk.yml`.

### Cutting a release

Preview unreleased commits without committing anything:

```sh
just changelog
```

`just promote` moves code between branches; `just release` cuts and publishes
the tag. They're deliberately separate: moving code to beta or main is
reversible and local, publishing a tag is not — other machines on that
channel auto-update to it. Never bundled into one command.

Standard release batch:

```sh
just bump                          # bump Cargo.toml, write CHANGELOG, commit + tag locally
just promote beta                  # alpha→beta
just release beta                  # publish vX.Y.Z-beta.N source-build tag, trigger CI
just promote main                  # beta→main
just release main                  # publish vX.Y.Z source-build tag, trigger CI
```

Promote code only and stop there (test locally before publishing):

```sh
just promote beta                  # alpha→beta, no tag
just promote main                  # beta→main, no tag
```

Add `install` to `promote` to build and install that channel after promoting:

```sh
just promote beta install
just promote main install
```

## Channel Update Policy

A binary updates only to releases its channel accepts. The policy lives in
`UpdateChannel::accepts` (`src/cli/release_resolver.rs`); the table below mirrors
it.

| Binary channel | Accepts |
|---|---|
| `plexi` (stable) | stable only |
| `plexi-beta` | beta + stable |
| `plexi-alpha`, `plexi-pr-*` | alpha + beta + stable |

`plexi update` lists all published tags (not just `/latest`), filters by the running
binary's channel, picks the highest SemVer candidate newer than the current
version, checks out that exact tag in `~/.plexi-src`, and rebuilds. The install
target channel is always the running binary's channel — updating `plexi-alpha`
to a stable tag still installs as `alpha`.

Do not use `plexi-beta update` to test unreleased alpha work — beta never
accepts alpha tags.
