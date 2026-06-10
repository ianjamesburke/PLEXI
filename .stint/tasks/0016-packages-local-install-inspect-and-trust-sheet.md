---
id: "0016"
title: "Packages: local install inspect and trust sheet"
status: backlog
sprint: "s3"
estimate: 12h
blocked_by: ["0015"]
blocked_by_gh: []
gh_issue: ["866"]
area: ["host/permissions", "ui/overlays", "cli/commands"]
tags: ["packages", "trust-labels", "install"]
---

Show manifest, runtime trust label, and declared capabilities before local package install proceeds.

## Why

Users need to understand what a package can do before installing it, and marketplace review depends on the same install-time language.

## Gotchas

- Trust labels must be blunt: reviewed native process, sandboxed WASM only after WASM exists, or first-party core.

## References

- GitHub issue #866
- `docs/prm/app-framework-marketplace.md`
