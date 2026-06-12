---
id: "0051"
title: "v1 PGAP perf: background wake scheduling"
status: in-progress
estimate: "12h"
started_at: "2026-06-12T18:32:11Z"
sprint: "s13"
blocked_by:
  - 50
gh_issue:
  - "2021"
area:
  - "host/pane-ops"
  - "host/notifications"
  - "sdk/pgap"
tags:
  - "v1"
  - "pgap"
  - "performance"
  - "background-apps"
---


Move parked and non-active app progress off every-frame `background_tick()` polling and onto explicit wakes/deadlines for timers, workers, MCP, AI, and notifications.

## Why

Background app behavior must remain prompt without forcing foreground repaint cadence to poll every app.
