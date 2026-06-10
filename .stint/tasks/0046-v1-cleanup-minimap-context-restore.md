---
id: "0046"
title: "v1 cleanup: minimap context restore"
status: backlog
sprint: "s11"
estimate: 3h
blocked_by: []
blocked_by_gh: []
gh_issue: ["2052"]
area: ["host/navigation", "host/context"]
tags: ["v1", "cleanup", "minimap"]
---

Restore each context's saved minimap visibility when focus-history traversal crosses context boundaries.

## Why

Context navigation should restore the destination context's UI state consistently, regardless of which navigation path changed focus.
