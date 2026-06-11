---
id: "0061"
title: "Host agent state: pane file slots"
status: in-progress
estimate: "10h"
started_at: "2026-06-11T06:31:01Z"
sprint: "s6"
blocked_by:
  - 33
  - 34
gh_issue:
  - "1994"
area:
  - "host/pane-ops"
  - "cli/commands"
  - "agents"
tags:
  - "v1"
  - "agents"
  - "state"
---


Add host-managed named file slots so agent panes can publish tasks and artifacts without PTY scraping or ad-hoc side-channel files.

## Why

Pane slots extend the host agent status/state lane from display state into recoverable coordination state.
