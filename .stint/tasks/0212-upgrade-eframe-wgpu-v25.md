---
id: "0212"
title: "Upgrade eframe to v0.32 and wgpu to v25"
status: todo
estimate: "2h"
sprint: "s32"
blocked_by: []
gh_issue: []
area:
  - "infra/build"
tags:
  - "v1"
  - "tooling"
---

Upgrade `eframe` from v0.31 to v0.32 and `wgpu` from v24 to v25. This eliminates the `block v0.1.6` transitive warning ("code that will be rejected by a future version of Rust") by moving to wgpu's Metal backend that uses `objc2-metal` instead of the deprecated `metal`+`block` stack.

## Scope

- Bump `eframe`, `egui-wgpu`, and `wgpu` in `Cargo.toml` to their v0.32 / v25 equivalents
- Resolve any egui API breakage (egui 0.32 changelog is the reference)
- Confirm `cargo build` clean and `cargo test --bin plexi` green
- Confirm the `block v0.1.6` warning is gone from CI output

## Non-Scope

- No egui feature adoptions beyond what's needed to compile
- No wgpu feature flag changes

## Why

`block v0.1.6` will become a hard error in a future Rust edition; resolving it now avoids a forced emergency upgrade later.

## References

- `Cargo.toml` — eframe/wgpu version pins
- `Cargo.lock` — current dep tree: `metal v0.31.0 → block v0.1.6`
