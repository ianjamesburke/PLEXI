# Plexi Agent Platform Spec

Status: sequencing spec.
Parent: [`app-framework-marketplace.md`](app-framework-marketplace.md).
Last updated: 2026-06-11.

This spec links the rewrites needed for the host Assistant to become a real agent platform: unified permissions, undo history, app event subscriptions, and a chess proof of concept.

The product target is not "chat with an app nearby." The target is an agent that can sit in a Plexi pane, subscribe to app events with consent, call app tools with consent, write to the host timeline, and participate in rollback where the app exposes reversible state.

## Required Specs

Build in this order:

1. [`permissions-broker.md`](permissions-broker.md): generalize app permissions into one host broker for apps, agents, app connectors, event subscriptions, tools, model pointers, and undo operations.
2. [`undo-and-app-events.md`](undo-and-app-events.md): define host undo history, reversible app events, agent subscriptions, trigger modes, and context injection.
3. [`chess-agent-poc.md`](chess-agent-poc.md): prove the whole loop with a chess app and a chess-playing agent.
4. [`assistant-host-app.md`](assistant-host-app.md): host Assistant UI consumes the broker, timeline, event stream, agent registry, slash commands, and model routing.

Do not build the chess proof of concept before the permission broker. The point of the POC is not chess; it is proving that consent, app events, agent-triggered turns, mutating app tools, and undo all fit the same model.

## Core Decisions

- Agents are first-class permission actors.
- Apps own their state.
- The host owns permission decisions, subscriptions, audit, and undo timeline.
- Apps expose events and tools through typed contracts.
- Agents subscribe to app events only after host permission grants.
- State rollback is host-level undo history, not agent-owned app state.
- A subscription is not required for rollback. A reversible mutation event or reversible tool result is required.
- Conversation rewind is always available. State rollback is available only for checkpointed host/app changes.

## Another POC Candidate

After chess, the next small proof should be a Kanban board app:

- Events: `card.created`, `card.moved`, `card.completed`, `lane.changed`.
- Tools: `kanban.create_card`, `kanban.move_card`, `kanban.update_card`.
- Agent behavior: triage new cards, suggest next card, move cards after user approval.
- Undo: move-card and update-card operations are reversible.

Kanban is a better second example than typo diagnostics because the app state is simple and discrete. It still proves subscriptions, ambient assistance, tool calls, and undo without introducing text-diff complexity.

## Done When

- The permission broker can express grants for agents, apps, app connectors, event streams, model pointers, and undo operations.
- An app can expose a subscription event stream with schemas.
- The host can route selected app events into an agent context.
- The host can trigger an agent turn or ambient automation from an event.
- Mutating app tools can return reversible checkpoint metadata.
- The history surface can show conversation-only rewind and state rollback separately.
- The chess POC lets a user play against an agent through app events and `chess.make_move`.
