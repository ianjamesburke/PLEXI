---
id: "0020"
title: "Marketplace: browse and install from registry"
status: in-progress
estimate: "16h"
started_at: "2026-06-13T01:57:12Z"
sprint: "s4"
blocked_by:
  - 18
  - 16
gh_issue: []
area:
  - "cli/commands"
  - "host/permissions"
  - "infra/server"
tags:
  - "marketplace"
  - "install"
  - "trust-labels"
---


Let a user browse reviewed apps and install a free hosted app while seeing the same trust labels and capabilities as local package install.

## Why

This is the concrete Marketplace-up moment: a reviewed app can be discovered, inspected, installed, and run from the registry.

## Gotchas

- Do not require hosted login for local app install.
- Remote install must reuse the package trust sheet, not bypass it.

## References

- `docs/prm/app-framework-marketplace.md`
- `docs/prm/marketplace-hosted.md`
