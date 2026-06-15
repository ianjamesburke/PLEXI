---
id: "0196"
title: "v2: host event log coverage audit"
status: backlog
estimate: "2h"
blocked_by: []
gh_issue: []
area:
  - "host/events"
  - "host/pane-ops"
  - "ui/overlays"
tags:
  - "v2"
  - "instrumentation"
---

Make `events.jsonl` close to one-to-one with user-visible host actions, so demos, agent traces, and debugging tools can rely on the host event stream instead of scraping `plexi.log`.

## Scope

- Audit host actions and add missing `HostEvent` emits for app open and close, note save, note open and close, palette open, new window, new context, and pane switches.
- Make built-in apps and process apps follow the same event contract where possible.
- Add focused regression coverage that proves each action writes the expected event.
- Document any intentionally unlogged action and why it is excluded.

## Non-Scope

- Do not build a new analytics UI.
- Do not change event retention, rotation, or storage format beyond adding event variants needed for coverage.

## Why

Plexi should leave a trustworthy action trail that matches what happened in the app.

## References

- `src/host/event_log.rs` - host event schema and JSONL writer.
- `src/pane_ops/layout.rs` - pane close, app close, and focus-changing layout paths.
- `src/pane_ops/create.rs` - app open, scratchpad, and built-in app launch paths.
- `src/overlays/notes_picker.rs` - notes picker and note-open flows.
- `src/app/mod.rs` - global shortcut dispatch for palette, notes, windows, contexts, and pane movement.
