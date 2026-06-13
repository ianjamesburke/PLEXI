---
id: "0119"
title: "v2 SDK: timer capability"
status: backlog
sprint: "s26"
estimate: 4h
blocked_by:
gh_issue: ["293"]
area: ["sdk/pgap"]
tags: ["v2", "sdk", "timer"]
---

Add `SetTimer` / `PlexiEvent::Timer` so apps can request periodic or delayed behavior without polling.

## v1 Decision

Not a v1 blocker. No current v1 milestone requires periodic app behavior beyond existing frame/event flows.
