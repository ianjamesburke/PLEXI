---
id: "0116"
title: "v2 SDK: responsive breakpoint system"
status: backlog
sprint: "s26"
estimate: 6h
blocked_by:
  - 30
  - 31
gh_issue: ["1336"]
area: ["sdk/python", "sdk/pgap", "apps/github-issues"]
tags: ["v2", "sdk", "responsive"]
---

Let apps declare size tiers and render alternate layouts for narrow, medium, and wide panes instead of forcing one layout into every pane size.

## v1 Decision

Not a v1 blocker. The v1 app-authoring sprint already has small-pane fit work; formal breakpoint APIs are the broader v2 answer after the first authoring path is stable.
