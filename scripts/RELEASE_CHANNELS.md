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

Experiments are centralized in `src/release.rs`. Channels do not decide
features: every experiment is off by default on every channel, including alpha,
and is turned on per user with `plexi experiments enable <name>` (see
`docs/CONFIG.md`). Channels still decide update cadence and profile directory.

Use an experiment for product surface that is not part of stable v1. Use config
sections for stable user preferences, such as `[effects]`. CLI app wrappers
(`app open --cli`) are part of stable v1; `--mcp` wrappers are the `mcp_client`
experiment.

## Local RC Flow

Create an isolated stable-tier install without touching beta:

```sh
just channel-install rc-010
plexi-rc-010 --version
just channel-list
```

Installation destinations and ownership are defined in [Distribution contract](DISTRIBUTION.md).

After installing a new or updated channel, open a fresh Plexi pane before testing
completion behavior. Shell registration and cache handling follow the
[Distribution contract](DISTRIBUTION.md).

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

Releases are cut by `.github/workflows/release.yml` (tag chosen by
`scripts/release-next-tag.sh`). Apple Silicon only. A started release always
finishes.

| Branch | Publishes | Tag |
|---|---|---|
| `alpha` | nothing; install from source with `just channel-install alpha` | none |
| `beta` | one scheduled nightly run (03:00 UTC-7) | `vX.Y.Z-beta.N` |
| `main` | a push, made by `just promote main` | `vX.Y.Z` |

The nightly run does nothing when alpha has no commits since the last beta tag.
Otherwise it builds, tests and packages alpha's head commit; only if everything
passes it tags that exact commit (never a newer alpha tip) and publishes the
next beta, then fast-forwards the `beta` branch to that commit. `just promote beta` starts the same run
by hand.

Per-push alpha releases cost about 55 hosted minutes each and served no one but
the owner. The full suite now gates the nightly beta instead of each merge.

`X.Y.Z` is `Cargo.toml`'s version. Once `vX.Y.Z` is released, alpha and beta
count on the next patch, because SemVer ranks `X.Y.Z-alpha.N` below `X.Y.Z`
and the updater only offers newer versions. `main` fails the release when it
has commits but `vX.Y.Z` already exists: bump first. The Python SDK publishes
on stable tags only.

### Cutting a release

```sh
just changelog       # preview unreleased commits
just bump            # on alpha: version + CHANGELOG commit (minor|major optional); push it
just promote beta    # optional: start the nightly beta release now; CI publishes vX.Y.Z-beta.N
just promote main    # fast-forward main to the newest beta tag's commit; CI publishes vX.Y.Z
```

The full host suite with PTY tests runs on demand via the `Rust host`
workflow (`workflow_dispatch`).

## Channel Update Policy

Release selection, channel acceptance, immutable identity and rollback are
defined in [Distribution contract](DISTRIBUTION.md).
