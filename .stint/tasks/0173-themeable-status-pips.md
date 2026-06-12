---
id: "0173"
title: "v1 cleanup: themeable status pips — optional color overrides + dim factor"
status: backlog
estimate: "2h"
sprint: "s11"
blocked_by: []
blocked_by_gh: []
gh_issue:
  - "2219"
area:
  - "ui/widgets"
  - "host/config"
tags: []
---

## What

Expose optional [theme] pip_working/pip_idle/pip_blocked overrides (falling
back to success/warning/danger — single source of truth) plus pip_dim
(default 0.45, currently hardcoded UNFOCUSED_DIM) so status pips harmonize
with custom themes.

## References

- GitHub issue #2219
- src/ui/activity.rs:6-52
- src/ui/theme.rs
