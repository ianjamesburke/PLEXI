---
id: "0054"
title: "v2 terminal perf: blink repaint cost"
status: backlog
sprint: "s18"
estimate: 8h
blocked_by:
  - 30
  - 31
gh_issue: ["2022"]
area: ["host/terminal"]
tags: ["v2", "performance", "terminal"]
---

Reduce terminal idle GPU/CPU cost by avoiding full terminal-grid repaint work for simple cursor and search blink animations.

## Why

Terminal idle cost matters, but this is a separate renderer-internal lane from PGAP performance.
