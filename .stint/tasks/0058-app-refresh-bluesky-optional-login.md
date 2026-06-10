---
id: "0058"
title: "App refresh: Bluesky optional login"
status: backlog
sprint: "s8"
estimate: 8h
blocked_by: ["0014", "0017", "0041"]
blocked_by_gh: []
gh_issue: ["2071"]
area: ["apps/examples", "host/permissions", "host/secrets", "sdk/python"]
tags: ["v1", "app-refresh", "bluesky", "permissions", "secrets"]
---

Add optional Bluesky login as the app-side proof for point-of-need capability prompts and persistent secret writes.

## Why

This belongs in the app refresh lane after the trust, permissions, and secrets primitives are stable enough to exercise from a first-party reference app.
