# Plexi Future PRD

## Summary

This document captures the product vision beyond the first shippable Plexi MVP. These features are intentionally deferred so the MVP can stay focused on spatial local shell workflows.

## Future Product Direction

Plexi should evolve from a spatial terminal workspace into a broader project orchestration environment that can manage terminals, embedded tools, agent workflows, and project-specific layouts in one keyboard-first system.

## Future Themes

### 1. Multi-surface workspace

Support non-terminal panes that can live alongside shell sessions in the same spatial graph.

Candidate pane types:

- embedded browser or Chromium webview
- Excalidraw-style diagram or whiteboard surface
- markdown or notes pane
- project reference pane

### 2. Agent orchestration

Support workflows where many agent-driven terminal sessions run at once.

Candidate capabilities:

- attention badges for panes waiting on user input
- notification routing to the relevant context
- status summaries for many concurrent sessions
- fast jump actions for interrupted work

### 3. Rich workspace composition

Move beyond ad hoc splits into reusable workspace structures.

Candidate capabilities:

- templates for common project layouts
- one-command layout presets
- reusable project startup arrangements
- richer grouping semantics

### 4. Remote and hybrid environments

After the local session model is stable, support remote workflows.

Candidate capabilities:

- SSH-backed terminal sessions
- connection pooling
- richer remote host metadata
- mixed local and remote workspaces

### 5. Advanced rendering

Revisit the terminal renderer once the session architecture is proven.

Candidate capabilities:

- `libghostty` integration
- better rendering fidelity
- improved performance at larger session counts
- tighter Ghostty ecosystem compatibility

### 6. Appearance customization

After the core interaction model is stable, expose terminal appearance settings to the user.

Candidate capabilities:

- configurable terminal font family
- configurable font size and line height
- optional light theme alongside the default dark workspace chrome
- terminal appearance profiles that disable ligatures and other font features consistently

## Explicitly Deferred From MVP

- SSH
- notifications
- browser panes
- Excalidraw-style panes
- markdown panes
- templates
- advanced grouping
- remote orchestration
- `libghostty`
- terminal font configuration
- light theme and appearance presets

## Decision Log

- Chosen: keep future enhancements in a separate document.
  Reason: protects the MVP from scope creep while preserving product ambition.
- Chosen: prioritize agent orchestration and embedded tools after the local shell foundation.
  Reason: those features only make sense once core session management is stable.
