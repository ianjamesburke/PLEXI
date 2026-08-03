---
name: create-stint
description: Use this skill when the user wants to add a stint task, create a ticket, track a unit of work, file a bug, propose a feature, or log something to .stint/. Triggered by phrases like "add a task", "create a stint", "new task", "track this", "file a bug", "log a ticket", or when planning work that should go in the operating graph.
risk: low
source: local
date_added: "2026-06-19"
---

# Create Stint

Task creation flow only. Do not implement anything. The goal is a single, well-scoped stint task in `.stint/tasks/` with correct metadata, optionally linked to a GitHub issue.

Stint is the operating graph. Priority, size, blocking, and area tags. A task without correct metadata derails `stint next` and bottleneck analysis.

The stint task body owns all implementation detail, acceptance criteria, priority, and dependency order. `.stint/` is the only live work graph; do not maintain a separate priority projection.

---

## Step 0 -- Clarification Gate

Never create a stint from an idea that still needs fleshing out. When a request arrives as a batch or brain-dump, process candidates **one at a time**: for each, either the scope is already crisp (proceed), or ask the single most important clarifying question and wait for the answer before drafting. One question at a time, never a barrage. A vague candidate that can't be clarified right now gets surfaced back to the user as "needs definition" — not silently created.

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

## Step 2 -- Research and Confirm Plan

Before drafting, investigate the relevant code:

- Find the files, functions, or modules most related to the task
- Identify every file that will need to change and what changes in each
- Note risks, gotchas, or non-obvious constraints

For bug tasks, verify the root cause by reading code. Never state an unverified root cause as fact. If a claim can't be verified, mark it as an unconfirmed hypothesis in `## Open Questions`.

Grep `GOTCHAS.md` for related pitfalls if it exists.

**When a human is in the loop, present the decided implementation plan and confirm before writing the task file.** One approach only — no options, no A/B. If multiple approaches exist, pick the best one yourself and state it. Only ask the user if there is a genuine trade-off they must own (e.g., breaking API change, scope that affects other teams). Wait for confirmation before proceeding to Step 3.

**Autonomous mode — no human to confirm with.** When this skill is invoked by an agent mid-run (a babysitter worker filing a follow-up, a tester recording a discovered gap, any unattended flow), there is nobody to answer and waiting deadlocks the caller. In that case: skip the confirmation gate, pick the approach yourself, and write the chosen approach into the task body's `## Scope` as the decision of record. If a genuine trade-off exists that a human should own, still write the task — put the open decision under `## Open Questions` and set priority so it surfaces, rather than blocking on an answer that will never come. Never stall an unattended run waiting for confirmation.

---

## Step 3 -- No Sprints

**This repo runs a flat task list. There are no sprints.** The sprint index files were deleted on 2026-08-02 and the `sprint` frontmatter field stripped from every task, so there is nothing to place a task into and nothing to ask the user about. Do not run `stint sprint ...`, do not add a `sprint` field, and do not ask which sprint a task belongs to.

Ordering comes from `priority` plus `blocked_by`. `stint next` reads those directly and surfaces every unblocked `todo` task, so a well-prioritized task is scheduled by virtue of existing. That is the whole mechanism now — get the priority right and the task lands where it should.

Two facts worth keeping, in case sprints ever come back:

- Membership was never the task frontmatter. stint's parser accepts a `sprint` field only to tolerate old files and then discards it (`/// Silently ignored if present in old task files` — `src/parse.rs`), and nothing serializes it back. A task claiming `s2` in frontmatter was simply unsprinted, silently, and `stint check` never flagged it.
- The pre-flattening membership snapshot is archived at `.stint/archive/RESTORE-MANIFEST.md` alongside the original sprint files, recording every sprint's goal and ordered task list. Note `.stint/` is git-ignored, so this is local-only.

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

There is no `stint reserve`. `stint add` creates the file and assigns the ID in one call -- write the body to a temp file first, then pass it via `--body-file`:

```bash
cat > /tmp/stint-body.md <<'EOF'
<One paragraph -- what this task is and why it exists.>

## Scope

- <Bullet: exactly what gets built or changed>

## Non-Scope

- <Bullet: what explicitly is NOT in this task>

## Why

<One sentence on the motivation or user impact.>

## References

- `<path>` -- <why relevant>
EOF

stint add "<title>" \
  --priority <p0-p4> \
  --size <s|m|l> \
  --area "<area/one>" \
  --tags "<v1|v2>" \
  --blocked-by "<ref>" \
  --gh-issue "<N>" \
  --body-file /tmp/stint-body.md \
  --no-edit
```

Omit `--blocked-by` / `--gh-issue` entirely when empty -- do not pass an empty string. Repeat `--area`/`--tags` flags for multiple values, or comma-separate them.

`--no-edit` is required in agent context -- without it `stint add` opens `$EDITOR` and blocks. On success the command prints the created file's path (e.g. `.stint/tasks/0727-temp-syntax-check-task.md`); the ID is the leading number in that filename. Capture it from the printed path -- never derive an ID by listing task files beforehand.

Omit body sections that have nothing to say.

---

## Step 6 -- GitHub Issue (only when explicitly requested)

Do not create a GitHub issue unless the user explicitly asks for one. Stint tasks stand alone. If the user does request an issue, link it in the task's `gh_issue` field.

---

## Step 7 -- Validate

Run `stint check` once at the end of the session, not after every individual task. If the user asks for multiple tasks in one request, write them all first, then validate once.

```bash
stint check 2>&1
```

Then confirm each new task is visible to stint:

```bash
stint list 2>&1 | grep "<NNNN>"
```

The SPRINT column is always `-`; that is correct and expected on a flat list.
Read the row to confirm priority, size, and status came through as intended —
`stint check` validates structure, not whether the metadata says what you meant.

If a task is missing from `stint list` entirely, the file landed in the wrong place. Find it with `find . -name "<NNNN>-*.md"`, move it to `.stint/tasks/`, and re-run validation.

Fix any errors before returning.

---

## Step 8 -- Return and Recommend

Return the task ID, file path, and GitHub issue URL if created.

```
RECOMMENDATION:
1. <one call -- either "dispatch via /implement-stint <NNNN>" or "park it -- task is filed and will surface in stint next by priority">
```
