---
id: "0012"
title: "App authoring: verification harness and docs"
status: backlog
sprint: "s2"
estimate: 12h
blocked_by: ["0011"]
blocked_by_gh: []
gh_issue: []
area: ["infra/docs", "sdk/python", "sdk/pgap"]
tags: ["app-authoring", "docs", "verification"]
---

Add acceptance coverage and docs proving generated apps render, handle input, save state, and avoid layout overlap.

## Why

The app authoring milestone is complete only when agents can verify the app they generated without relying on visual guesswork.

## Gotchas

- Tests define done. Avoid merging scaffold or docs changes that are not exercised by the render/inspect loop.

## References

- `docs/prm/app-framework-marketplace.md`
- `docs/sdk-v2.md`
- `docs/SDK_QUICKSTART.md`
