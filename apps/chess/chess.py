#!/usr/bin/env python3
"""Chess — one game instance that people and agents play through the same
domain operations.

Board clicks and `chess.play` tool calls both commit through
`chess_domain.play`, so revision checks, seat ownership, operation dedup, and
receipts are identical for a human and an agent. The game (moves, revision,
seats, receipts, event outbox) is this context's `[state]`; the cursor and
selection are process-local UI state.

Every committed move is published as `chess.move_committed` on the host event
bus. The event is written to the game's outbox in the same state value as the
move, emitted, and only then cleared, so a restart between commit and
publication re-emits it on the next launch. Consumers deduplicate by
`operation_id`/`revision_after`.
"""

from __future__ import annotations

import json
from typing import Any, Optional

import chess_domain as domain
from chess_engine import FILES, Position, sq_name
from plexi_sdk import log, state, tools
from plexi_sdk.effects import (
    DeclareEventStreams,
    EmitEvent,
    EventStreamDecl,
    PersistState,
    SetState,
    SetStatus,
    SetTitle,
)
from plexi_sdk.events import KeyEvent, MouseEvent, Resize, StateChanged
from plexi_sdk.ui import (
    AppBar,
    Canvas,
    CanvasRect,
    CanvasText,
    Column,
    FooterKeys,
    Spacer,
    Text,
)

DEFAULT_GAME_ID = "game-1"
MIN_CELL = 24.0
CHROME_RESERVE = 120.0

# Pane size from `init` and `Resize`; the board canvas fills what is left
# after the app bar and footer.
_pane_size = (480.0, 480.0)

PIECE_GLYPHS = {
    "K": "♔", "Q": "♕", "R": "♖", "B": "♗", "N": "♘", "P": "♙",
    "k": "♚", "q": "♛", "r": "♜", "b": "♝", "n": "♞", "p": "♟",
}

_EVENT_SCHEMA = json.dumps({
    "type": "object",
    "properties": {
        "game_id": {"type": "string"},
        "operation_id": {"type": "string"},
        "actor": {"type": "string"},
        "revision_after": {"type": "integer"},
        "side_to_move": {"type": "string"},
        "status": {"type": "string"},
    },
    "required": ["game_id", "operation_id", "actor", "revision_after"],
})


# ── State ───────────────────────────────────────────────────────────────────


def _game() -> Optional[dict]:
    game = state.get("game", None)
    return game if isinstance(game, dict) and game.get("schema") == domain.GAME_SCHEMA else None


def _ui() -> dict:
    selected = state.get("selected", None)
    return {
        "cursor": list(state.get("cursor", [4, 1])),
        "selected": list(selected) if selected is not None else None,
        "message": str(state.get("message", "") or ""),
    }


def _seats_from_args(args: Any) -> dict:
    """`--white=<actor>` / `--black=<actor>` launch arguments seat agents."""
    seats: dict = {}
    for arg in args or []:
        text = str(arg)
        for side in domain.SIDES:
            prefix = f"--{side}="
            if text.startswith(prefix) and text[len(prefix):]:
                seats[side] = text[len(prefix):]
    return seats


def _publish(game: dict) -> list:
    """Persist the game with its outbox, emit the outbox, then clear it."""
    drained, pending = domain.drain_outbox(game)
    effects: list = [PersistState({"game": game})]
    effects += [_emit(event) for event in pending]
    if pending:
        effects.append(PersistState({"game": drained}))
    return effects


def _emit(event: dict) -> EmitEvent:
    before = event.get("revision_before")
    if event["event"] == domain.MOVE_COMMITTED:
        summary = f"{event['actor']} played {event['san']} ({event['side']})"
    else:
        summary = f"new game {event['game_id']}"
    return EmitEvent(
        event=event["event"],
        actor="agent" if event["actor"].startswith("agent:") else "user",
        actor_id=event["actor"],
        summary=summary,
        resource_id=event["game_id"],
        resource_scope="chess.game",
        revision_before=None if before is None else str(before),
        revision_after=str(event["revision_after"]),
        caused_by=event["operation_id"],
        payload_json=json.dumps(event),
    )


