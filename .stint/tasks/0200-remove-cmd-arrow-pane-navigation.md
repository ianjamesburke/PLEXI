---
id: "0200"
title: "Shortcuts: remove Cmd+Arrow pane navigation aliases"
status: done
estimate: "30m"
actual: "9m"
started_at: "2026-06-16T04:23:25Z"
completed_at: "2026-06-16T04:32:16Z"
blocked_by: []
gh_issue: []
area:
  - "host/navigation"
  - "host/config"
tags:
  - "v1"
  - "shortcuts"
---



Remove the host-level Cmd+Arrow aliases for pane navigation so text editors can own line-boundary and document-navigation chords.

## Scope

- Remove Cmd+Arrow entries that alias Cmd+H/J/K/L pane navigation.
- Keep Cmd+H/J/K/L as the canonical pane navigation shortcuts.
- Update shortcut help/docs/config descriptions so they no longer advertise Cmd+Arrow navigation.
- Add or update shortcut tests proving Cmd+Arrow does not produce host navigation.

## Non-Scope

- Do not remove Cmd+Shift+Arrow scroll bindings unless a test proves they conflict with the same editor behavior.

## References

- `src/host/keys.rs`
- `docs/CONFIG.md`
