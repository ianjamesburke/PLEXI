# Plexi Cloud Assistant

Status: active.
Stint: none yet.
Parent: the local intake, receipt, and chess demonstration recorded here.
Authority plane: [`assistant-authority-model.md`](assistant-authority-model.md).
Last updated: 2026-10-05.

This document owns the versioned intake envelope, intake receipts, the chess tool contract, and the local simulated-intake proof. It does not own assistant product behavior ([`assistant-host-app.md`](assistant-host-app.md)), mesh addressing ([`assistant-agent-mesh.md`](assistant-agent-mesh.md)), run records ([`agent-run-orchestration.md`](agent-run-orchestration.md)), the wasm sandbox ([`wasm-runtime.md`](wasm-runtime.md)), Linux qualification ([`linux-support-plan.md`](linux-support-plan.md)), or marketplace accounts ([`marketplace-hosted.md`](marketplace-hosted.md), [`marketplace-monetization.md`](marketplace-monetization.md)). Those documents keep their rules. This one only names where a cloud-assistant path touches them.

No deployment, DNS, or Railway change belongs to this contract. Phone, daemon, and hosted variants are unsupported until their own gates exist.

## Seams

Turn injection for the in-process assistant is `AssistantHarness::run_turn`. That harness rejects any scripted call whose name does not start with `host.`, so it cannot drive a chess connector. Cloud intake therefore enters through `CloudSession::enqueue` and `CloudSession::pump`, which call `parse_intake` and then `ChessApp::dispatch`. `dispatch` builds a `ToolDispatcher` and calls `dispatch_call`. The host MCP server already exposes registered app tools as `<app_id>__<tool>` from `ToolDispatcher::from_namespaced_registry`; chess registers on that same registry through `AppEventSender::InProcess`. There is no second tool protocol.

Events are `AppTimeline::record_event` on the stream `chess.move_committed`. Subscribers are `SubscriptionRecord` values filtered by `evaluate_reach` and `resource_id`. A delivery is data. It does not grant a move.

Closing a view is `ChessApp::close_view`. `BackgroundPolicy::Continue` leaves the instance registered and callable. `BackgroundPolicy::StopOnLastViewClose` clears the tool list and makes a cached dispatcher fail with `instance_stopped`. Hiding a view or marking it `InactiveContext` does not stop the instance and does not add a grant. Cross-context discovery fails in `ToolDispatcher::from_registry` because `evaluate_reach` is same-context only.

Host restart without a window is not this contract. `app::quit::exit_host` still ends the process. Reopening `ChessApp::open` reloads `chess-journal.json` and `publish_pending` emits moves that were committed but not yet published. That is in-process recovery, not a daemon gate.

Linux packaging stays on the owner named in [`linux-support-plan.md`](linux-support-plan.md). This tree does not add a parallel port.

## Conflicts with existing contracts

`GrantRecord::matches` compares actor, target, and workspace. It does not compare `resource_id`. Chess authorization must not call it. `ChessStore::play` matches actor, game id, and side, and it refuses a client-supplied actor field. Context membership remains visibility (`evaluate_reach`), not authority: an actor in the chess context still needs a grant to play.

`assistant-agent-mesh.md` still discusses path-shaped addressing in places. Intake identities here are opaque strings (`agent_id`, `conversation_id`, `instance_id`). Display names are not references. One assistant fixture has one conversation id across detached and reattached views.

`AssistantHarness` remains the deterministic loop for `host.*` tools. The chess proof uses a scripted model only to choose a UCI string, then the real dispatcher. That is not a pass of the Assistant pane loop, and it is not a pass of a TOML scene: there is no chess pane under `apps/`.

## Contracts

`parse_intake` accepts schema version 1. The body is the fixture shape in `fixtures/cloud-assistant/intake-turn.json`: request, host, agent, conversation, expiry, text content, and an optional target resource plus expected revision. A client field for role, tenant, or actor is invalid input. The host attaches `Principal` before `enqueue`. The same owner and request id with the same fingerprint returns the stored receipt and does not dispatch again. The same id with a different fingerprint is `duplicate_conflict`. An expiry at or before the session clock is `expired` and does not play. Cancellation of a queued request is `cancelled` and `pump` will not play it.

`chess.play` takes game id, expected revision, operation id, and UCI. The actor is the dispatcher caller. A fresh legal move commits one revision and one event. The same actor, operation id, and payload returns the stored receipt with `replayed: true` and does not mutate or emit again. A different payload for that operation id is `duplicate_conflict`. A missing grant is `denied` and does not include another game's revision. A grant for the other side is `unauthorized`. A mismatched revision is `stale_revision`. `chess.new_game` requires the reset grant. `chess.state` and `chess.legal_moves` require inspect, play, or reset on that game.

Receipt phases are `accepted_at_edge`, `delivered_to_host`, `queued`, `running`, `waiting_for_permission`, and `terminal`. Terminal kinds are `succeeded`, `failed`, `cancelled`, `expired`, and `outcome_unknown`. The local simulator uses queued, running, and terminal. It does not pretend an edge or a phone accepted the request.

## What this proof does not cover

A visible chess board, `AssistantHarness` connector turns, phone browser automation, a no-window daemon, and a hosted runtime are unsupported. Later phases name those variants rather than marking them passed.
