---
id: "0035"
title: "App refresh: Bluesky render-validated rewrite"
status: done
estimate: "12h"
actual: "62m"
started_at: "2026-06-10T22:39:24Z"
completed_at: "2026-06-10T23:40:33Z"
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

Variance note: actual time was lower than estimate because the rewrite stayed localized to one app plus one SDK capability bookkeeping fix, with render validation catching issues quickly.
