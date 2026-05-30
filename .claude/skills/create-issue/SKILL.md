---
name: create-issue
description: Use this skill when the user wants to create a GitHub issue, file a bug, propose a feature, or log a task on a repo. Triggered by phrases like "create an issue", "file an issue", "open an issue", "log a ticket", or "add a GitHub issue".
risk: low
source: local
date_added: "2026-03-24"
---

# Create Issue Skill

This is an **issue creation flow only**. Do not implement anything. The goal is a single, well-researched GitHub issue that can be handed off or picked up later.

---

## Step 0 — North Star + Foot Gun Gate (enhancements and features only)

For any `enhancement` or `idea` issue, invoke the `foot-guns` skill before proceeding. It will:

1. Read `NORTH_STAR.md` and verify the proposal maps to a real phase of Plexi's vision.
2. Check it against the "What Does Not Belong Here" exclusions.
3. Audit for architectural foot guns — reinvented wheels, protocol violations, divergent state stores.

**If foot-guns returns REJECTED** → do not create the issue. Surface the rejection reason to the user and stop.

**If foot-guns returns unresolved P0/P1 foot guns** → do not create the issue until those are resolved or the approach is changed.

**If the proposal passes** → continue. Carry any P2–P4 foot gun findings into the issue's Action Plan as explicit constraints.

Skip this step for `bug` issues where the fix is clearly scoped and doesn't require new architecture.

---

## Step 1 — Check for Existing Issues

Before creating anything, pull a broad list of issue titles — including recently closed ones — and scan for anything that looks related:

```
gh issue list --limit 200 --state all
```

Skim the titles. If anything looks like a near-duplicate (open **or** closed), fetch that issue's full body:

```
gh issue view <number>
```

**If a near-duplicate exists**, do not ask the user to decide — act on it:

1. Compare the existing issue body against the user's new description.
2. Determine the relationship:
   - **Superset** — the new description covers everything in the old issue *plus* significantly more. Create the new issue, then close the old one with `gh issue close <number> --comment "Superseded by #<new>."`. Reference the old issue in the new body so the history is traceable.
   - **Additive** — the new description adds context, steps, or scope to an existing issue without replacing it. Extract what's new and ask: "Want me to add these to issue #N instead of creating a new one?" If yes, `gh issue edit <number> --body "<updated body>"` and return the URL. Stop.
   - **Duplicate** — the new description is essentially the same issue. Surface it: "This looks like #N — want me to add anything to it, or close it?" Don't create a new issue.
3. Only proceed to Step 2 when it's clear a new issue is warranted (superset case, or user said no to folding).

---

## Step 2 — Research the Codebase

Before drafting anything, investigate the relevant code:

- Find the files, functions, or modules most related to the issue (Grep, Glob)
- Identify every file that will need to change and exactly what needs to change in each
- Note any risks, gotchas, or non-obvious constraints

**Persist the file map.** You are already locating every affected file — write it down instead of discarding it. The result goes into the issue body under `## Implementation Map` (feature/bug templates below) as `path:line — what changes`, plus a "Do not touch" list of adjacent-but-out-of-scope files. This is the single biggest lever on dispatch cost: the implementing agent reads only these files instead of re-running the whole discovery sweep. Record line numbers where you saw them, but treat them as hints — the implementing agent re-verifies each path before trusting it, so an approximate line is fine and a moved file is caught downstream.

**Grep GOTCHAS.md and DEV_LOG.md for related pitfalls (if they exist in the repo):**

Extract 3–5 key terms from the proposed change (the subsystem, method, or pattern being touched). Run:
```bash
grep -in "<term1>\|<term2>\|<term3>" GOTCHAS.md DEV_LOG.md 2>/dev/null
```
Any matches go directly into the issue body under a `## Known Gotchas` section — so the implementing agent sees them at the top of Phase 3 without having to re-discover them. If nothing matches, omit the section.

**For bug issues — root cause verification (mandatory):** Every claim in "Root Cause" and "Action Plan" must be confirmed by reading the actual code before it goes into the issue body. Grep for each named function, read the relevant lines, and confirm the described mechanism (ordering, data flow, call site) is present. If a claim can't be verified, mark it explicitly as an unconfirmed hypothesis in `## Open Questions` — never state an unverified root cause as fact in the Action Plan. An issue with a wrong root cause is worse than an issue with no root cause, because it misdirects the implementing agent.

This research feeds directly into the issue body — it is not a separate deliverable.

---

## Step 2b — Research External Dependencies (if applicable)

If the issue involves external APIs, libraries, or third-party services, resolve all unknowns now using web research (WebFetch, WebSearch — do NOT use Playwright for this). The goal is zero open questions in the final issue.

