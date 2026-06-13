---
id: "0018"
title: "Marketplace: hosted registry and CDN"
status: done
estimate: "16h"
actual: "319m"
started_at: "2026-06-13T01:57:12Z"
completed_at: "2026-06-13T07:16:09Z"
sprint: "s4"
blocked_by:
  - 17
gh_issue: []
area:
  - "infra/server"
  - "infra/build"
tags:
  - "marketplace"
  - "registry"
  - "cdn"
---



Stand up the hosted app registry surface needed for reviewed marketplace apps, using local package metadata as the source format.

## Why

The marketplace can only be up when users can fetch reviewed app metadata from a hosted registry without making hosted login required for local apps.

## Gotchas

- Hosted services may list, review, and sell apps, but installed apps and state stay local.

## References

- `docs/prm/app-framework-marketplace.md`
- `docs/prm/marketplace-hosted.md`
