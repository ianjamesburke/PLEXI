---
id: "0080"
title: "v2 renderer cleanup: shared text render helper"
status: backlog
sprint: "s16"
estimate: 6h
blocked_by:
  - 30
  - 31
gh_issue: ["1146"]
area: ["ui/widgets", "sdk/pgap"]
tags: ["v2", "renderer", "text"]
---

Centralize text measurement and painting for PGAP text, text rows, and layout leaves so font selection, elision, anchors, selectable labels, and bold behavior stay consistent.
