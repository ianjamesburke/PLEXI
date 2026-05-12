# Agent Supervisor Pattern and Pane Trust Model

This document captures thinking from a design session on agent orchestration in Plexi. It maps to **Phase 2** in the North Star.

## The Problem

Claude Code can already launch Plexi panes and dispatch sub-agents via CLI. But those panes are fire-and-forget — there's no feedback loop, no way to unblock a child that's waiting for clarification, and no audit trail for inter-agent communication.

The natural next step is a supervisor pattern: one orchestrator pane that watches child agent panes, routes clarifying questions back to itself, answers them, and unblocks the children.

## The Two Primitives Needed

### 1. `pane input <id> <string>` — raw PTY write

Write a string directly to a pane's PTY stdin. No process awareness, no key-combo abstraction. Whatever is running in that pane reads it as typed input.

**Security constraint:** scoped to the spawn tree. A pane can only inject into panes it spawned. This preserves the hackability of raw terminal interaction while bounding the blast radius. Every write is logged in `permissions.jsonl`.

### 2. `plexi emit` as the status bus

Child panes already have `plexi emit`. Claude Code hooks (`Stop`, `UserPromptSubmit`, `PreToolUse`) call `plexi emit` with a structured payload:

```json
{"pane": "<id>", "state": "waiting", "question": "..."}
```

The spawner pane subscribes and acts on it.

**Security constraint:** emit payloads must be attributable to the sending pane. An unauthenticated emit bus allows a compromised child to impersonate any state. Pane-scoped emit channels (only a pane's own emissions are attributable to it) fix this.

## The Supervisor Loop

```
child hook → plexi emit {state, question}
spawner receives emit → reasons over question + task context
spawner → plexi pane input <child-id> "answer\n"
child unblocks
```

The spawner has full task context for every child it launched. It can answer most clarifying questions without human involvement. Eventually, humans only see questions the orchestrator couldn't resolve.

## Spawn Tree Capability Inheritance

Child panes can spawn their own children. The tree can be arbitrarily deep. The invariant:

- You can only inject into panes you spawned
- You can only grant capabilities you yourself hold
- No privilege escalation through delegation

This is standard capability inheritance — the same model Unix uses for process groups and pipes.

## Graduated Trust Tiers

Static tier labels (peer / subordinate / orchestrator) are Phase 2 infrastructure. The richer model: trust is earned dynamically.

Agents start with a narrow capability allowlist. As they demonstrate trustworthy behavior, the orchestrator grants additional capabilities. The mechanics:

- Capability grants are explicit, logged, and reversible
- An agent can only grant capabilities it holds (capability inheritance)
- Promotion criteria: N successful tool uses without policy violations, human sign-off, or both

**Who can promote?**
- Human-only (safe default) — human is never out of the loop
- Orchestrator-can-promote (opt-in capability) — enables emergent autonomy, requires strong audit trail

This is gradient descent on agent capability space: capability is the parameter, observed behavior is the signal, and every update is logged and human-reversible.

## What Doesn't Ship in v1

`pane input` should not ship in v1 without the spawn-tree scope constraint. An unscopeed version is a universal code injection primitive — any caller can inject into any pane. Restricting it later is a breaking change.

Defer `pane input` entirely to Phase 2, where it lands correctly scoped from day one.

## Relationship to North Star Phase 2

From `NORTH_STAR.md`:

> `[app.agent]` manifest — installable agent apps with system prompt + tool allowlist. Agent-invokable app registry. **Trust tiers: peer, subordinate, orchestrator. Agents that can spawn panes, wire pipes, and hand off to other agents.**

This document fills in the implementation detail behind that line. The spawn-tree scope, capability inheritance, and dynamic trust promotion are how Phase 2 agent infrastructure gets built without creating exploitable side channels.
