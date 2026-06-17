---
id: "0214"
title: "Host event subscriptions for third-party agents"
status: in-progress
estimate: "12h"
started_at: "2026-06-17T21:08:31Z"
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
brokered `PlexiEvent::AppEvent` deliveries through both CLI NDJSON and a
host-level MCP server.

## Scope

- Add one host subscription service that owns subscriber identity, broker
  checks, `AppTimeline` subscriptions, delivery draining, and disconnect
  cleanup.
- Add `plexi events subscribe <app_id> <stream_name>` as the universal
  subprocess/stdout frontend, with payload, trigger, resource, and all-stream
  options.
- Keep the socket connection open for subscriptions and stream JSON lines to
  stdout until interrupted.
- Add a host-level MCP server surface that exposes the same subscription
  primitive to MCP-aware agents without requiring a wrapper subprocess.
- Reuse `AppTimeline` subscriptions, broker grants, payload shaping, trigger
  modes, and cleanup semantics. CLI and MCP are transports only, not second
  event buses.
- Derive subscriber identity from the terminal pane / agent state;
  never let the CLI spoof arbitrary subscriber identity.
- Remove the subscription and queued deliveries when the client disconnects.
- Add focused host, socket, and MCP regression coverage for grant refusal,
  grant success, payload shaping, event delivery, and disconnect cleanup.

## Non-Scope

- Do not change per-app MCP server behavior.
- Do not introduce a parallel event store, schema, or permission model.

## Proposal

Land the optimal infrastructure now: a shared host subscription core plus two
thin transports. The CLI NDJSON stream is the lowest-common-denominator path
for any agent that can read subprocess stdout; the host-level MCP server is the
native path for agents that already speak MCP. Both must wrap the same core so
Plexi has one event permission model, one delivery lifecycle, and one schema.

## End-to-End Companion

Build a tiny first-party PGAP proof app, `event-probe`, as the manual validation
companion. It should declare one stream (`probe.tick`) and expose one visible
button/action that emits a deterministic event:

```json
{
  "event": "probe.tick",
  "summary": "Probe tick 3",
  "resource_id": "probe-session",
  "revision_after": "tick-3",
  "payload": { "count": 3 }
}
```

Manual validation should exercise both agent-facing transports:

- Open `event-probe` in Plexi and trigger one event from the UI.
- From a terminal pane, run `plexi events subscribe event-probe probe.tick`
  and confirm it prints the subscribed response plus the emitted event JSON.
- Spawn a fresh Claude Code or Codex pane with the Plexi host MCP server
  configured, then send a prompt equivalent to:

```text
Use the Plexi MCP server. Subscribe to the event-probe app's probe.tick event
stream with full payload. Wait for the next event and report its count and
summary. Do not read plexi.log or call the CLI event command directly.
```

The pass condition is that the external agent discovers the MCP tool/resource,
subscribes through MCP, waits, and reports the next `probe.tick` event after the
button is clicked. This proves the MCP adapter is actually wired, not just that
the Rust host test can route an in-process delivery.

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
- `src/process_app/mcp_server.rs` for reference only; host-level MCP should not
  be implemented by changing per-app MCP semantics.
