---
id: "0007"
title: "File Explorer: Plexi-native actions and agent selection state"
status: backlog
sprint: "s1"
estimate: 12h
blocked_by:
  - 4
gh_issue: ["2141"]
area: ["apps/file-browser", "host/pane-ops", "host/permissions"]
tags: ["file-explorer", "agents", "selection"]
---

Expose selected paths to linked terminals, host commands, Plexi apps, and agents through explicit host contracts.

## Why

File Explorer should become a Plexi working surface, not just a visual browser. Selection needs to be safe, scriptable, and visible to agents.

## Gotchas

- Do not bypass context or capability boundaries.
- Add HostHarness coverage if new host actions or capability checks are introduced.

## References

- GitHub issue #2141
- Blocks: #2141 is blocked by #2138 and #2139
- `docs/prm/file-explorer-overhaul.md`
