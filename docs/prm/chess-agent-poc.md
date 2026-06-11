# Chess Agent Proof Of Concept

Status: proof-of-concept spec.
Parent: [`agent-platform.md`](agent-platform.md).
Last updated: 2026-06-11.

The chess proof of concept ties together agents, app events, subscriptions, mutating app tools, permission grants, and undo history.

The user should be able to play a chess game against a Plexi agent. The agent receives turn events, chooses legal moves, calls the chess app's move tool, and writes a visible response into the Assistant pane.

## Why Chess

Chess is a clean first proof:

- State is discrete.
- Legal actions are enumerable.
- Mutations are reversible.
- The app can expose full state without privacy complexity.
- Agent and user turns are easy to distinguish.
- Undo is a native domain concept.

## App Contract

The chess app exposes event streams:

- `game.started`
- `turn.ready`
- `move.played`
- `move.undone`
- `game.ended`

The chess app exposes tools:

- `chess.current_state`
- `chess.legal_moves`
- `chess.make_move`
- `chess.undo_move`
- `chess.resign`

`chess.current_state` returns:

```text
game id
side to move
FEN
move list
legal moves
result, if game ended
revision id
```

`chess.make_move` input:

```json
{
  "game_id": "game-123",
  "move": "Nf6",
  "notation": "san"
}
```

`chess.make_move` output:

```json
{
  "ok": true,
  "summary": "Black played Nf6",
  "revision_before": "rev-12",
  "revision_after": "rev-13",
  "rollback_token": "move-13",
  "changed_resources": ["game-123"]
}
```

The app validates legal moves. The agent may suggest a move, but the app is the authority.

## Agent Contract

Agent files:

```text
<workspace>/<workspace_channel_dir>/agents/chess-opponent/
  AGENT.md
  settings.toml
```

`AGENT.md` contains the agent's role in prose:

```md
You are a chess opponent inside Plexi.
Play legal moves only.
Use chess.legal_moves before moving unless the current event already includes legal moves.
When it is your turn, explain your move briefly, then call chess.make_move.
Do not coach the user unless asked.
Do not undo moves unless the user asks.
```

That prose is model guidance, not enforcement. The app validates moves, and the host permission broker gates tools.

`settings.toml` requests subscriptions and tools:

```toml
[agent]
id = "chess-opponent"
display_name = "Chess Opponent"
default_tier = "medium"

[permissions]
default_posture = "review"

allow = [
  "app.chess.current_state",
  "app.chess.legal_moves",
]

ask = [
  "app.chess.make_move",
  "app.chess.undo_move",
]

[[subscriptions]]
app = "chess"
events = ["game.started", "turn.ready", "move.played", "move.undone", "game.ended"]
payload = "full"
trigger = "conversation"
default = "ask"
```

The settings file requests behavior. The permission broker stores what the user allowed.

## User Flow

1. User opens Chess app.
2. User opens the host Assistant pane and selects Chess Opponent.
3. Agent asks to subscribe to Chess events for this game.
4. Host shows a grant sheet:
   - agent: Chess Opponent
   - app: Chess
   - events: game and move events
   - payload: full
   - trigger: conversation
   - tools: current state, legal moves, make move
   - scope: this game
5. User grants for this game/session.
6. User makes a move in Chess.
7. Chess emits `move.played` and then `turn.ready`.
8. Host injects the event into Chess Opponent's context and triggers a turn.
9. Agent responds in text and calls `chess.make_move`.
10. Chess validates and applies the move.
11. Host records the move in conversation history, audit, and undo timeline.

## Undo

Every move creates a checkpoint. User moves and agent moves are both rollback-capable if the app emits reversible events.

Rollback rules:

- Conversation rewind can return to any prior turn.
- State rollback can undo moves only when the current game revision matches the checkpoint.
- If the user or agent made later moves, rollback must show the affected move range.
- `chess.undo_move` is the app-owned rollback path.

The history surface should show:

```text
Turn 8
User: e4
Agent: ...c5
State: reversible, game rev-12 -> rev-14
```

## Permission Requirements

Required broker targets:

- `app_event_stream:chess.game.started`
- `app_event_stream:chess.turn.ready`
- `app_event_stream:chess.move.played`
- `app_connector:chess.current_state`
- `app_connector:chess.legal_moves`
- `app_connector:chess.make_move`
- `undo_checkpoint:chess.game`

The agent cannot:

- subscribe to events without a grant
- make a move without a grant
- change its subscription trigger mode without a grant
- undo moves without a grant
- bypass app legal-move validation

## Done When

- Chess app declares events and tools.
- Host permission broker can grant event subscription and move tool access to Chess Opponent.
- Chess Opponent appears in the Assistant agent palette.
- User can grant the agent access for one game.
- User makes a move; agent receives the event and makes a legal reply move.
- Both moves appear in Assistant history.
- Undo history can roll back reversible chess moves.
- Denying the subscription prevents the agent from seeing moves.
- Denying `chess.make_move` lets the agent comment but not move.
