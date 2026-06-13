---
id: "0030"
title: "v1 RELEASE GATE: cut v1 (acceptance = PRM Definition Of Finished)"
status: backlog
sprint: "s14"
estimate: 16h
blocked_by: []
gh_issue: []
area: ["infra/build", "cli/commands"]
tags: ["v1", "release", "gate"]
---

This is the terminal v1 release milestone, not a dispatchable implementation task. Do not send it to an agent. Completing it means "v1 is cut."

The entire v2 phase is parked in `v2-backlog/` (out of the active board) precisely so the live board stays v1-only. When v1 ships, follow `v2-backlog/README.md` to reintroduce v2; its `v2-after-v1` gate hangs on this task, so reintroduced v2 work becomes claimable the moment this is done.

## Acceptance

v1 is done when the PRM Definition Of Finished passes — see `docs/prm/app-framework-marketplace.md` (Definition Of Finished + section 5, v1 Release Readiness). The release-cut checklist is owned there, not duplicated here. In summary, before marking this done:

- App framework, trust/packaging, and marketplace plan all meet their Definition Of Finished bullets.
- Clean install, upgrade, channel isolation, local package install, hosted marketplace install, and app trust flows are verified on a non-customized profile (alpha/beta/main/PR where routing or profile isolation matters).
- Public docs and CLI references are regenerated from the current build; security/trust wording audit (0031) is complete.

## Why this is a gate, not a feature

The release is not ready when features land; it is ready when a clean install/upgrade path exercises the v1 workflows without hidden local state. That is a release ritual with one binary outcome (shipped / not shipped), so it is modeled as the single gate anchor rather than a feature in the dispatch pool. Mark it done only when v1 actually ships.

## Gotchas

- Do not use a customized alpha config as evidence that defaults work.
- Marking this done unblocks 83 v2 tasks — do not complete it early to clear the queue.
