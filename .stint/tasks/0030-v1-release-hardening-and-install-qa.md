---
id: "0030"
title: "v1: release hardening and install QA"
status: backlog
sprint: "s14"
estimate: 16h
blocked_by: ["0028", "0029"]
blocked_by_gh: []
gh_issue: []
area: ["infra/build", "cli/commands"]
tags: ["v1", "release", "qa"]
---

Run v1 acceptance QA across install, upgrade, channel isolation, local package install, hosted marketplace install, and app trust flows.

## Why

The release is not ready when features land; it is ready when a clean install and upgrade path can exercise the v1 workflows without hidden local state.

## Gotchas

- Test alpha, beta, main, and PR-channel behavior where routing or profile isolation matters.
- Do not use customized alpha config as evidence that defaults work.
