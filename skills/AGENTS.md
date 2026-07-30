# skills — Agent Contract

## Scope

User-facing agent skills shipped with Plexi. Currently `plexi-cli/` — the CLI
vocabulary an agent needs to drive a running Plexi host. Repo-internal workflow
skills (ship pipeline, babysitter, and friends) live under `.claude/skills/` and
`.agents/skills/` and are **never** published from here.

## What the Skill Is Verified Against

**The skill documents the CLI built from the same tree it lives in.** Not "the
last release", not alpha-in-general: this commit. That is machine-enforced —
`src/cli/skill_surface.rs` (runs in `cargo test --bin plexi`) walks every fenced
code block in SKILL.md and fails if any documented subcommand path or `--flag`
is absent from the compiled clap tree, or if `plexi_version` differs from
`Cargo.toml`. Changing the CLI without updating the skill (or vice versa) is a
red test, not a convention.

The gate checks *surface existence*, not prose semantics. A sentence like
"default: context" is not machine-checked — behavioral claims still require
re-reading the skill against the CLI when they change, per the same-PR rule in
`src/cli/AGENTS.md`.

`plexi_version` is stamped by `just bump` (see `scripts/release-version.sh`),
never hand-edited alone. Between releases, alpha's copy carries the last
released version number while documenting alpha surface — that is fine
**because the copy in this repo is never what users install**; see below.

## Canonical Copy and the Published Mirror

`skills/plexi-cli/SKILL.md` in this repo is **canonical**. The public repo
[`ianjamesburke/plexi-skills`](https://github.com/ianjamesburke/plexi-skills) is a
manually published **mirror** consumed by the `npx skills` installer
(vercel-labs). Never edit the mirror directly; edit here, then republish.

The mirror is republished **only at stable release time, only from the release
tree** (the tree `promote.sh` promotes beta→main, after `just bump`). By then
the surface gate has run against that tree's own binary and `plexi_version`
equals the tagged version — so the published skill matches the released binary
by construction. Publishing the alpha copy mid-cycle is forbidden: it would
document unreleased surface under the released version number, which is exactly
the failure this contract exists to prevent.

Publish flow (manual; `just release main` prints this reminder):

1. From the release tree, copy `skills/plexi-cli/SKILL.md` — that single file,
   never a directory glob — into the mirror repo at
   `skills/plexi-cli/SKILL.md`, commit, push `main`.
2. Tag the mirror repo `v<plexi_version>` so users can pin
   (`npx skills add ianjamesburke/plexi-skills#v<version>`).

## Release Gate

`scripts/promote.sh` runs `cargo test --bin plexi skill_surface` in the beta
tree on the beta→main path and **fails the release** if the skill documents
surface that binary does not have or the version stamp is off.

## Content Rules

- Written for an outside user: the installed `plexi` binary and nothing else.
  No references to this checkout, `.stint/`, ship-pipeline skills, `just`
  recipes, channel shims, or worktree paths.
- Docs links point at `plexiapp.com`, never at repo-relative paths.
- Frontmatter: `plexi_version` = the tree's `Cargo.toml` version (stamped by
  `just bump`); `skill_version` bumps when content changes; `last_verified` =
  date the prose was last re-read against the CLI.
