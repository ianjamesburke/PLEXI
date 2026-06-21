---
name: create-stint
description: Use this skill when the user wants to add a stint task, create a ticket, track a unit of work, file a bug, propose a feature, or log something to .stint/. Triggered by phrases like "add a task", "create a stint", "new task", "track this", "file a bug", "log a ticket", or when planning work that should go in the operating graph.
risk: low
source: local
date_added: "2026-06-19"
---

# Create Stint

Task creation flow only. Do not implement anything. The goal is a single, well-scoped stint task in `.stint/tasks/` with correct metadata, optionally linked to a GitHub issue.

Stint is the operating graph. Sprint order, size, blocking, and area tags. A task without correct metadata derails `stint next` and bottleneck analysis.

---

## Step 1 -- Duplicate Check

Before writing anything, scan existing tasks:

```bash
stint list 2>&1
```

Skim titles for near-matches. If one exists:
- **Same scope** -- surface it and stop. Add missing context to the existing task body instead.
- **Overlapping but distinct** -- note the relationship and proceed; set `blocked_by` or cross-reference in the body.

Also check recent GitHub issues for overlap:

```bash
gh issue list --limit 100 --state all --json number,title --jq '.[] | "\(.number) \(.title)"' 2>/dev/null | head -50
```

If a near-duplicate issue exists:
- **Superset** -- the new task covers everything plus more. Create the task, close the old issue with a comment pointing to the new work.
- **Additive** -- extract what's new, ask if it should be folded into the existing issue/task.
- **Duplicate** -- surface it and stop.

---

## Step 2 -- Research

Before drafting, investigate the relevant code:

- Find the files, functions, or modules most related to the task
- Identify every file that will need to change and what changes in each
- Note risks, gotchas, or non-obvious constraints

For bug tasks, verify the root cause by reading code. Never state an unverified root cause as fact. If a claim can't be verified, mark it as an unconfirmed hypothesis in `## Open Questions`.

Grep `GOTCHAS.md` for related pitfalls if it exists.

This research feeds into the task body. It is not a separate deliverable.

---

## Step 3 -- Sprint Selection

```bash
stint sprint list 2>&1
```

For any sprint that looks relevant:

```bash
stint sprint show <id> 2>&1
```

Rules:
- Place the task in the earliest sprint whose goal it serves.
- If no sprint goal fits, omit the sprint field entirely.
- Infrastructure and tooling tasks default to the current active sprint unless they clearly belong elsewhere.

---

## Step 4 -- Metadata Assembly

### Size (mandatory, always set, never omit or ask)

| Size | Meaning |
|---|---|
| `S` | A few hours, 1-3 files |
| `M` | A day or two, multiple files or systems |
| `L` | Multi-day, architectural or cross-cutting |

Pick one yourself from the research in Step 2. Never ask the user. Never leave it blank.

**No `time_estimate` field.** Never add it. Size is the only effort signal.

### Priority (mandatory, pick one)

- `P0` -- on fire
- `P1` -- shipping blocker
- `P2` -- important, not blocking
- `P3` -- polish
- `P4` -- backlog

### Area (mandatory, at least one)

Pull vocabulary from existing tasks:

```bash
grep -h "^  - " .stint/tasks/*.md | sort -u 2>/dev/null | head -40
```

Use strings already in use. Common areas:
- `host/pane-ops`, `host/config`, `host/terminal`, `host/permissions`, `host/notifications`, `host/secrets`
- `ui/chrome`, `ui/overlays`, `ui/widgets`, `ui/tile-tree`, `ui/sidebar`
- `sdk/pgap`, `sdk/python`
- `cli/commands`, `cli/completions`
- `apps/file-browser`, `apps/github-issues`, `apps/examples`
- `infra/build`, `infra/docs`, `infra/agents`, `infra/testing`, `infra/skills`

### Tags (mandatory, at least one)

At minimum one of `v1` or `v2`. Add domain tags as appropriate (`ui`, `tooling`, `testing`, `sdk`, etc.).

### Blockers

Ask: does this task require another task or issue to be done first?

| Syntax | Meaning |
|---|---|
| bare integer | local stint task (e.g. `153`) |
| `@N` | local GitHub issue |
| `owner/repo@N` | external GitHub issue |
| quoted string | free-text note |

If no real dependency exists, leave `blocked_by: []`.

### GitHub Issue

If a GitHub issue should be created for this work, create it in Step 6. If one already exists, record its number in `gh_issue`.

---

## Step 5 -- Write the Task File

Find the next available ID:

```bash
ls .stint/tasks/ | grep -oE '^[0-9]+' | sort -n | tail -1
```

Increment by 1, zero-pad to 4 digits. Confirm no collision.

File path: `.stint/tasks/<NNNN>-<kebab-slug>.md`

Template:

```markdown
---
id: "<NNNN>"
title: "<title>"
status: todo
priority: <p0-p4>
size: <S|M|L>
sprint: "<sN>"
blocked_by: []
gh_issue: []
area:
  - "<area/one>"
tags:
  - "<v1|v2>"
---

<One paragraph -- what this task is and why it exists.>

## Scope

- <Bullet: exactly what gets built or changed>

## Non-Scope

- <Bullet: what explicitly is NOT in this task>

## Why

<One sentence on the motivation or user impact.>

## References

- `<path>` -- <why relevant>
```

Omit sections that have nothing to say.

---

## Step 6 -- GitHub Issue (only when explicitly requested)

Do not create a GitHub issue unless the user explicitly asks for one. Stint tasks stand alone. If the user does request an issue, link it in the task's `gh_issue` field.

---

## Step 7 -- Validate

Run `stint check` once at the end of the session, not after every individual task. If the user asks for multiple tasks in one request, write them all first, then validate once.

```bash
stint check 2>&1
```

Fix any errors before returning.

---

## Step 8 -- Return and Recommend

Return the task ID, file path, and GitHub issue URL if created.

```
RECOMMENDATION:
1. <one call -- either "dispatch via /implement-stint <NNNN>" or "park it -- task is ready when the sprint opens">
```
