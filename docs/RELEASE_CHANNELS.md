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

Workspace config for that RC lives under `.plexi-rc-010/` inside the project
root. For example:

```text
my-project/.plexi-rc-010/config.toml
```

That config affects preferences only. It does not change release tier. The
running binary name decides release-gated feature availability.

## Stable Release Flow

1. Finish release polish on `alpha`.
2. Run tests.
3. Run `just bump minor` for `0.1.0`.
4. Install and test `rc-010`.
5. Promote alpha to beta with `just promote beta`.
6. Test beta.
7. Promote beta to main with `just promote main`.

Do not use `plexi-beta update` to test unreleased alpha work. `plexi update`
downloads the latest public GitHub release.
