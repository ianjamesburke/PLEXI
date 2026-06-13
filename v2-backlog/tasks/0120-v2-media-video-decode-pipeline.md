---
id: "0120"
title: "v2 media: video pipeline production hardening"
status: backlog
sprint: "s27"
estimate: 12h
blocked_by:
gh_issue: ["947"]
area: ["host/video"]
tags: ["v2", "media", "video"]
---

Harden the already-landed host video pipeline for real files, frame timing, teardown, and app-facing reliability.

## Current State

The original #947 body is stale: `open_video`, `set_video_state`, SDK emitter support, host routing, `VideoOpenAck` / `VideoOpenError`, and routing tests exist. This is not a v1 blocker because File Explorer v1 explicitly does not rewrite media players; v2 should turn the substrate into a production media capability.
