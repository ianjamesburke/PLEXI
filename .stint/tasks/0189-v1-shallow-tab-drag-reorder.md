---
id: "0189"
title: "v1: shallow tab drag reorder"
status: done
estimate: "4h"
actual: "90m"
started_at: "2026-06-15T07:57:56Z"
completed_at: "2026-06-15T17:44:37Z"
blocked_by: []
gh_issue: []
area:
  - "host/pane-ops"
  - "ui/chrome"
tags:
  - "v1"
  - "ui"
  - "tabs"
  - "drag"
---




Add shallow drag reordering inside an existing tab bar without changing the broader pane docking model.

## Scope

- Convert the tab bar hit handling around `paint_tab_bar()` into proper egui drag interactors.
- Support reordering tabs within the same `Container::Tabs`.
- Preserve the active pane, focused pane, zoom state, activity pips, and click-to-switch behavior.
- Add a pure pane-op for `reorder_tab(container_tile, from_idx, to_idx)` and cover it with `HostHarness` or focused unit tests.
- Log successful reorder commits at `info` level.

## Non-Scope

- Do not add dragging a tab/header out of a tab group.
- Do not add pane docking, edge drop previews, cross-window moves, floating panes, or persistent dock layouts.
- Do not rewrite the layout engine.

## Why

This gives users the immediate missing tab affordance while keeping the change reversible. The full pane docking rewrite is tracked separately as v2 work.

## References

- `src/spatial/tiling.rs`
- `src/render/terminal_pane.rs`
- `src/app/render.rs`
- `src/pane_ops/layout.rs`

## Variance

Kept the scope shallow: same-container reorder only, no drag preview or docking model changes. The pure pane operation and tests covered the risky state preservation path.
