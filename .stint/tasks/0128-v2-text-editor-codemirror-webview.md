---
id: "0128"
title: "v2 text editor: CodeMirror WebView pane"
status: backlog
sprint: "s29"
estimate: 12h
blocked_by:
  - 39
  - 30
  - 31
gh_issue: ["1748"]
area: ["host/pane-ops", "sdk/pgap", "apps/file-browser", "apps/text-editor"]
tags: ["v2", "text-editor", "webview"]
---

Explore replacing the egui TextEdit path with a CodeMirror 6 WebView editor pane that integrates with file open and scratchpad flows.

## v1 Decision

Not a v1 blocker. v1 text-editor work is covered by the refinement and workspace-restore tasks; a WebView/CodeMirror replacement is a larger runtime/editor direction after the v1 app framework stabilizes.
