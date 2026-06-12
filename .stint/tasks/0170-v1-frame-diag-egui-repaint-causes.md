---
id: "0170"
title: "v1 PGAP perf: back frame_diag with egui repaint_causes()"
status: backlog
estimate: "2h"
sprint: "s13"
blocked_by: []
gh_issue:
  - "2209"
area:
  - "host/events"
tags:
  - "v1"
  - "pgap"
  - "performance"
  - "instrumentation"
---


## What

frame_diag counts note() calls, and some sites note once per rendered frame
regardless of what woke the renderer (terminal_cursor_blink showed 601/601
frames while a per-frame Title send was the real driver). Aggregate egui's
ctx.repaint_causes() (file:line ground truth) into the 10s summary and move
the egui_term blink note inside the toggle branch.

## Why

The 2026-06-12 idle-CPU audit needed a custom probe build to find a cause
egui already tracked; the next regression should be diagnosable from the
log alone.

## Done When

Idle focused terminal logs ~2 blink wakes/sec and the summary lists egui
file:line causes (gh #2209 Done When).
