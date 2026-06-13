---
id: "0102"
title: "v2 dispatch: fail-to-ship recovery flow"
status: backlog
sprint: "s23"
estimate: 6h
blocked_by: []
gh_issue: ["1552"]
area: ["infra/agents"]
tags: ["v2", "dispatch", "agents", "recovery"]
---

Automate failed ship cleanup: close/revert the failed attempt, audit it, annotate the issue, relabel it, and restart from a clean lane.
