---
id: "0125"
title: "v2 terminal: Windows process handle ownership"
status: backlog
sprint: "s28"
estimate: 6h
blocked_by:
gh_issue: ["1606"]
area: ["host/terminal"]
tags: ["v2", "terminal", "windows"]
---

Hold duplicated Windows process handles to avoid PID-reuse races in process cancellation, wait, drop, and reaper paths.