def _status_text(game: dict) -> str:
    snap = domain.snapshot(game)
    if snap["status"] != "ongoing":
        return f"Game over: {snap['status']} {snap['result'] or ''}".strip()
    return f"rev {snap['revision']} · {snap['side_to_move'].title()} to move"


# ── Tools ───────────────────────────────────────────────────────────────────


def _require_game() -> dict:
    game = _game()
    if game is None:
        raise domain.ChessError("unknown_game", "no game in this instance")
    return game


@tools.tool("chess.state",
            "Current game: id, revision, FEN, side to move, status, seats, and move history.",
            read_only=True)
def _tool_state() -> dict:
    return domain.snapshot(_require_game())


@tools.tool("chess.legal_moves",
            "Legal UCI moves for the side to move. Fails if `revision` is not current.",
            {"game_id": str, "revision": int}, read_only=True)
def _tool_legal_moves(game_id: str, revision: int) -> dict:
    return domain.legal_moves(_require_game(), game_id, revision)


@tools.tool("chess.play",
            "Play one UCI move (e.g. e2e4, e7e8q) for the seat you hold. Requires the "
            "current revision and a unique operation_id; repeating an operation_id "
            "returns its original receipt without moving again.",
            {"game_id": str, "expected_revision": int, "operation_id": str, "move": str},
            caller=True)
def _tool_play(game_id: str, expected_revision: int, operation_id: str, move: str,
               caller_id: str) -> tools.Reply:
    outcome = domain.play(_require_game(), actor=caller_id, game_id=game_id,
                          expected_revision=expected_revision,
                          operation_id=operation_id, move=move)
    if outcome.duplicate:
        log.info(f"chess: duplicate operation {operation_id} from {caller_id}; no mutation")
        return tools.Reply(outcome.receipt)
    log.info(f"chess: {caller_id} played {outcome.receipt['move']} "
             f"rev {outcome.receipt['revision_before']}->{outcome.receipt['revision_after']}")
    effects = _publish(outcome.game)
    effects.append(SetStatus(_status_text(outcome.game)))
    return tools.Reply(outcome.receipt, effects)


@tools.tool("chess.new_game",
            "Start a new game with a new id, replacing the current one. Privileged: "
            "only the local user may reset.",
            {"game_id": str, "white": str, "black": str}, caller=True)
def _tool_new_game(game_id: str, white: str, black: str, caller_id: str) -> tools.Reply:
    game = domain.start_new_game(_game(), actor=caller_id, game_id=game_id,
                                 seats={"white": white or None, "black": black or None})
    return tools.Reply(domain.snapshot(game), _publish(game))


# ── Lifecycle ───────────────────────────────────────────────────────────────


def init(size, args) -> list:
    global _pane_size
    if size:
        _pane_size = (float(size[0]), float(size[1]))
    effects: list = [
        SetTitle("Chess"),
        DeclareEventStreams([
            EventStreamDecl(domain.MOVE_COMMITTED, _EVENT_SCHEMA,
                            "A move committed to the game, with its receipt."),
            EventStreamDecl(domain.GAME_STARTED, _EVENT_SCHEMA,
                            "A new game replaced the previous one."),
        ]),
        tools.expose(),
    ]
    game = _game()
    seats = _seats_from_args(args)
    if game is None:
        game = domain.new_game(DEFAULT_GAME_ID, seats)
        log.info(f"chess: new game {game['game_id']} seats={game['seats']}")
        effects += _publish(game)
    elif game.get("outbox"):
        # Committed before the last shutdown but never confirmed published.
        log.info(f"chess: re-publishing {len(game['outbox'])} pending event(s) after restart")
        effects += _publish(game)
    elif seats and game["revision"] == 0 and seats != {
            k: v for k, v in game["seats"].items() if v}:
        # Launch-time seating applies only to an unplayed game.
        game = domain.new_game(game["game_id"], seats, game["start_fen"])
        effects += _publish(game)
    log.info(f"chess: ready game={game['game_id']} rev={game['revision']}")
    effects.append(SetStatus(_status_text(game)))
    return effects


