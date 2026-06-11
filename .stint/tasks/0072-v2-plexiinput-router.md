---
id: "0072"
title: "v2 host architecture: PlexiInput router"
status: backlog
sprint: "s15"
estimate: 12h
blocked_by:
  - 71
  - 30
  - 31
gh_issue: ["1239"]
area: ["host/pane-ops"]
tags: ["v2", "input", "architecture", "blocked"]
---

Route frame input by ownership transfer to a single focus owner instead of letting overlays, apps, terminals, and global key polling observe the same shared event queue.

## Why

This unblocks the terminal Cmd+A and Windows copy/paste fixes and closes the class of paste/key leakage bugs caused by shared input observation.
