---
id: "0204"
title: "Assistant: open scratchpad context and live diff edits"
status: todo
estimate: "2h"
sprint: "s34"
blocked_by:
  - 0203
gh_issue: []
area:
  - "host/ai"
  - "apps/text-editor"
  - "host/events"
tags:
  - "v1"
  - "assistant"
  - "scratchpad"
---

Let the host Assistant see open scratchpad panes and apply reviewed diffs that hot-load into the visible text editor.

## Scope

- Expose open scratchpad/text-editor note metadata to Assistant context: pane id, path, title, dirty state, and current revision.
- Add read access for current open scratchpad contents through a host-mediated path.
- Add a propose/apply flow for text diffs with revision checks so stale edits are rejected or surfaced as conflicts.
- Apply accepted diffs to the live editor state and backing file so the pane updates immediately.
- Show a compact diff or changed-resource summary before a mutating apply.
- Add HostHarness/model tests for read, proposed diff, apply success, and revision conflict.

## Non-Scope

- Do not give third-party PGAP assistants ambient file-edit powers.
- Do not add durable Assistant memory in this task.

## References

- `docs/prm/assistant-host-app.md`
- `src/assistant/mod.rs`
- `src/app/text_editor_app.rs`
- `src/host/app_timeline.rs`
