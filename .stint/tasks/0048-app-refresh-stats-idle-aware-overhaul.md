---
id: "0048"
title: "App refresh: Stats idle-aware overhaul"
status: backlog
sprint: "s8"
estimate: 8h
blocked_by: []
blocked_by_gh: []
gh_issue: ["2144"]
area: ["apps/stats", "host/events"]
tags: ["v1", "apps", "app-refresh", "stats"]
---

Refresh the Stats app UI and make its time calculations ignore or clamp idle focus segments caused by leaving a pane open while away.

Use the 15-minute pane-switch heuristic from issue #2144: if no pane switch happens for at least 15 minutes, truncate that stale segment to 1 minute and ignore further stale time until pane switches resume faster than the 15-minute window.

## Why

Stats should report active use, not passive focus ownership. Fake multi-hour pane blocks make the app hard to trust.

## Gotchas

- `focus_changed.duration_secs` is focus ownership, not proof of user activity.
- Do not add new global activity monitoring for this task; use the existing focus event `reason` and duration fields unless implementation proves they are insufficient.
