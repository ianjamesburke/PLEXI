---
id: "0190"
title: "v2: Plexi-owned docking layout engine"
status: backlog
estimate: "80h"
sprint: "s33"
blocked_by: []
gh_issue: []
area:
  - "host/pane-ops"
  - "ui/chrome"
  - "infra/testing"
tags:
  - "v2"
  - "docking"
  - "layout"
  - "tabs"
  - "drag"
---

Replace the current `egui_tiles`-owned layout semantics with a Plexi-owned docking model that treats tabs, splits, pane moves, and future floating/cross-window docking as first-class commands.

## Scope

- Introduce a stable Plexi dock tree model: `Window -> DockTree -> Node`, with nodes for `Split`, `Tabs`, and `Pane`; evaluate whether `Floating` belongs in the first rewrite or a follow-up.
- Move layout mutations behind pure commands:
  - `move_pane`
  - `dock_pane`
  - `undock_pane`
  - `reorder_tab`
  - `split_node`
  - `merge_tabs`
  - `close_node`
  - `normalize_tree`
- Build one drag-session state machine:
  - `Idle`
  - `DraggingTab`
  - `DraggingPaneHeader`
  - `HoveringDropTarget`
  - `Commit`
  - `Cancel`
- Render drop previews for left/right/top/bottom/center-tab targets.
- Make layout persistence Plexi-owned and stable across crate upgrades.
- Preserve existing host behavior: focus history, zoom, portals, file drops, activity pips, pane names, close semantics, and keyboard split/tab/navigation commands.
- Add broad pure layout tests plus minimal UI smoke tests for hit zones and preview visibility.

## Downstream Block

Until this task is promoted and implemented, do not add broad docking features on top of `egui_tiles` glue. These are blocked by this rewrite:

- Dragging a tab/header out of a tab group to split or move a pane.
- Dragging pane headers between panes.
- Cross-window pane dragging.
- Floating/detached panes.
- Persistent named dock layouts.
- Edge/center drop preview systems beyond the shallow v1 tab reorder task.

Small tactical fixes to existing split/tab/close behavior remain allowed when they are bug fixes and do not expand the docking interaction surface.

## Why

The current system can support shallow tab reorder cheaply, but full drag-to-dock would accumulate one-off tree surgery across rendering and pane ops. A Plexi-owned dock tree gives agents and CLI commands a clear contract: layout changes are commands over stable nodes, and egui is just the renderer/input layer.

## References

- `src/spatial/tiling.rs`
- `src/pane_ops/layout.rs`
- `src/app/render.rs`
- `src/host/context.rs`
- `docs/TESTING.md`

