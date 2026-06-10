---
id: "0047"
title: "v2 host architecture: focused state-machine extraction"
status: backlog
sprint: "s13"
estimate: 12h
blocked_by: []
blocked_by_gh: []
gh_issue: ["2096"]
area: ["ui/overlays", "host/pane-ops", "host/navigation"]
tags: ["v2", "architecture"]
---

Extract one cohesive state machine from `PlexiApp`, starting with focus/modal routing, after v1-visible work is stable.

## Why

This is worthwhile architecture work, but it should not interrupt v1 unless it becomes a blocker for the UI stabilization sprint.
