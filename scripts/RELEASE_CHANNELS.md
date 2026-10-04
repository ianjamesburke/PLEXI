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
the experimental DAW, media I/O, accessibility, Assistant, and
marketplace/account flows. CLI and MCP app wrappers are part of stable v1. Use config sections for
stable user preferences, such as `[effects]`.

## Local RC Flow

Create an isolated stable-tier install without touching beta:

```sh
just channel-install rc-010
plexi-rc-010 --version
just channel-list
```

Installation destinations and ownership are defined in [Distribution contract](DISTRIBUTION.md).

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

## CLI launchers

The contextual bare command and channel-scoped launchers are defined in
[Distribution contract](DISTRIBUTION.md).

## Release Tags

The package requirements, unsigned policy and publication gates live in
[Distribution contract](DISTRIBUTION.md).

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
just release beta                  # publish vX.Y.Z-beta.N binary release, trigger CI
just promote main                  # beta→main
just release main                  # publish vX.Y.Z binary release, trigger CI
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

Release selection, channel acceptance, immutable identity and rollback are
defined in [Distribution contract](DISTRIBUTION.md).
