---
id: "0180"
title: "Testing: cargo test + coverage gate in validate-pr — skip binary install on full coverage"
status: todo
estimate: "8h"
sprint: "s8"
blocked_by: []
gh_issue:
  - "2162"
area:
  - "infra/testing"
  - "infra/skills"
tags:
  - "v1"
  - "testing"
---


Wire `cargo test --bin plexi` and `cargo llvm-cov` into the ship cycle so validate-pr runs the test suite as a pre-gate and can skip the manual binary install for PRs whose diff is fully covered by harness tests.

## Why

Epic #2162: the test infrastructure (egui_kittest, PlexiUiHarness, HostHarness, scenes) exists with 784+ tests but is not systematically required — features can ship without tests and validate-pr always falls back to manual binary install. The /testing skill (0154) and per-test profile isolation (0178) are done; this is the remaining lifecycle wiring.

## Done When

validate-pr runs `cargo test --bin plexi` as a pre-gate, reads the Test Evidence block from the /testing skill, and skips binary install when coverage criteria are met (gh #2162 acceptance criteria).
