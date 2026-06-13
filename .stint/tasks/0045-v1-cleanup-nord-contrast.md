---
id: "0045"
title: "v1 cleanup: Nord theme contrast"
status: done
estimate: "3h"
actual: "15m"
started_at: "2026-06-13T19:47:18Z"
completed_at: "2026-06-13T20:01:58Z"
sprint: "s11"
blocked_by: []
gh_issue:
  - "2062"
area:
  - "ui/widgets"
tags:
  - "v1"
  - "cleanup"
  - "theme"
---




Audit and adjust Nord theme dim/section text contrast where it is hard to read on dark surfaces.

## Why

Theme presets should remain usable across sidebar, overlays, status labels, and pane chrome.

## Variance Note

Estimated 3h; actual 15m. The fix was two hex constant changes in one file. The estimate likely assumed a broader audit involving multiple files and themes, but the issue was isolated entirely to Nord's `text_dim` and `text_section` values which were identical to background colors.
