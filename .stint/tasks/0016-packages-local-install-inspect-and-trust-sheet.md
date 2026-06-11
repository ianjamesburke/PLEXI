---
id: "0016"
title: "Packages: local install inspect and trust sheet"
status: done
estimate: "12h"
actual: "104m"
started_at: "2026-06-11T17:54:22Z"
completed_at: "2026-06-11T19:38:06Z"
sprint: "s3"
blocked_by:
  - 15
gh_issue: []
area:
  - "host/permissions"
  - "ui/overlays"
  - "cli/commands"
tags:
  - "packages"
  - "trust-labels"
  - "install"
---



Show manifest, runtime trust label, and declared capabilities before local package install proceeds.

## Why

Users need to understand what a package can do before installing it, and marketplace review depends on the same install-time language.

## Gotchas

- Trust labels must be blunt: reviewed native process, sandboxed WASM only after WASM exists, or first-party core.

## References

- `docs/prm/app-framework-marketplace.md`
