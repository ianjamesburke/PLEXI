---
id: "0148"
title: "v2 portals: terminal and pane activity state visualization"
status: backlog
sprint: "s20"
estimate: 6h
blocked_by:
  - 85
gh_issue: ["1918"]
area: ["ui/tile-tree", "host/terminal", "host/context"]
tags: ["v2", "navigation", "portals", "terminal", "status"]
---

Surface terminal/pane activity state in the existing Portal tile presentation, separately from agent hook state.

This is the non-agent status lane. It should answer whether a child context has running/busy/idle/error terminal or app work without zooming in. Agent hook state is handled by task 0085; do not conflate the two systems.

Scope:

- Define what terminal activity means for Portal tiles: recent output, running foreground process, server-like long-running process, exited/error state, and idle.
- Prefer host-owned terminal/process state over cosmetic animation. If a signal cannot be observed reliably, log the gap and leave it out.
- Fold the result into the existing `ContextState` / `PortalPreview` path so the portal card remains the same shape with better state.
- Preserve the current SubContext portal look; this is status surfacing, not live miniatures.

Prior attempt:

- Issue #1918 tried line-output activity dots in the portal minimap.
- It added `last_lines_written` and `last_activity` to terminal panes, compared `TerminalBackend::lines_written` every frame, and rendered animated dots with an activity decay.
- It failed because app/portal panes were initially hardcoded active, and the core terminal signal never proved reliable: the diagnostic line for `lines_written` deltas did not fire.
- Next attempt must instrument first. Add unconditional logs or a HostHarness-visible signal proving the snapshot loop sees the expected terminal panes and that the chosen activity source changes under a controlled command before wiring UI.

Done when Portal tiles can distinguish active/running terminal work, idle terminal work, and error/exited state using verified host-side state, with regression coverage for the state calculation.
