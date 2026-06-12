---
id: "0164"
title: "config: minimal default template, docs/CONFIG.md reference, plexiapp.com links"
status: backlog
estimate: "4h"
sprint: "s11"
blocked_by: []
gh_issue:
  - "2195"
area:
  - "host/config"
  - "infra/docs"
tags:
  - "config"
  - "docs"
---

Rewrite `scripts/default-config.toml` to ~50 prose-free lines, fold `theme_preset` into `[theme] preset` (loud error on old key), write `docs/CONFIG.md` as the canonical reference, and replace all `plexiapp.dev` links with `plexiapp.com`. Full spec in GH #2195.
