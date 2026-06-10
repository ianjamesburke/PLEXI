---
id: "0010"
title: "App authoring: component polish and small-pane fit"
status: backlog
sprint: "s2"
estimate: 8h
blocked_by: ["0008"]
blocked_by_gh: []
gh_issue: ["2111"]
area: ["sdk/python", "ui/widgets"]
tags: ["app-authoring", "components", "small-pane"]
---

Tighten SDK component defaults so generated apps fit small panes without footer clipping, text overlap, or egui-looking shortcut rows.

## Why

Generated apps will inherit every rough SDK component decision. The app authoring sprint must make those defaults boring and solid.

## Gotchas

- Keep host layout responsible for spacing and fit; do not push pixel math back into apps.

## References

- GitHub issue #2111
- `docs/prm/app-framework-marketplace.md`
