---
id: "0053"
title: "v2 performance: CLI renderer allocation cleanup"
status: backlog
sprint: "s15"
estimate: 6h
blocked_by: []
blocked_by_gh: []
gh_issue: ["2027"]
area: ["cli/commands"]
tags: ["v2", "performance", "cli-renderer"]
---

Render the native CLI descriptor UI from borrowed descriptor data instead of rebuilding cloned row, argument, and flag vectors every frame.

## Why

This is useful cleanup, but it is not a v1 blocker unless the CLI renderer becomes a visible release path.
