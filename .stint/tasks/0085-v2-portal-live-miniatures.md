---
id: "0085"
title: "v2 portals: agent state visualization"
status: in-progress
estimate: "3h"
started_at: "2026-06-12T17:30:28Z"
sprint: "s20"
blocked_by:
  - 81
gh_issue:
  - "1495"
area:
  - "ui/overlays"
  - "ui/tile-tree"
  - "agents"
tags:
  - "v2"
  - "navigation"
  - "portals"
  - "agents"
---


Surface agent state in the existing Portal tile presentation instead of prioritizing live miniature rendering before v1.

Use the current SubContext portal visualization as the base. Preserve the text-tier/portal-card feel, then add agent affordances that make nested work observable:

- Surface agent state from `PaneAgentState` / pane metadata for panes inside the child context, including working, blocked, idle, and agent label.
- Consider a distinct agent portal presentation when the child context is primarily agent-driven, but keep it as a Portal variant/presentation rather than a separate navigation model unless the implementation proves otherwise.
- Keep live miniatures explicitly deferred. The current portal look is acceptable; miniatures are not a pre-v1 priority.
- Do not solve terminal activity/pane-status inference here; that is split into task 0148 because it uses different state plumbing and has a failed prior attempt in #1918.

Done when a parent context can tell, from the Portal tile alone, whether the child context contains active agent work or blocked agent work without zooming into the child.
