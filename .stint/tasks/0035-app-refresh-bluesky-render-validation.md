---
id: "0035"
title: "App refresh: Bluesky render-validated rewrite"
status: in-progress
estimate: "12h"
started_at: "2026-06-10T22:39:24Z"
sprint: "s7"
blocked_by: []
gh_issue:
  - "2118"
area:
  - "apps/bluesky"
  - "sdk/python"
tags:
  - "apps"
  - "app-refresh"
  - "render-validation"
---


Refresh the Bluesky app around permission-correct avatar loading, proportional image rendering, measured thread text, visible stats, and PNG render validation.

## Why

Reference apps should demonstrate the SDK's intended quality bar instead of accumulating patched visual and permission bugs.
