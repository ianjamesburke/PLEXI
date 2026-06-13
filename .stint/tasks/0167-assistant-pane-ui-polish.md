---
id: "0167"
title: "Assistant native chat UI overhaul — visual parity, rename, multiline, picker, mocked broker tests"
status: done
estimate: "8h"
actual: "460m"
started_at: "2026-06-12T21:47:58Z"
completed_at: "2026-06-13T05:27:46Z"
sprint: "s12"
blocked_by: []
gh_issue:
  - "2216"
area:
  - "ui/widgets"
  - "host/ai"
tags: []
---



## What

Overhaul the host Assistant pane into a native-feeling chat surface: visual
parity with the text-editor pane chrome, Cmd+R session/pane rename, growing
multiline composer (Enter sends, Shift+Enter newline), slash-command picker
rendered above the composer bounded to the pane rect, thinking/streaming
indicator, styled stub-command rows, subscription chips, and a MockBroker
test double driving an aggressive HostHarness/ui_tests suite.

Supersedes the narrower picker-clipping polish scope (#2201, closed).

## Why

The Assistant is the flagship surface of the agent platform; it must read as
a finished chat app, and the mocked broker makes every UI state testable
without a live model.

## References

- GitHub issue #2216 (supersedes #2201)
- src/assistant/render.rs (draw_picker, composer)
- src/assistant/model.rs
- src/app/text_editor_app.rs (rename + multiline patterns)
