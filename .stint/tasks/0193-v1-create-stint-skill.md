---
id: "0193"
title: "v1: create-stint skill"
status: done
estimate: "2h"
actual: "1m"
completed_at: "2026-06-14T07:20:24Z"
variance: "Wrote skill directly — 60 seconds actual vs 2h estimate. ~120x off. Estimate assumed human writing pace; agent wrote the full SKILL.md in one pass."
sprint: "s14"
blocked_by: []
gh_issue: []
area:
  - "infra/agents"
tags:
  - "v1"
  - "tooling"
  - "skills"
  - "stint"
---


Build a `create-stint` skill analogous to `create-issue` — a structured, agent-invocable flow for adding new stint tasks with correct metadata from the start.

## Motivation

`create-issue` enforces label conventions, checks for duplicates, and ensures North Star alignment before creating a GitHub issue. Stint tasks get no equivalent guidance — agents write them by hand, often missing area, estimate, sprint assignment, or blocker links. A skill closes that gap.

## Scope

- Create `.agents/skills/create-stint/SKILL.md`.
- The skill must guide the agent through:
  1. **Title check** — run `stint list` and grep for near-duplicate titles before creating.
  2. **Sprint selection** — read sprint goals (`stint sprint list` + `stint sprint show <id>`) and recommend the sprint whose goal the new task serves; default to `backlog` status if none fits.
  3. **Area assignment** — enumerate valid area values from existing tasks; require at least one.
  4. **Estimate** — require an explicit estimate (e.g. `1h`, `4h`, `1d`); disallow bare integers. Warn that agent estimates run ~10x high because agents assume human coding speed — actual agent implementation is typically 10x faster, so divide naive estimates accordingly before writing.
  5. **Tags** — at minimum `v1` or `v2`; flag if neither is present.
  6. **Blocker check** — ask whether the task is blocked by any existing task or GitHub issue; if yes, emit correct `blocked_by` syntax.
  7. **gh_issue link** — if a corresponding GitHub issue already exists or should be created, record `gh_issue`.
  8. **Write** — emit the final YAML frontmatter + body as a `.stint/tasks/<NNNN>-<slug>.md` file using the next available ID (from `stint list --json` or `ls .stint/tasks/`).
  9. **Validate** — run `stint check` and surface any errors.
- The skill must be triggered by phrases like "add a stint task", "create a stint", "new stint", or when the agent identifies planning work that should be tracked.

## Non-Scope

- Do not add a `start-stint` or `done-stint` flow in this skill — those are already covered by the `implement-stint` skill and CLAUDE.md timing rules.
- Do not create a GitHub issue automatically — that's `create-issue`'s job; cross-link only.

## References

- `.agents/skills/create-issue/SKILL.md` — pattern to follow
- `CLAUDE.md` — stint timing rules and blocker syntax reference
- `.stint/tasks/` — existing tasks for area/tag vocabulary
