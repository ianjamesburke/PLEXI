# Undo History And App Events Spec

Status: architecture spec.
Parent: [`agent-platform.md`](agent-platform.md).
Last updated: 2026-06-11.

This spec defines two related host systems:

- App events: semantic app state changes the host can route to agents.
- Undo history: reversible state checkpoints the host can roll back.

They are related, but not the same.

## Core Rule

Apps own state. The host owns the timeline. Agents never own app state.

A subscription is not required for rollback. A reversible mutation is required for rollback.

An app event can do three things:

1. Enter the host timeline.
2. Enter an agent's context if a subscription grant allows it.
3. Add an undo checkpoint if the event or tool result includes rollback metadata.

## App Event Contract

Apps may expose named event streams. Event names are app-defined but must be declared with schemas.

Example:

```json
{
  "event": "move.played",
  "actor": "user",
  "summary": "White played e4",
  "state_ref": "chess://game/abc/rev/13",
  "payload": {
    "san": "e4",
    "fen": "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
  },
  "revision_before": "rev-12",
  "revision_after": "rev-13",
  "rollback_token": "undo-abc"
}
```

Required fields:

- event name
- actor: user, agent, app, system
- summary
- resource id: document, game, pane, or app instance
- revision after

Optional fields:

- structured payload
- state ref
- revision before
- rollback token
- changed resources
- suggested trigger mode

## Subscriptions

An agent can subscribe to an app's event stream only through the host broker.

Subscription controls:

- payload: off, summary, full, state-ref
- trigger: never, conversation, ambient, ask
- event names
- app package identity
- pane/document/game scope
- duration

Mode meanings:

- `never`: record event in timeline only.
- `conversation`: inject event into agent context and trigger a visible agent turn.
- `ambient`: run a bounded tool workflow without a visible chat turn; show compact activity.
- `ask`: prompt before triggering.

The app developer defines events. The agent developer defines which events the agent wants. The user grants or denies.

## Undo Timeline

The host undo timeline records reversible mutations from:

- agent connector tool calls
- user app interactions when the app emits reversible events
- host-owned operations such as pane open/close
- file edits done through host-mediated file tools

It does not promise to roll back:

- arbitrary terminal side effects
- network side effects
- private app mutations that were not emitted to the host
- external file edits not made through host tools

Undo checkpoint metadata:

```text
checkpoint_id
actor_type
actor_id
app_id
pane_id
resource_scope
resource_id
revision_before
revision_after
rollback_token
changed_resources
summary
created_at
```

Before rollback, the host asks the app whether the current revision still matches `revision_after`. If not, rollback is blocked or enters conflict resolution.

## Conversation Rewind vs State Rollback

Conversation rewind is always available. It changes what future agent turns see.

State rollback is available only when checkpoints exist.

History UI must show:

- conversation only
- reversible state
- partial rollback
- not reversible
- conflict

The user chooses explicitly:

- rewind conversation only
- rewind conversation and reversible state
- inspect changes
- fork from here

## Ambient Automation

Ambient automation is an event-triggered workflow that calls tools without a visible conversational turn. It still writes to the Assistant timeline and audit log.

Example:

```text
Book Agent fixed 6 typos on page 1.
View diff · Undo · Settings
```

Ambient writes require stronger grants than context-only subscriptions. If rollback metadata is unavailable, ambient writes should default to ask.

## Done When

- Apps can declare event streams and payload schemas.
- The host can record app events in the timeline.
- The host can route events to subscribed agents.
- The host can trigger conversation turns or ambient workflows from events.
- Mutating app tools can return undo metadata.
- User app interactions can be rollback-capable when the app emits reversible events.
- The history surface can rewind conversation state separately from state rollback.