- Confirm exact API endpoint paths, parameter names, and supported values
- Check whether a feature/capability actually exists before listing it in the action plan
- Look up pricing, rate limits, or constraints that affect implementation decisions
- Use official docs or llms.txt schema endpoints where available

Do not leave anything as "(unconfirmed)" if it can be resolved with a quick web lookup.

---

## Step 3 — Assign Labels

Every PLEXI issue gets exactly **one type**, **one priority**, **one or more areas**, **one load**, and **one triage state** (`ready`, `untriaged`, or `blocked`). Version label is optional. Size label is set when the change is clearly small.

**Type** (pick one):
- `bug` — something is broken
- `enhancement` — new feature or improvement
- `idea` — speculative, exploratory, or not yet verifiable as a unit of work

**Priority** (pick one):
- `P0` — on fire, fix before anything else
- `P1` — shipping blocker or severe user-facing bug
- `P2` — important, not blocking a release
- `P3` — nice to have, polish, ergonomics
- `P4` — backlog, someday

**Area** (one or more — every area the change substantively touches; don't tag areas that only need a trivial import or re-export):
- `area:host/config` — config.toml parsing, hot-reload, validation
- `area:host/navigation` — pane/window focus, directional nav, history
- `area:host/notifications` — notify system, persistence, routing
- `area:host/pane-ops` — split, close, swap, promote, pane lifecycle
- `area:host/permissions` — capability grants, consent prompts
- `area:host/secrets` — secrets index, injection, SecretsApp
- `area:host/terminal` — PTY, scrollback, OSC sequences, terminal bindings
- `area:ui/chrome` — title bar, status bar, window chrome
- `area:ui/overlays` — modals, quick-note, scrims, overlay lifecycle
- `area:ui/sidebar` — context sidebar, context descriptions
- `area:ui/tile-tree` — tiling layout, pane tree rendering, resize
- `area:ui/widgets` — shared egui widgets, key chips, styled inputs
- `area:sdk/pgap` — Python SDK, PGAP protocol, DrawCommands, frame loop
- `area:sdk/python` — Python SDK packaging, API surface, type stubs
- `area:cli/commands` — CLI subcommands, argument parsing
- `area:cli/completions` — shell completion scripts
- `area:apps/file-browser` — file browser app
- `area:apps/github-issues` — github-issues app
- `area:apps/examples` — example/POC apps under examples/
- `area:apps/calc` — calc app
- `area:apps/descriptor-renderer` — descriptor renderer app
- `area:apps/mcp-renderer` — MCP renderer app
- `area:infra/build` — Justfile, install scripts, release pipeline
- `area:infra/docs` — docs, README, CLAUDE.md, website
- `area:infra/skills` — Claude skills, ship-issue workflow, dispatch

If no existing area fits, create a new `area:*` label before filing the issue:
```bash
gh label create "area:<namespace>/<module>" --color "#0075ca"
```

**Load** (pick one — mandatory, never omit):
- `load:S` — a few hours, contained to 1–3 files
- `load:M` — a day or two, touches multiple files or systems
- `load:L` — multi-day, architectural or cross-cutting

**Size** (set when the change is clearly small — omit if uncertain):
- `bundle` — touches ≤ 2 files, estimated diff ≤ ~50 lines, verifiable by diff alone with no install-and-test step required. These get batched into a single maintenance PR with other bundle issues rather than shipped individually.

To determine if `bundle` applies: look at the Action Plan from Step 2. If it names ≤ 2 files and the change is a constant swap, a log level, a label fix, or similarly mechanical — mark it `bundle`. If the change requires building and running the app to verify, do not mark it `bundle`.

**Version era** (pick one — only set when the user specifies; omit entirely if they don't):
Run `gh label list --json name --jq '[.[] | select(.name | startswith("v") or . == "future") | .name] | sort'` to get the current era labels. Pick from that list — do not assume which labels exist.

**Triage state** (pick one — mandatory):
- `ready` — fully triaged: has type, priority, area, load, and no blocking dependencies. Immediately actionable.
- `untriaged` — issue created but not yet fully labeled or assessed. Apply when any required label (priority, area, load) is uncertain and needs a triage pass.
- `blocked` — has open blocking dependencies — always pair with native GitHub blocking relationships (see Step 4b)

**Status** (apply as needed):
- `in progress` — actively being worked on (set by the ship skill, not here)
- `testing` — merged to alpha, awaiting user verification
- `needs-info` — awaiting clarification before work can proceed

---

## Step 3b — Verifiability Check

**Can this issue be verified end-to-end as a unit of completed work?**

A verifiable issue has a concrete, observable pass condition:
- "The user sees X in the UI after doing Y"
- "The log line `<text>` appears when `<action>` happens"
- "Cargo test `<name>` passes"

**If not verifiable** (exploratory, speculative, design-only): label it `idea`. Ideas do not get `ready` until they have a concrete, testable scope. The pass condition becomes the `## Done When` section.

---

## Step 4 — Draft and Create the Issue

**Title**: short, specific, actionable. Implies a verifiable outcome.

Three templates live at `.github/ISSUE_TEMPLATE/`. Pick the structure that matches the type:

Every issue body starts with a `## Summary` line — one plain-English sentence, no jargon, no filenames. This is the only section that gets read in dispatch recommendations and status updates. It must make sense to someone who has never opened the issue.

**feature / enhancement:**
```
## Summary
[One sentence — what this does, in plain English. e.g. "Add two light-mode color themes users can uncomment in config.toml."]

## What
[One short paragraph]

## Why
[User impact or technical consequence]

## Action Plan
[Bullet list — name files, functions, components where known]

## Implementation Map
[From Step 2 research. One line per file: `path:line — exactly what changes`.
 Then a "Do not touch:" line naming adjacent-but-out-of-scope files.
 The implementing agent reads ONLY these files (after a cheap existence check)
 instead of re-discovering them. Omit only if the change is a single obvious file.]

## Done When
[Concrete, observable pass condition — specific enough to appear in a [TESTING] block]

## Open Questions
[Only if genuine blockers exist before work can start — omit section entirely if none]
```

**bug:**
```
## Summary
[One sentence — what's broken and the symptom. e.g. "Audio recorder hangs on save because the pipe drain fails with EBADF."]

## What
[What is broken]

## Reproduce
[Exact steps]

## Expected
[What should happen instead]

## Action Plan
[What needs to change and where]

## Implementation Map
[From Step 2 research. One line per file: `path:line — exactly what changes`.
 Then a "Do not touch:" line naming adjacent-but-out-of-scope files.
 The implementing agent reads ONLY these files (after a cheap existence check)
 instead of re-discovering them. Omit only if the fix is a single obvious file.]

## Done When
[Observable pass condition]

## Open Questions
[Omit if none]
```

**idea:**
```
## Summary
[One sentence — what the idea is.]

## What
[What the idea is]

## Why
[Why it might be worth pursuing]

## Open Questions
[What needs to be resolved before this becomes a real issue]
```

Create the issue:
```bash
gh issue create --title "<title>" --body "<body>" \
  --label "<type>" --label "<priority>" --label "<area:namespace/module>" [--label "<area:...>"] \
  --label "<load:S|M|L>" \
  [--label "bundle"] [--label "<version-if-specified>"] \
  --label "<ready|untriaged|blocked>"
```

Return the issue URL.

---

## Step 4b — Wire Blocking Relationships (if applicable)

If this issue cannot start until other issues are closed, set GitHub's native blocking relationships using the `gh-issue-ext` extension (already installed):

```bash
# Mark <new-issue> as blocked by each dependency
gh issue-ext blocking add <new-issue> <blocker-1>
gh issue-ext blocking add <new-issue> <blocker-2>
```

Also apply the `blocked` label. Do **not** add any `depends_on` front matter to the issue body — the native relationship is the source of truth.

If `gh-issue-ext` is not installed: `gh extension install jwilger/gh-issue-ext`

That's the end of this flow — **do not implement**.

---

## Step 5 — Recommendation + Hand-Off Offer

After returning the issue URL, always end with exactly one `RECOMMENDATION:` block.

**Check alignment with North Star first.** Read `NORTH_STAR.md` and identify the next concrete phase or milestone. Ask: is this issue on the critical path to that next step?

- **Yes, aligned with the next North Star step** → recommend starting work on it.
- **No, or uncertain** → do not recommend starting work, regardless of priority. The single recommendation is to close the pane. The issue is labeled `ready` and will be picked up when priorities trickle down.

The priority label does not override this. A P0 that is not aligned with the next North Star step still gets "park it and close the pane."

Format:
```
RECOMMENDATION:
1. <one call — either "start work on this now (implement-issue #N)" or "park it — issue is ready, close the pane">
```

**If the recommendation is to start work now**, append a hand-off offer on the next line:

```
Hand off? Say "hand it off" and I'll split a new pane for implement-issue #N and close this one.
```

If the user responds with any affirmation ("hand it off", "yes", "do it", "go"), immediately invoke the `hand-off` skill with the issue number as the argument — do not ask what to run.

Do not offer hand-off when the recommendation is to park the issue.

---

## Notes

- If `gh` is not authenticated or there's no git remote, surface that early and stop.
- Never infer version labels from priority — only set them when the user specifies or context makes it unambiguous.
- `ready` is a signal to agents and the ship skill — only apply it when the issue is genuinely actionable right now.
