---
id: "0043"
title: "v1 cleanup: pane info previous steps"
status: backlog
sprint: "s11"
estimate: 3h
blocked_by:
  - 147
gh_issue: ["2081"]
area: ["cli/commands", "host/navigation"]
tags: ["v1", "cleanup", "cli"]
---

Let `plexi pane info --previous` accept an optional step count so callers can inspect deeper focus history.

Sequenced after `0147` because validation handoff reliability is the immediate ship-pipeline CLI blocker.

## Why

Agent and workflow scripts sometimes need the pane focused two or three hops ago, not only the immediately previous pane.
