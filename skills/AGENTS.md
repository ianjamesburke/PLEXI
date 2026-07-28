# skills — Agent Contract

## Scope

User-facing agent skills shipped with Plexi. Currently `plexi-cli/` — the CLI
vocabulary an agent needs to drive a running Plexi host. Repo-internal workflow
skills (ship pipeline, babysitter, and friends) live under `.claude/skills/` and
`.agents/skills/` and are **never** published from here.

## Canonical Copy and the Published Mirror

`skills/plexi-cli/SKILL.md` in this repo is **canonical**. The public repo
[`ianjamesburke/plexi-skills`](https://github.com/ianjamesburke/plexi-skills) is a
manually published **mirror** consumed by the `npx skills` installer
(vercel-labs). Never edit the mirror directly; edit here, then republish.

Publish flow (manual, at stable release time):

1. Update `plexi_version` and `last_verified` in the SKILL.md frontmatter; bump
   `skill_version` if content changed.
2. Copy `skills/plexi-cli/SKILL.md` — that single file, never a directory glob —
   into the mirror repo at `skills/plexi-cli/SKILL.md`, commit, push.
3. Tag the mirror repo `v<plexi_version>` so users can pin
   (`npx skills add ianjamesburke/plexi-skills#v<version>`).

## Version Lockstep

The SKILL.md frontmatter `plexi_version` must equal the `Cargo.toml` version
being shipped at every stable release. `scripts/check-skill-version.sh` enforces
this; `scripts/promote.sh` runs it on the beta→main path and **fails the
release** on mismatch. Run manually with `just check-skill-version`.

## Content Rules

- Written for an outside user: the installed `plexi` binary and nothing else.
  No references to this checkout, `.stint/`, ship-pipeline skills, `just`
  recipes, channel shims, or worktree paths.
- Docs links point at `plexiapp.com`, never at repo-relative paths.