def update(event) -> list:
    global _pane_size
    handled = tools.dispatch(event)
    if handled is not None:
        return handled

    if isinstance(event, StateChanged):
        if event.error:
            log.warn(f"chess: state file error: {event.error}")
            return [SetStatus(f"chess: {event.error}")]
        game = _game()
        return [SetStatus(_status_text(game))] if game else []

    if isinstance(event, Resize):
        _pane_size = (float(event.width), float(event.height))
        return []
    if isinstance(event, MouseEvent):
        return _handle_mouse(event)
    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    ui = _ui()
    key = event.key
    if key == "n":
        game = _game()
        new_id = _next_game_id(game)
        fresh = domain.start_new_game(game, actor=domain.LOCAL_USER, game_id=new_id,
                                      seats=game["seats"] if game else None)
        log.info(f"chess: new game {new_id} from keyboard")
        return _publish(fresh) + _set_ui(dict(ui, selected=None, message="New game"), fresh)
    if key == "escape":
        return _set_ui(dict(ui, selected=None))
    moves = {"left": (-1, 0), "h": (-1, 0), "right": (1, 0), "l": (1, 0),
             "up": (0, 1), "k": (0, 1), "down": (0, -1), "j": (0, -1)}
    if key in moves:
        dx, dy = moves[key]
        ui["cursor"] = [max(0, min(7, ui["cursor"][0] + dx)), max(0, min(7, ui["cursor"][1] + dy))]
        return _set_ui(ui)
    if key in ("enter", "return", "space"):
        return _select_or_move(ui)
    return []


def _next_game_id(game: Optional[dict]) -> str:
    suffix = game["game_id"].rsplit("-", 1)[-1] if game else ""
    return f"game-{int(suffix) + 1}" if suffix.isdigit() else DEFAULT_GAME_ID


def _handle_mouse(event: MouseEvent) -> list:
    """Click a piece, then click its destination."""
    if not event.pressed or event.button not in (None, "left", "primary"):
        return []
    square = _square_at(event.x, event.y)
    if square is None:
        return []
    return _select_or_move(dict(_ui(), cursor=list(square)))


def _set_ui(ui: dict, game: Optional[dict] = None) -> list:
    game = game or _game()
    effects: list = [SetState(ui)]
    if game:
        effects.append(SetStatus(ui["message"] or _status_text(game)))
    return effects


def _select_or_move(ui: dict) -> list:
    game = _game()
    if game is None:
        return _set_ui(dict(ui, message="No game"))
    pos = domain.position(game)
    cursor = tuple(ui["cursor"])
    if ui["selected"] is None:
        piece = pos.board.get(cursor)
        if not piece:
            return _set_ui(dict(ui, message=f"No piece on {sq_name(cursor)}"))
        if piece.isupper() != pos.white_to_move:
            return _set_ui(dict(ui, message=f"{domain.side_to_move(pos).title()} to move"))
        return _set_ui(dict(ui, selected=list(cursor), message=f"Selected {sq_name(cursor)}"))

    selected = tuple(ui["selected"])
    if selected == cursor:
        return _set_ui(dict(ui, selected=None, message=""))
    uci = sq_name(selected) + sq_name(cursor)
    piece = pos.board.get(selected)
    if piece and piece.upper() == "P" and cursor[1] in (0, 7):
        uci += "q"
    try:
        # The board commits through the same operation as `chess.play`.
        outcome = domain.play(game, actor=domain.LOCAL_USER, game_id=game["game_id"],
                              expected_revision=game["revision"],
                              operation_id=f"ui:{game['game_id']}:{game['revision']}:{uci}",
                              move=uci)
    except domain.ChessError as err:
        return _set_ui(dict(ui, selected=None, message=err.message))
    log.info(f"chess: board move {uci} rev {outcome.receipt['revision_after']}")
    return _publish(outcome.game) + _set_ui(dict(ui, selected=None, message=""), outcome.game)


