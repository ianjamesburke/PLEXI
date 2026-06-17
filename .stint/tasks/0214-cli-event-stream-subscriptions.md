---
id: "0214"
title: "CLI event stream subscriptions for terminal agents"
status: backlog
estimate: "8h"
blocked_by: []
gh_issue:
  - "2288"
area:
  - "cli/commands"
  - "host/events"
  - "sdk/pgap"
  - "agents"
tags:
  - "v1"
  - "assistant"
  - "events"
  - "agents"
---

Let third-party CLI agents subscribe to Plexi app event streams and receive
brokered `PlexiEvent::AppEvent` deliveries as newline-delimited JSON.

## Scope

- Add `plexi events subscribe <app_id> <stream_name>` with payload, trigger,
  resource, and all-stream options.
- Keep the socket connection open for subscriptions and stream JSON lines to
  stdout until interrupted.
- Reuse `AppTimeline` subscriptions, broker grants, payload shaping, trigger
  modes, and cleanup semantics. The socket path is transport only, not a second
  event bus.
- Derive the socket subscriber identity from the terminal pane / agent state;
  never let the CLI spoof arbitrary subscriber identity.
- Remove the subscription and queued deliveries when the client disconnects.
- Add focused host/socket regression coverage for grant refusal, grant success,
  payload shaping, event delivery, and disconnect cleanup.

## Non-Scope

- Do not build the host-level MCP server in this task.
- Do not change per-app MCP server behavior.
- Do not introduce a parallel event store, schema, or permission model.

## Proposal

Ship socket stream mode first, then expose it through a host-level MCP server
later. The socket path gives Claude Code, Codex, Pi, and any future terminal
agent immediate access through a subprocess/stdout contract while proving the
lifecycle issues MCP also needs: identity, grants, delivery shaping, disconnect
cleanup, and stable JSON.

## References

- GitHub issue #2288
- `docs/prm/agent-platform.md`
- `docs/prm/undo-and-app-events.md`
- `src/app/mod.rs`
- `src/cli/args.rs`
- `src/cli/mod.rs`
- `src/host/app_timeline.rs`
- `src/protocol/commands.rs`
- `src/protocol/events.rs`
- `src/testing/harness_tests.rs`
