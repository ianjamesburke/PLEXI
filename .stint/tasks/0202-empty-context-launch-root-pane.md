---
id: "0202"
title: "Empty contexts: explicit launches create a root pane"
status: done
estimate: "1h"
actual: "11m"
started_at: "2026-06-16T04:26:14Z"
completed_at: "2026-06-16T04:36:31Z"
blocked_by: []
gh_issue: []
area:
  - "host/pane-ops"
  - "apps/text-editor"
tags:
  - "v1"
  - "contexts"
  - "scratchpad"
---



Make explicit text-editor and scratchpad launches work from an empty context without auto-opening a terminal as a workaround.

## Scope

- Reproduce the non-operative path when opening a text editor or scratchpad from an empty initial context.
- Ensure explicit app launches materialize a root pane when the active context has no focused pane.
- Keep the welcome/empty context state valid until the user explicitly launches something.
- Add HostHarness coverage for text-editor/scratchpad launch into an empty context.
- Log the empty-context root-pane launch path at `info` level.

## Non-Scope

- Do not remove the welcome screen or automatically create a terminal on app startup.
- Do not change generic process-app empty-context behavior unless the repro shows it is still incomplete.

## References

- `src/pane_ops/create.rs`
- `src/app/text_editor_app.rs`
