---
id: "0209"
title: "Single-source generated infrastructure docs"
status: todo
estimate: "4h"
sprint: "s14"
blocked_by: []
gh_issue: []
area:
  - "infra/docs"
  - "infra/build"
tags:
  - "v1"
  - "tooling"
---


Every website doc that describes real infrastructure (config, CLI, SDK, PGAP
protocol, capabilities, keybindings) should be generated from the code that is
its source of truth — never hand-copied. The hand-maintained copies drift: the
config.md default block had diverged from `scripts/default-config.toml`
(wrong `[beta]`/`theme_preset`/quick_note sections), and per-doc
`verified_version` stamps silently went stale. CLI docs (`tools/gen_cli_docs`)
and SDK docs (`website/scripts/generate-sdk-docs.py`) already prove the
pattern. Extend it to the remaining factual reference content so prose pages
hold only narrative, and every fact has exactly one source.

## Scope

- Generate config.md's reference + default block from `scripts/default-config.toml`
  plus the `Config` / `KeybindingsConfig` structs in `src/config/mod.rs` (the
  overridable-action list and color-key list come from the structs, not a
  hand-list).
- Generate the capabilities reference from `src/app/permissions.rs` (the
  canonical capability enum/map) so docs and the host agree by construction.
- Generate the PGAP message-type reference from `src/app_protocol.rs` /
  `sdk/protocol/pgap.schema.json` instead of the hand-written summary in pgap.md.
- Add a CI staleness check per generated doc, mirroring
  `.github/workflows/check-cli-docs.yml`, so any drift fails the build.
- Each generator stamps `verified_version` from `Cargo.toml` (already the
  pattern for CLI/SDK after task-adjacent work — keep it consistent).

## Non-Scope

- Narrative/conceptual docs (getting-started, panes overview prose, app-design
  philosophy) stay hand-written — only their embedded factual reference blocks
  get sourced from code.
- No website visual redesign — content-sourcing only.
- Marketplace/registry doc generation (no stable schema yet).

## Why

Hand-duplicated reference docs are a recurring source of silent drift between
the shipped binary and plexiapp.com; generation + a CI gate makes drift
impossible rather than something a human has to catch.

## References

- `tools/gen_cli_docs/src/main.rs` — existing CLI doc generator (the pattern)
- `website/scripts/generate-sdk-docs.py` — existing SDK doc generator (ast-based)
- `.github/workflows/check-cli-docs.yml` — the staleness-gate to replicate
- `scripts/default-config.toml` — canonical config template
- `src/config/mod.rs` — `Config`, `KeybindingsConfig`, theme color keys
- `src/app/permissions.rs` — canonical capability list
- `src/app_protocol.rs`, `sdk/protocol/pgap.schema.json` — PGAP message types
