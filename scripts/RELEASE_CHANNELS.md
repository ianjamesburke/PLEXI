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

| Lane | Tag scheme | Branch tagged from |
|---|---|---|
| Alpha | `vX.Y.Z-alpha.N` | `alpha` |
| Beta | `vX.Y.Z-beta.N` | `alpha` |
| Stable | `vX.Y.Z` | `main` |

The base `X.Y.Z` is the current `Cargo.toml` version. Prerelease tags do **not**
bump `Cargo.toml` or touch the changelog — they pin a commit on `alpha` for
testing the next version. The stable bump (`just bump`) sets the base version and
regenerates the changelog.

Alpha tags are cut nightly by `.github/workflows/nightly.yml` when `alpha` has
commits since the last alpha tag (`scripts/nightly-alpha-tag.sh` picks the next
`N`); beta and stable stay manual. Pushes to `alpha` run no Rust host CI, only
pull requests do.

### Signed publish approval

Every job that can read the release signing identity runs in the GitHub
environment `release-signing`, and that includes nightly alpha, `verify_only`
and `signing_only` runs. The job starts only after a required reviewer
approves it in the Actions run. Publication depends on the signed macOS
package job, so nothing publishes without that approval.

The boundary is the server-side reviewer rule on that environment, together
with the signing secrets existing only as its environment secrets.
`scripts/release-approval-gate.py`, the first step of each signing job, is a
misconfiguration check. It refuses unless the environment names a required
reviewer, forbids administrator bypass and restricts deployment refs, so a
missing or auto-created environment stops signing. Signing does not fail
closed while repository-level copies of the signing secrets exist: any job,
without an environment, can read those. `scripts/test_release_approval.py`
(the `Release approval lint` workflow, and `just test-infra-scripts`) fails
when a job uses the identity or the environment outside the gated jobs, reads
secrets wholesale, or publishes release assets outside `release.yml`.

Reviewers: check which commit a run signs before approving. Anyone who can
push can create a `v*` tag or dispatch the workflow on any commit.

Repository settings this depends on (Settings > Environments >
`release-signing`):

- Required reviewers: the release owner. Prevent self-review stays off, since
  the owner dispatches most runs.
- "Allow administrators to bypass configured protection rules" is unticked.
- Deployment branches and tags: selected, `alpha`, `beta`, `main` and tags `v*`.
- Environment secrets `PLEXI_ALPHA_SIGNING_P12_BASE64`,
  `PLEXI_ALPHA_SIGNING_P12_PASSWORD` and `PLEXI_ALPHA_SIGNING_CERT_SHA1`. The
  repository-level secrets with those names are deleted.

The approval is only as strong as the reviewer's GitHub credential. A token
that can approve deployments, including a `gh` login on a machine where agents
run, can approve a signing job too.

Open question: `alpha` has branch protection without required reviews or
status checks, and `beta` and `main` have none, so any writer can put a commit
on a deployable ref. Whether to require reviews there is undecided.

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

## Round-trip acceptance

The bump, promote and release flow above is exercised end to end, without publishing, by `scripts/release-round-trip-e2e.sh` (CI: `release-round-trip.yml`). Contract: `scripts/AGENTS.md`.
