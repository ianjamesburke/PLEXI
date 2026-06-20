---
id: "0257"
title: "refactor: host state-machine extraction (focus/modal routing)"
status: backlog
priority: p2
estimate: "8h"
blocked_by: []
gh_issue: []
area:
  - "host/pane-ops"
tags:
  - "v2"
---

Extract one cohesive state machine from PlexiApp, starting with focus/modal routing. Today focus and modal handling are spread across PlexiApp event handling; consolidate them into a single, testable state machine that commands flow into and effects flow out of.
