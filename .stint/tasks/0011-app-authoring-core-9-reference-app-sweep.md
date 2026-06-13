---
id: "0011"
title: "App authoring: Core 9 reference app sweep"
status: done
estimate: "16h"
completed_at: "2026-06-13T08:34:04Z"
sprint: "s2"
blocked_by:
  - 9
  - 10
gh_issue: []
area:
  - "apps/examples"
  - "sdk/python"
  - "host/permissions"
tags:
  - "app-authoring"
  - "core-9"
  - "references"
---



Make Core 9 apps clean references for common app patterns: list, form, table, network fetch, AI/chat, state persistence, and canvas fallback.

## Why

The fastest path for agents to generate good marketplace apps is copying first-party patterns that already look and behave correctly. Each Core 9 app should be marketplace-publishable as-is.

## Scope

- Audit each Core 9 app against the SDK dev standard from 0009 and component defaults from 0010.
- Each app must: render cleanly at minimum pane size, use `FooterKeys` correctly, have a valid manifest with marketplace-compatible fields, and demonstrate one primary SDK pattern.
- Pattern coverage: list (file-explorer or similar), form, table, network fetch (with capability declaration), AI/chat (ai.query), state persistence (save/load), canvas fallback.
- Add `[app.marketplace]` section (commented-out) to each Core 9 manifest so marketplace publish is one uncomment away.

## Gotchas

- Core 9 only. Do not spend this sprint maintaining unrelated `apps/dev/` examples.
- Keep permission-gated flows explicit in examples that touch network or secrets.
- 0166 shipped Canvas leaf and UI gallery. Use the gallery app as a visual reference, not a Core 9 member.

## References

- `docs/prm/app-framework-marketplace.md`
- `src/app/marketplace.rs` (submission validation)

## Sweep outcome

The first-party app set is 13 (the "Core 9" is a brand name): balls, calc, chess,
csv_viewer, github-issues, kraken, logs, permissions, snake, stats, tetris, todo,
wikipedia. All were already in good shape:

- **FooterKeys** — used in all 13 (verified).
- **Marketplace fields** — all 13 already have `version` + `description`.
- **`[marketplace]` placeholder** — added the commented top-level `[marketplace]`
  block (publisher `"plexi"` since these are first-party) to all 13 manifests, so
  publish is one uncomment away. Verified each still parses and the section stays
  inert. NOTE: the validator reads a **top-level** `[marketplace]` section, not
  `[app.marketplace]` as the 0011 spec said — corrected to match 0008.

Pattern coverage (reference app per SDK pattern):
- list/table → csv_viewer (fs.read), github-issues, todo
- form/button → calc, todo
- canvas/game → snake, tetris, chess, balls; canvas viz → stats, kraken
- network fetch → wikipedia (net.http + allowed_hosts)
- state persistence → kraken, todo (self.state.save/load)
- timer/polling → logs
- permissions API → permissions
- **ai/chat → GAP.** No Core app calls `ai.query`; the reference lives in
  `apps/dev/assistant-pgap`. Not adding a new Core app here (Core set is fixed).

github-issues uses `subprocess(gh)` directly (declares no capability) — left as-is;
re-architecting it behind a capability is out of scope for this sweep.

## Variance

Estimate 16h. The apps were already clean (FooterKeys everywhere, valid fields),
so the work was the mechanical `[marketplace]` placeholder across 13 manifests
plus the pattern-coverage audit — not a per-app rebuild.
