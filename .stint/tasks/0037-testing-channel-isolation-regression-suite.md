---
id: "0037"
title: "Testing: channel isolation regression suite"
status: backlog
sprint: "s8"
estimate: 8h
blocked_by: []
blocked_by_gh: []
gh_issue: ["2095"]
area: ["infra/testing", "host/config", "cli/commands", "infra/build"]
tags: ["testing", "v2"]
---

Add focused regression coverage for channel-specific profile, workspace, socket, app registry, secrets, and event paths across main, alpha, beta, and PR builds.

## Why

Channel leakage is expensive to debug and dangerous for release confidence, even if the broader testing pass is post-v1.
