---
id: "0154"
title: "Testing: /testing skill — agent self-validation artifact bridging implement and validate-pr"
status: in-progress
estimate: "12h"
started_at: "2026-06-11T10:06:01Z"
sprint: "s8"
blocked_by: []
gh_issue:
  - "2169"
area:
  - "infra/testing"
  - "infra/skills"
tags:
  - "testing"
  - "v1+"
  - "ship-pipeline"
---


Create the `/testing` skill that the implementing agent runs after writing code, before pushing. Produces a structured test-evidence artifact in the Ship Log that validate-pr reads to decide whether binary install can be skipped.

## Why

The ship cycle has no explicit quality handoff between implement and validate. This skill closes that gap: HostHarness results and optional kittest render screenshots are written as durable evidence into the issue Ship Log and PR comment, letting validate-pr skip binary install for fully-covered PRs.

## Scope

1. Create `.agents/skills/testing/SKILL.md` — the full skill flow:
   - Diff analysis → classify change (host logic / egui layer / both)
   - Host logic: `cargo test --bin plexi` scoped to touched modules
   - egui layer: write throwaway render script to `/tmp/plexi-test-<issue>.rs`, append temporarily to `src/ui_tests.rs`, run `cargo test render_validate_issue_<n>`, save PNG to `/tmp/plexi-render-<issue>.png`, revert `src/ui_tests.rs`
   - Guard: `git diff --cached` must show no staged test block before commit
   - Write `**Test evidence:**` block to issue Ship Log
   - Attach render PNG to PR comment (if produced) for session-durability

2. Update `.agents/skills/implement-issue/SKILL.md` Phase 3 — invoke `/testing` inline after `cargo build` passes, before `git push`

3. Update `.agents/skills/implement-stint/SKILL.md` Phase 4 — same

4. Update `.agents/skills/validate-pr/SKILL.md`:
   - Step 1: third install-gate outcome — evidence-present → skip install, diff-review only
   - Step 2b: skip `cargo test` re-run if evidence block already present
   - Step 3: surface test counts + render PNG link at top of testing block

## Ship Log artifact format
```
**Test evidence (attempt <N>):**
- cargo test: <N> passed, 0 failed — modules: <list>
- PlexiUiHarness render: /tmp/plexi-render-<issue>.png (attached to PR #<n> comment)
- Conclusion: binary install required | install skippable — full coverage
```

## Done When
- `/testing` invoked at end of implement-issue/implement-stint produces a `**Test evidence:**` block in the Ship Log
- A PR whose Ship Log contains `install skippable` passes through validate-pr without `just pr-install`
- Throwaway render test is never staged or committed (verified by `git diff --cached` guard)
