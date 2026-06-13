---
id: "0011"
title: "App authoring: Core 9 reference app sweep"
status: done
estimate: "16h"
actual: "0m"
completed_at: "2026-06-13T16:45:46Z"
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
