---
id: "0021"
title: "Marketplace: paid apps license and revenue model"
status: in-progress
estimate: "12h"
started_at: "2026-06-13T01:57:12Z"
sprint: "s4"
blocked_by:
  - 20
gh_issue: []
area:
  - "infra/server"
tags:
  - "marketplace"
  - "billing"
  - "licenses"
---


Specify paid app purchase, license metadata, revenue share, refunds, takedowns, and publisher analytics without changing local app ownership.

## Why

The marketplace business model needs to be coherent before paid submissions start, even though free hosted app install can ship first.

## Gotchas

- Paid licensing can be hosted, but installed code and user state remain on disk.

## References

- `docs/prm/app-framework-marketplace.md`
- `docs/prm/marketplace-hosted.md`
