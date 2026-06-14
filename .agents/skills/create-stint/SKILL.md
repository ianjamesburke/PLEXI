---
name: create-stint
description: Use this skill when the user wants to add a stint task, track a unit of work, or log something to .stint/. Triggered by phrases like "add a stint task", "create a stint", "new stint task", "track this in stint", or when planning work that should go in the operating graph.
risk: low
source: local
date_added: "2026-06-14"
---

# Create Stint Skill

This is a **task creation flow only**. Do not implement anything. The goal is a single, well-scoped stint task in `.stint/tasks/` with correct metadata.

GitHub issues are implementation tickets. Stint is the operating graph — sprint order, estimates, timing, and blockers. A task without correct metadata derails future `stint next` output and bottleneck analysis.

---

## Step 1 — Duplicate Check

Before writing anything, scan existing tasks:

```bash
stint list 2>&1
```

Skim titles for near-matches. If one exists:
- **Same scope** → surface it to the user and stop. Add any missing context to the existing task body instead.
- **Overlapping but distinct** → note the relationship and proceed; set `blocked_by` or cross-reference in the body.

---

## Step 2 — Sprint Selection

Read the sprint list and match the task to the right sprint:

```bash
stint sprint list 2>&1
```

For any sprint that looks relevant, check its goal:

```bash
stint sprint show <id> 2>&1
```

Rules:
- Place the task in the earliest sprint whose goal it serves.
- If no sprint goal fits, set `status: backlog` and omit the sprint field — do not invent a sprint.
- Never place a v1 task in a sprint marked v2 or later.
- Infrastructure and tooling tasks default to `s14` (v1 release readiness) unless they clearly belong elsewhere.

---

## Step 3 — Metadata Assembly

Collect each field. Do not guess — confirm with the user if ambiguous.

### Area

Pull vocabulary from existing tasks:

```bash
grep -h "^  - " .stint/tasks/*.md | sort -u 2>/dev/null | head -40
```

Require at least one area. Use the same strings already in use — do not invent new namespaces without checking first.

Common areas (non-exhaustive):
- `host/pane-ops`, `host/config`, `host/terminal`, `host/permissions`, `host/notifications`, `host/secrets`
- `ui/chrome`, `ui/overlays`, `ui/widgets`, `ui/tile-tree`, `ui/sidebar`
- `sdk/pgap`, `sdk/python`
- `cli/commands`, `cli/completions`
- `apps/file-browser`, `apps/github-issues`, `apps/examples`
- `infra/build`, `infra/docs`, `infra/agents`, `infra/testing`, `infra/skills`

### Tags

Require at minimum one of `v1` or `v2`. Add domain tags as appropriate (`ui`, `tooling`, `testing`, `sdk`, etc.).

### Estimate

**Calibration rule — read this before writing any estimate:**

Coding agents execute at roughly 10x human coding speed. A task that would take a human developer 8 hours typically takes an agent under 1 hour. Always divide the naive human-derived estimate by ~10 before writing. If the user or context gives a human-framed estimate ("a day of work"), convert it: 1 day → `1h`, half a day → `30m`, a week → `4h`.

Use duration strings: `30m`, `1h`, `2h`, `4h`, `1d` (= 8h). Disallow bare integers.

If genuinely uncertain, bias toward shorter — overestimates skew bottleneck analysis.

### Blocker Check

Ask: does this task require another task or issue to be done first?

If yes, use the unified `blocked_by` field with correct syntax:

| Syntax | Meaning |
|---|---|
| bare integer | local stint task (e.g. `153`) |
| `@N` | local GitHub issue |
| `owner/repo@N` | external GitHub issue |
| quoted string | free-text note |

Accepts a single value or a YAML list.

If no real artifact dependency exists, leave `blocked_by: []`. Do not use blockers to express phase preference — use `status: backlog` and sprint ordering instead.

### gh_issue

If a GitHub issue already exists for this work, record its number. If one should be created, note it but do not create it here — that's `create-issue`'s job.

---

## Step 4 — Next Available ID

Find the highest existing task ID:

```bash
ls .stint/tasks/ | grep -oE '^[0-9]+' | sort -n | tail -1
```

Increment by 1. Zero-pad to 4 digits (e.g. `0192` → `0193`). Confirm no collision:

```bash
ls .stint/tasks/ | grep "^<NEXT_ID>"
```

If a collision exists, increment again.

---

## Step 5 — Write the Task File

File path: `.stint/tasks/<NNNN>-<kebab-slug>.md`

The slug comes from the title — lowercase, spaces to hyphens, strip punctuation, max ~6 words.

Template:

```markdown
---
id: "<NNNN>"
title: "<title>"
status: todo
estimate: "<Xh>"
sprint: "<sN>"
blocked_by: []
gh_issue: []
area:
  - "<area/one>"
tags:
  - "<v1|v2>"
---

<One paragraph — what this task is and why it exists.>

## Scope

- <Bullet: exactly what gets built or changed>
- <Bullet: ...>

## Non-Scope

- <Bullet: what explicitly is NOT in this task>

## Why

<One sentence on the motivation or user impact.>

## References

- `<path>` — <why relevant>
```

Omit sections that have nothing to say. Keep the body tight — the implementing agent reads it cold.

---

## Step 6 — Validate

```bash
stint check 2>&1
```

If it errors, fix the frontmatter before returning. Common errors: duplicate ID, invalid area format, missing required field.

---

## Step 7 — Return and Recommend

Return the task ID and file path. Then end with exactly one `RECOMMENDATION:` block:

```
RECOMMENDATION:
1. <one call — either "dispatch this now via /implement-stint <NNNN>" or "park it — task is ready when the sprint opens">
```

Do not offer both options. Pick one.
