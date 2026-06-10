---
id: "0085"
title: "v2 portals: pane and agent state visualization"
status: backlog
sprint: "s20"
estimate: 6h
blocked_by:
  - 81
gh_issue: ["1495"]
area: ["ui/overlays", "ui/tile-tree", "agents"]
tags: ["v2", "navigation", "portals", "agents", "status"]
---

Surface pane state and agent state in the existing Portal tile presentation instead of prioritizing live miniature rendering before v1.

Use the current SubContext portal visualization as the base. Preserve the text-tier/portal-card feel, then add state affordances that make nested work observable:

- Show child-context pane state summaries on Portal tiles: terminal/app mix, busy/idle/error signals, and pending pain/status state when available.
- Surface agent state from `PaneAgentState` / `GetAgentStates` for panes inside the child context, including working, blocked, idle, and agent label.
- Consider a distinct agent portal presentation when the child context is primarily agent-driven, but keep it as a Portal variant/presentation rather than a separate navigation model unless the implementation proves otherwise.
- Keep live miniatures explicitly deferred. The current portal look is acceptable; miniatures are not a pre-v1 priority.

Done when a parent context can tell, from the Portal tile alone, whether the child context contains active agent work, blocked agent work, or pain/error state without zooming into the child.