# ── View ────────────────────────────────────────────────────────────────────


def view():
    game = _game()
    ui = _ui()
    w, h = _canvas_size()
    if game is None:
        return Column([AppBar("Chess", "No game"), Text("Starting…")], grow=True)
    snap = domain.snapshot(game)
    pos = domain.position(game)
    last = snap["moves"][-1] if snap["moves"] else None
    subtitle = (f"{snap['side_to_move'].title()} to move" if snap["status"] == "ongoing"
                else f"{snap['status']} {snap['result'] or ''}".strip())
    return Column(
        [
            AppBar("Chess", f"{snap['game_id']} · rev {snap['revision']} · {subtitle}"),
            Canvas(_draw_board(pos, ui, w, h), width=w, height=h, grow=True),
            Text(ui["message"] or (f"Last: {last['san']} by {last['actor']}" if last
                                   else "No moves yet"), size=12.0),
            Text(f"White: {snap['seats']['white'] or 'you'} · Black: "
                 f"{snap['seats']['black'] or 'you'}", size=11.0, truncate=True),
            Spacer(6.0),
            FooterKeys([("click", "move"), ("arrows", "cursor"), ("enter", "select"),
                        ("n", "new game")]),
        ],
        padding=0,
        gap=4.0,
        grow=True,
    )


def _canvas_size() -> tuple[float, float]:
    w, h = _pane_size
    return max(MIN_CELL * 8, float(w)), max(MIN_CELL * 8, float(h) - CHROME_RESERVE)


def _board_geometry(w: float | None = None, h: float | None = None) -> tuple[float, float, float, float]:
    w, h = (w, h) if w is not None and h is not None else _canvas_size()
    board = max(MIN_CELL * 8, min(w - 24.0, h - 24.0))
    board = min(board, w, h)
    cell = board / 8.0
    return (w - board) / 2.0, (h - board) / 2.0, cell, board


def _square_at(x: float, y: float) -> tuple[int, int] | None:
    ox, oy, cell, board = _board_geometry()
    if x < ox or x >= ox + board or y < oy or y >= oy + board:
        return None
    file_idx = int((x - ox) // cell)
    rank = 7 - int((y - oy) // cell)
    return max(0, min(7, file_idx)), max(0, min(7, rank))


def _draw_board(pos: Position, ui: dict, w: float, h: float) -> list:
    ox, oy, cell, board = _board_geometry(w, h)
    commands: list = [CanvasRect(0, 0, w, h, "#11111b")]
    selected = tuple(ui["selected"]) if ui["selected"] is not None else None
    cursor = tuple(ui["cursor"])
    for rank in range(8):
        for file_idx in range(8):
            square = (file_idx, rank)
            x = ox + file_idx * cell
            y = oy + (7 - rank) * cell
            fill = "#a6adc8" if (file_idx + rank) % 2 == 1 else "#45475a"
            if square == selected:
                fill = "#f9e2af"
            elif square == cursor:
                fill = "#89b4fa"
            commands.append(CanvasRect(x, y, cell, cell, fill))
            piece = pos.board.get(square)
            if piece:
                commands.append(CanvasText(
                    x + cell / 2, y + cell / 2, PIECE_GLYPHS[piece], size=cell * 0.7,
                    color="#f5f5f5" if piece.isupper() else "#11111b",
                    align="center_center",
                ))
    for idx, file_name in enumerate(FILES):
        commands.append(CanvasText(ox + idx * cell + 4, oy + board - 5, file_name,
                                   size=max(8.0, cell * 0.18), color="#11111b"))
    return commands
