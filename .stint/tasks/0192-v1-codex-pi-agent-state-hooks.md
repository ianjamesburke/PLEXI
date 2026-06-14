---
id: "0192"
title: "v1: Codex and Pi agent state hooks"
status: done
estimate: "3h"
actual: "10m"
started_at: "2026-06-14T21:37:18Z"
completed_at: "2026-06-14T21:46:36Z"
blocked_by: []
gh_issue: []
area:
  - "infra/agents"
tags:
  - "v1"
  - "tooling"
  - "codex"
  - "pi"
---



Wire up the Plexi agent-state hook to Codex CLI and Pi so they report tool activity into Plexi pane headers/status the same way Claude Code does.

## Background

Claude Code calls `~/.plexi-beta/hooks/claude-code-agent-state.sh` on every tool batch and permission request via `~/.claude/settings.json`. Codex and Pi are also installed and run regularly but their tool events don't surface in Plexi.

## Scope

### Codex

- Audit Codex hook event format: check `~/.codex/config.toml` hook syntax and what fields the Codex hook payload contains.
- Determine whether the existing `claude-code-agent-state.sh` script can handle Codex payloads directly, or whether a separate `codex-agent-state.sh` is needed.
- Wire hook(s) to the relevant Codex hook events (tool use, permission request, session start/end — match Claude parity as closely as Codex allows).
- Confirm hook trust is persisted so Codex doesn't prompt on every run.

### Pi

- Audit Pi hook/extension system: check `~/.pi/agent/settings.json` and Pi docs for a hook or lifecycle callback mechanism.
- If Pi supports hooks natively, wire the same agent-state script.
- If Pi does not support hooks, document the limitation in the task body and close as `won't-do` with a note pointing to a potential Pi feature request.

### Script / Host

- The hook script must detect which agent is calling it (Claude / Codex / Pi) and label the pane status accordingly if the format differs.
- The hook script must remain backward-compatible: no changes to Claude Code behaviour.
- Log at `info` level when an agent-state update fires so the channel log confirms it worked.

## Non-Scope

- Do not rewrite the hook script from scratch.
- Do not add hooks for any other agent (Gemini CLI, etc.) in this task.
- Do not change the Plexi host to handle new hook message shapes — adapt the script to emit the existing shape.

## Implementation Notes

- Codex supports command hooks via `~/.codex/hooks.json` or inline `[hooks]` config tables. The installer writes `~/.codex/hooks.json` to avoid rewriting the user's main config and registers SessionStart, UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, and Stop.
- Codex non-managed hook trust is intentionally user-mediated. After install, Codex persists trust for the hook hash through `/hooks`; the installer cannot safely bypass that in normal interactive use.
- Pi supports lifecycle/tool observation through its extension system rather than a Claude/Codex-style hook config. The installer writes an auto-discovered global extension at `~/.pi/agent/extensions/plexi-agent-state.ts`.
- Claude, Codex, and Pi all call the existing `claude-code-agent-state.sh` path. The script detects the calling agent through `PLEXI_AGENT_NAME` when needed and emits the existing `plexi agent report` shape.
- Variance: implementation was much faster than the 3h estimate because Codex and Pi already expose compatible lifecycle payloads; this only needed installer/config wiring and script adaptation, not host changes.

## References

- `~/.plexi-beta/hooks/claude-code-agent-state.sh`
- `~/.claude/settings.json` (Claude hook wiring)
- `~/.codex/config.toml` (Codex config)
- `~/.pi/agent/settings.json` (Pi config)
