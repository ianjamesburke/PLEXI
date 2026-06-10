---
id: "0113"
title: "File Explorer: linked terminal exit directory handoff"
status: backlog
sprint: "s1"
estimate: 5h
blocked_by:
  - 86
gh_issue: ["2145"]
area: ["apps/file-browser", "host/terminal", "host/pane-ops"]
tags: ["v1", "file-explorer", "terminal", "cwd"]
---

Replace per-navigation terminal `cd` writes with an explicit exit-time directory handoff.

## Why

Browsing in File Explorer should not silently mutate the linked terminal cwd. If the user exits from a different directory, show a compact confirmation so they can choose whether the terminal should change directories or stay where it started.

## UX Shape

When exiting File Explorer from a different directory, prompt with a fast keyboard path: accept the handoff to change the terminal once, or dismiss/escape to close without changing the terminal. Keep terminal-to-File-Explorer sync where appropriate, but avoid app-to-terminal sync loops.
