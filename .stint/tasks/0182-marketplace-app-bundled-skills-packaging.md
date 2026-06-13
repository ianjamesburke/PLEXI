---
id: "0182"
title: "Marketplace: app-bundled skills packaging"
status: backlog
estimate: "8h"
sprint: "s32"
blocked_by:
  - 181
gh_issue: []
area:
  - "cli/commands"
  - "sdk/pgap"
tags:
  - "marketplace"
  - "skills"
  - "packaging"
---


Let an app package carry a `skills/` directory that installs alongside the app into the same scope (workspace or global) the app was installed into. Packaging only — registration as dormant skills, no activation logic yet.

## Why

The chess app should ship with its skills attached. This is the packaging half of the app-bundles-skills vision: the distributable unit can contain skills, and they land in the correct scope. Activation (when the skill becomes available) is a separate runtime task (`0183`).

## Done When

- The package validator (`0015`) recognizes a `skills/` dir and validates its contents.
- `plexi app install` places bundled skills into the scope-appropriate skills dir alongside the app.
- Bundled skills register as dormant (present but not yet active) — no activation behavior in this task.
- `plexi app inspect` lists bundled skills in the trust sheet.
- Tests: a package with a `skills/` dir installs the skills into the right scope; an app without one is unaffected.

## References

- `docs/prm/marketplace-hosted.md`
- Depends on scope resolution from `0181`.
