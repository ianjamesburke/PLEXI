---
id: "0197"
title: "command palette: commands.toml entries with hot reload"
status: in-progress
estimate: "8h"
created_at: "2026-06-15T17:04:26Z"
started_at: "2026-06-15T17:53:31Z"
blocked_by: []
gh_issue:
  - "2269"
area:
  - "ui/overlays"
  - "cli/commands"
  - "host/config"
tags:
  - "v1"
  - "palette"
  - "commands"
---


Surface workspace and global `commands.toml` entries in the command palette without creating a second command registry.

## Why

`plexi run` already gives users and agents a structured command layer over shell aliases. The palette should make those commands discoverable inside Plexi with scope chips, fuzzy search, and hot reload.

## Scope

1. Reuse the existing `plexi run` parser and execution semantics from `src/cli/mod.rs` and `src/cli/run.rs`.
2. Resolve channel-scoped workspace commands from the active workspace root and global fallback commands/scripts from the profile.
3. Cache command rows at palette-open time, then invalidate on focused workspace or command-file mtime changes.
4. Render workspace/global command rows in `src/overlays/command_palette.rs` with scope chips.
5. Add unit/HostHarness coverage for precedence, hot reload, fuzzy ordering, and selection dispatch plus one `PlexiUiHarness` screenshot.

## Notes

Task `0185` already implemented dynamic shell completions from `commands.toml`; reuse its scope assumptions instead of deriving channel paths again.
