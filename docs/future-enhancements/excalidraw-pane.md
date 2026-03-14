# Excalidraw Pane Integration

## Purpose

This document describes a future enhancement for embedding an Excalidraw surface inside Plexi as a panel type.

This is a **future enhancement** document, not MVP scope by default.

## Summary

Embedding Excalidraw in Plexi is realistic with low-to-moderate implementation risk if introduced as an additional panel type.

Chosen default for v1 of this enhancement:

- Add `excalidraw` as a new panel type alongside `terminal`
- Keep terminal creation behavior as the default workflow
- Persist Excalidraw scene state in workspace data
- Reuse the existing panel positioning/focus model
- No multiplayer sync or cloud collaboration in v1

## Why This Direction

The MVP is explicitly terminal-first and excludes Excalidraw panes.
Replacing terminal creation with Excalidraw would conflict with MVP identity and core success criteria.

Adding Excalidraw as an optional pane type preserves product direction while enabling diagramming and whiteboarding in the same spatial workspace.

## Product Behavior

### Pane lifecycle

- User can create an Excalidraw panel from workspace commands.
- Excalidraw panel participates in the same 2D layout, focus, and movement model as terminal panels.
- Closing the panel removes it from layout and releases in-memory scene state.

### Focus and input ownership

- When an Excalidraw panel is active, pointer and keyboard input are owned by Excalidraw.
- Global workspace shortcuts continue to use explicit modifiers (`Cmd/Ctrl+...`) and must not break typical Excalidraw editing behavior.
- Directional focus and panel movement shortcuts remain consistent across pane types.

### Persistence

- Workspace save includes Excalidraw panel metadata and serialized scene data.
- Workspace restore recreates Excalidraw panels with prior scene content.
- v1 stores scene data locally only.

## Technical Direction

### Integration model

- Add `excalidraw` to panel type definitions and schema validation.
- Render panel surface conditionally by panel type:
  - `terminal` uses existing xterm runtime.
  - `excalidraw` mounts an Excalidraw instance in the panel surface.
- Keep PTY/session manager unchanged for non-terminal panels.

### State model

- Extend panel record with optional Excalidraw scene payload for `excalidraw` type.
- Keep terminal-specific fields (`cwd`, PTY session wiring) terminal-only in behavior.
- Ensure serialization/deserialization paths handle mixed panel types deterministically.

### Command surface

- Introduce an explicit workspace command for creating Excalidraw panels.
- Do not repurpose existing `new-terminal-*` commands in v1.
- Menu and keybinding labels should reflect pane type clearly.

## Risks and Mitigations

- Keyboard conflicts: isolate Excalidraw input ownership and keep workspace commands modifier-based.
- Persistence size growth: cap or compress serialized scenes if workspace files become large.
- Render/perf overhead: only mount active Excalidraw instance and avoid unnecessary rerenders.

## Acceptance Criteria

1. A user can create, focus, move, and close an Excalidraw panel.
2. Existing terminal creation and PTY behavior remains unchanged.
3. Excalidraw edits persist across save/restore.
4. Mixed workspaces (terminal + Excalidraw) restore reliably.
5. Keyboard navigation between panels remains consistent.

## Test Plan

### Unit and state tests

- Panel type parsing/validation accepts `terminal` and `excalidraw`.
- Workspace serialization round-trips Excalidraw panel state.
- Focus and directional navigation works in mixed pane workspaces.

### E2E tests

- Create Excalidraw panel and verify canvas mounts.
- Draw a simple shape, save workspace, reload, and verify restoration.
- Create terminal + Excalidraw panels and verify directional focus behavior.
- Confirm terminal interaction still works after Excalidraw interactions.

## Non-Goals (for this enhancement)

- No replacement of terminal-first workflow.
- No shared realtime collaboration.
- No remote scene syncing.
- No asset library or plugin system.
