---
name: stint
description: "Unified stint entry point. Routes by intent: task ID or 'next' → delegates to /implement-stint; description of new work → runs the create flow inline. Use for all stint interactions: creating tasks, checking status, or dispatching implementation."
risk: low
source: local
date_added: "2026-06-19"
---

# Stint Skill

Single entry point for all stint operations. Determine intent from the args, then route:

| Args | Route |
|---|---|
| A task ID (e.g. `0223`) or `next` | **→ invoke `/implement-stint`** with that arg |
| A description of work to track | **→ run the Create flow below** |
| No args | Run `stint next` and ask the user which task to dispatch |

---

## Create Flow

This is a **task creation flow only**. Do not implement anything. The goal is a single, well-scoped stint task in `.stint/tasks/` with correct metadata.

GitHub issues are implementation tickets. Stint is the operating graph — sprint order, estimates, timing, and blockers. A task without correct metadata derails future `stint next` output and bottleneck analysis.

### Step 1 — Duplicate Check

```bash
stint list 2>&1
```

Skim titles for near-matches. If one exists:
- **Same scope** → surface it to the user and stop. Add any missing context to the existing task body instead.
- **Overlapping but distinct** → note the relationship and proceed; set `blocked_by` or cross-reference in the body.

### Step 2 — Sprint Selection

```bash
stint sprint list 2>&1
```

For any sprint that looks relevant:

```bash
stint sprint show <id> 2>&1
```

Rules:
- Place the task in the earliest sprint whose goal it serves.
- If no sprint goal fits, set `status: backlog` and omit the sprint field — do not invent a sprint.
- Never place a v1 task in a sprint marked v2 or later.
- Infrastructure and tooling tasks default to `s14` (v1 release readiness) unless they clearly belong elsewhere.

### Step 3 — Metadata Assembly

Collect each field. Do not guess — confirm with the user if ambiguous.

#### Area

```bash
grep -h "^  - " .stint/tasks/*.md | sort -u 2>/dev/null | head -40
```

Require at least one area. Use existing strings — do not invent new namespaces without checking first.

Common areas (non-exhaustive):
- `host/pane-ops`, `host/config`, `host/terminal`, `host/permissions`, `host/notifications`, `host/secrets`
- `ui/chrome`, `ui/overlays`, `ui/widgets`, `ui/tile-tree`, `ui/sidebar`
- `sdk/pgap`, `sdk/python`
- `cli/commands`, `cli/completions`
- `apps/file-browser`, `apps/github-issues`, `apps/examples`
- `infra/build`, `infra/docs`, `infra/agents`, `infra/testing`, `infra/skills`

#### Priority

Set `priority` to one of: `p0`, `p1`, `p2`, `p3`, `p4`:

- `p0` — on fire / blocking release
- `p1` — shipping blocker
- `p2` — important, not blocking
- `p3` — polish
- `p4` — backlog

If the user specifies a priority, use it. If not, infer from context — a bug blocking users defaults to `p1`, a polish task to `p3`. Do not omit the field; stint uses it to sort `Ready` tasks within the same area.

#### Tags

Require at minimum one of `v1` or `v2`. Add domain tags as appropriate (`ui`, `tooling`, `testing`, `sdk`, etc.).

#### Estimate

Coding agents execute at roughly 10x human coding speed. Always divide the naive human-derived estimate by ~10 before writing. If the user gives a human-framed estimate: 1 day → `1h`, half a day → `30m`, a week → `4h`.

Use duration strings: `30m`, `1h`, `2h`, `4h`, `1d` (= 8h). Disallow bare integers.

If genuinely uncertain, bias toward shorter — overestimates skew bottleneck analysis.

#### Blocker Check

If the task requires another task or issue first, use `blocked_by`:

| Syntax | Meaning |
|---|---|
| bare integer | local stint task (e.g. `153`) |
| `@N` | local GitHub issue |
| `owner/repo@N` | external GitHub issue |
| quoted string | free-text note |

If no real artifact dependency exists, leave `blocked_by: []`. Do not use blockers to express phase preference.

#### gh_issue

If a GitHub issue already exists, record its number. If one should be created, note it but do not create it here — that's `/create-issue`'s job.

### Step 4 — Next Available ID

```bash
ls .stint/tasks/ | grep -oE '^[0-9]+' | sort -n | tail -1
```

Increment by 1, zero-pad to 4 digits. Confirm no collision:

```bash
ls .stint/tasks/ | grep "^<NEXT_ID>"
```

### Step 5 — Write the Task File

File path: `.stint/tasks/<NNNN>-<kebab-slug>.md`

Slug: lowercase, spaces to hyphens, strip punctuation, max ~6 words.

```markdown
---
id: "<NNNN>"
title: "<title>"
status: todo
priority: <p0|p1|p2|p3|p4>
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

## Non-Scope

- <Bullet: what explicitly is NOT in this task>

## Why

<One sentence on the motivation or user impact.>

## References

- `<path>` — <why relevant>
```

Omit sections that have nothing to say. Keep the body tight — the implementing agent reads it cold.

### Step 6 — Validate

```bash
stint check 2>&1
```

Fix any frontmatter errors before returning.

### Step 7 — Return and Recommend

Return the task ID and file path. End with exactly one `RECOMMENDATION:` block:

```
RECOMMENDATION:
1. <either "dispatch now via /implement-stint <NNNN>" or "park it — task is ready when the sprint opens">
```

Do not offer both options. Pick one.
