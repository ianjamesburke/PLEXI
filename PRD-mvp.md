# Plexi MVP PRD

## Summary

Plexi is a local-first desktop terminal workspace for solo developers who manage multiple local projects and tasks at once. The MVP is a keyboard-first, spatial alternative to `tmux`, terminal tabs, and pane grids. It should help a developer build a stable mental model of their active work by arranging terminal sessions in a persistent 2D layout and switching between them quickly.

The first milestone of this PRD is a real local shell backend. Each terminal panel must run an actual interactive shell session through a PTY so the user gets normal shell behavior, normal shell config loading, and real command execution.

## Problem

Current terminal workflows create cognitive overhead:

- `tmux` is powerful but intimidating and modal.
- Terminal tabs hide structure and make context switching expensive.
- Pane grids are useful, but they do not provide a durable spatial map of work.
- AI-heavy workflows multiply the number of active terminals, repos, and subprocesses a developer needs to coordinate.

The MVP should reduce this overhead with a simpler interaction model than `tmux` while preserving the power of multiple long-lived shell sessions.

## Primary User

Solo developer working locally across multiple repos and tasks, often with agent-driven CLI workflows, who wants a simpler replacement for `tmux` and terminal tabs.

## Core Goal

Reduce the cognitive load of multitasking across many terminal sessions by combining:

- persistent spatial layout
- full keyboard navigation
- customizable workspace structure
- real local shell sessions

## Non-Goals

These are explicitly out of scope for the MVP:

- SSH orchestration
- team collaboration
- cloud sync
- telemetry
- embedded browser panes
- Excalidraw-style panes
- markdown/document panes
- notifications and agent attention routing
- templates and project presets
- `libghostty` integration

## Product Principles

- Local-first by default
- Keyboard-first interaction model
- Spatial layout as a cognitive aid, not decoration
- Real shell behavior over simulated terminal behavior
- Maintainable architecture over feature sprawl
- Explicit scope boundaries

## MVP Scope

### Required capabilities

- Create and close terminal panels in a 2D workspace
- Navigate between panels entirely by keyboard
- Persist workspace layout and restore it on launch
- Run a real local shell in each terminal panel
- Support roughly 25-50 local sessions with smooth navigation
- Allow user-customizable layouts and keyboard-centric workflows

### First implementation milestone

Implement PTY-backed local shell sessions:

- one terminal panel maps to one PTY-backed shell session
- the shell launches as the user’s actual shell when possible
- shell input, output, resize, and exit are wired correctly
- shell config is read by the shell itself through normal startup behavior
- the browser-side terminal remains a renderer, not a fake shell

## Functional Requirements

### Workspace

- The workspace stores panel positions in a 2D coordinate system.
- The active context has one active panel at a time.
- New panels open relative to the active panel.
- The user can move focus directionally with the keyboard.
- The workspace can be saved and restored locally.

### Terminal sessions

- Each terminal panel owns an independent shell session.
- Sessions continue to exist while another panel is focused.
- Input sent to a panel reaches that panel’s PTY session.
- Output from a PTY session is buffered and rendered when that panel is focused.
- Closing a panel closes its session cleanly.

### Configuration

- Settings and workspace state are stored locally on disk.
- The MVP should leave room for keyboard customization and terminal appearance settings.
- The system should be able to remember shell path, initial working directory, and workspace metadata.

## Non-Functional Requirements

### Performance

- Focus changes should feel immediate.
- Creating, switching, and closing panels should remain responsive at 25-50 sessions.
- Terminal output buffering should not block the UI thread.

### Reliability

- Session lifecycle should be explicit and testable.
- Closing a panel must not leak shell processes.
- Restore behavior should be deterministic.

### Privacy

- All state remains local by default.
- No cloud dependency is required for MVP operation.
- No telemetry is required.

### Maintainability

- PTY logic lives in the Bun process behind a narrow interface.
- The browser view should depend on a session bridge, not on process management details.
- Testable backend boundaries take priority over cleverness.

## Technical Direction

### Recommended architecture

- `xterm.js` remains the terminal renderer for the MVP.
- The Bun process owns shell session lifecycle.
- Shell sessions use PTY-backed subprocesses.
- The webview communicates with the Bun process through a dedicated RPC or session bridge.
- The browser-only verification loop may use a mock session bridge for headless UI testing, but production behavior must use real PTY sessions.

### Why not `libghostty` now

`libghostty` is a renderer/emulator decision, not the core session architecture. The MVP risk is session lifecycle, shell integration, persistence, and navigation. Those problems must be solved first. A renderer swap is easier after the PTY-backed session model is stable.

## Milestones

### Milestone 1

Real local shell sessions via PTY.

### Milestone 2

Reliable workspace persistence and restore for local sessions.

### Milestone 3

Stronger customization around layout behavior and keyboard workflow.

## Success Criteria

- A user can replace a basic local `tmux` workflow with Plexi for daily multitasking.
- Real shell sessions behave like real terminals instead of a demo shell.
- Spatial navigation reduces context-switching overhead.
- The codebase has clean separation between UI, workspace state, and session management.

## Decision Log

- Chosen: split the project into an MVP PRD and a future PRD.
  Reason: the first shippable product needs a hard boundary.
- Chosen: solo local developer as the MVP user.
  Reason: it matches the clearest immediate use case.
- Chosen: local-only by default.
  Reason: reduces complexity and aligns with user value.
- Chosen: target 25-50 sessions for the MVP.
  Reason: meaningful scale without overcommitting.
- Chosen: PTY-backed real shell sessions as the first milestone.
  Reason: the current terminal experience is still simulated.
- Chosen: defer SSH and `libghostty`.
  Reason: they add complexity before the core session model is stable.
