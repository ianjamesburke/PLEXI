# Plexi MVP Interaction Spec

## Scope

This document defines the control model and persistent chrome for the MVP terminal workspace.
It is intentionally narrow. It does not define future browser panes, Excalidraw panes, notifications, templates, or advanced tiling heuristics.

## Product Direction

Plexi is terminal-first.
The terminal is the primary focused surface and should feel trustworthy for normal shell work.
Workspace controls must not steal unmodified printable keys, shell control sequences, or bare arrow keys while a terminal owns focus.

The workspace model is inspired by scrollable tiling rather than split-tree layout:

- one active panel at a time
- panels positioned on a 2D coordinate plane
- directional focus based on spatial neighbors
- overview as an explicit zoomed-out workspace layer

## Input Ownership

### Terminal focus

When a terminal panel is focused:

- all printable keys go to the terminal
- bare arrow keys go to the terminal
- bare `Tab` and `Shift+Tab` go to the terminal
- shell control sequences such as `Ctrl+O` stay with the terminal
- app actions require modifiers, native menu selection, or pointer interaction

### Workspace actions

The MVP workspace actions are:

- `Cmd/Ctrl+N`: create terminal to the right
- `Cmd/Ctrl+Shift+N`: create terminal below
- `Cmd/Ctrl+W`: close active terminal
- `Cmd/Ctrl+B`: toggle sidebar
- `Cmd/Ctrl+/`: toggle keyboard help
- `Cmd/Ctrl+Shift+O`: toggle overview
- `Cmd/Ctrl+Arrow`: focus nearest panel in that direction

Overview-only actions:

- bare arrow keys pan the overview camera
- `Cmd/Ctrl+Shift+Arrow`: reposition the active panel
- zoom commands remain modifier-based

The MVP does not require shortcuts for context switching or panel cycling.
Those can remain menu or pointer actions until the control model is stable.

## Window Chrome

The native application menu remains the source of truth for File, Edit, View, Window, and Help behavior.
The webview should not duplicate document-style header chrome.

The persistent shell layout is:

- optional left sidebar
- thin workspace toolbar
- canvas or terminal surface
- thin status bar

There is no marketing header, hero copy, or large title block inside the focus view.

## Focus View

The focus view should feel like a real terminal app:

- the terminal surface dominates the window
- the toolbar is compact and factual
- panel title, cwd, mode, and coordinates are visible but subdued
- instructional copy is removed from the main canvas
- minimap is small and secondary

## Sidebar

The sidebar is a utility surface, not primary content.

- default visible in MVP
- toggled quickly with `Cmd/Ctrl+B`
- contains contexts and workspace actions
- can be hidden without losing access to core terminal behavior

## Overview

Overview is an explicit workspace layer for:

- seeing panel positions
- panning around the canvas
- selecting a panel
- repositioning a panel

Overview is not the default working mode.
Exiting overview returns the user to terminal focus.

## Native Menu Rules

- File and Edit roles should use native menu behavior where possible
- quit should remain native
- app shortcuts should prefer modified accelerators only
- no bare printable key accelerators should be reserved by the app

## Deferred

The following are intentionally deferred:

- advanced Niri-like half-pane and container heuristics
- browser pane input ownership rules
- Excalidraw pane input ownership rules
- command palette design
- templates and notifications
