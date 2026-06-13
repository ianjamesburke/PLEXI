---
id: "0123"
title: "v2 media: camera capture capability"
status: backlog
sprint: "s27"
estimate: 12h
blocked_by: []
gh_issue: ["1505"]
area: ["sdk/pgap", "apps/examples", "host/video"]
tags: ["v2", "media", "camera", "blocked"]
---

Add live camera capture through a `video.capture` capability, camera source routing, device listing, and a webcam-viewer POC.

## v1 Decision

Not a v1 blocker. Camera capture depends on the production video pipeline and is outside the app-framework/package/trust v1 path.
