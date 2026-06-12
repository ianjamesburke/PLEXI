---
id: "0178"
title: "cargo test runs mutate a shared real profile dir in $HOME — per-test tempdir isolation"
status: backlog
estimate: "4h"
sprint: "s8"
blocked_by: []
blocked_by_gh: []
gh_issue:
  - "2229"
area:
  - "infra/testing"
  - "host/config"
tags: []
---

## What

`config_dir()` resolves from the test binary basename, so every
`cargo test --bin plexi` run shares one real `~/.plexi-<hash>/` directory
across parallel test threads — genuine cross-test interference plus ~25
stray profile dirs polluting $HOME. Extend the existing per-thread
`set_test_channel` override to a full profile-path override, have
`HostHarness::new()` acquire a unique tempdir guard, clean up stray dirs,
and add the flaky-dismissal rule to the testing skill.

## References

- GitHub issue #2229
- src/config/mod.rs:818-849,906-917
- src/testing/mod.rs (HostHarness::new)
- src/pane_ops/create.rs:1853
- .agents/skills/testing/SKILL.md
