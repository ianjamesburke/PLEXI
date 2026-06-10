---
id: "0013"
title: "Trust: remove ambient host control from app processes"
status: backlog
sprint: "s3"
estimate: 16h
blocked_by:
  - 12
gh_issue: []
area: ["host/permissions", "sdk/pgap", "cli/commands"]
tags: ["trust", "capabilities", "marketplace"]
---

Stop app subprocesses from gaining unmediated host control through inherited routing such as `PLEXI_SOCKET`, or bind that routing to app identity and capability checks.

## Why

Marketplace trust labels cannot launch while native apps can bypass capability checks through inherited host routing or CLI subprocesses.

## Gotchas

- Python apps are reviewed native processes, not sandboxed apps.
- Do not describe consent plus audit as process isolation.

## References

- `docs/prm/app-framework-marketplace.md`
- `docs/SECURITY_MODEL.md`
