"""Chess domain operations — the single mutation path for board, UI, and tools.

Every move, whether it comes from a click on the board or from a
`chess.play` tool call, goes through `play()`. A game is a plain dict (so it
persists as app state verbatim) holding the starting position, the committed
move list, a monotonically increasing revision, seat assignments, the receipt
for every accepted operation, and an outbox of events committed with the move
but not yet confirmed published.

Authorization here is the app's own resource policy, applied *after* the host
has already admitted the call (reference monitor + grants). The host stamps the
caller identity; this module only decides whether that identity holds the seat
for the side to move.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Optional

from chess_engine import START_FEN, Position

GAME_SCHEMA = 1
MOVE_COMMITTED = "chess.move_committed"
GAME_STARTED = "chess.game_started"

# The local human at the keyboard. Board clicks act as this actor; it may
# move either side and is the only actor allowed to start a new game.
LOCAL_USER = "user"
SIDES = ("white", "black")


class ChessError(Exception):
    """A rejected operation. `code` is stable; nothing was mutated."""

    def __init__(self, code: str, message: str, **detail: Any) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.detail = detail

    def as_dict(self) -> dict:
        return {"code": self.code, "message": self.message, **self.detail}

    def __str__(self) -> str:
        return f"{self.code}: {self.message}"


@dataclass
class PlayOutcome:
    game: dict
    receipt: dict
    # Events committed by this operation; empty for a deduplicated replay.
    events: list = field(default_factory=list)
    duplicate: bool = False


def new_game(
    game_id: str,
    seats: Optional[dict] = None,
    start_fen: str = START_FEN,
) -> dict:
    """A fresh game at revision 0. `seats` maps "white"/"black" to the actor
    id allowed to move that side (None leaves a seat to the local user)."""
    if not game_id:
        raise ChessError("invalid_argument", "game_id is required")
    Position.from_fen(start_fen)  # validate before committing anything
    seats = dict(seats or {})
    for side in seats:
        if side not in SIDES:
            raise ChessError("invalid_argument", f"unknown seat {side!r}")
    game = {
        "schema": GAME_SCHEMA,
        "game_id": game_id,
        "start_fen": start_fen,
        "moves": [],
        "revision": 0,
        "seats": {side: seats.get(side) for side in SIDES},
        "receipts": {},
        "outbox": [],
    }
    game["outbox"].append(_started_event(game))
    return game


def position(game: dict) -> Position:
    """Rebuild the position by replaying committed moves from the start."""
    pos = Position.from_fen(game["start_fen"])
    for record in game["moves"]:
        pos.make_move(record["uci"])
    return pos


def side_to_move(pos: Position) -> str:
    return "white" if pos.white_to_move else "black"


def status(pos: Position) -> str:
    """`ongoing`, `checkmate`, or `stalemate`."""
    if pos.legal_moves_exist():
        return "ongoing"
    return "checkmate" if pos.in_check(pos.white_to_move) else "stalemate"


def snapshot(game: dict) -> dict:
    """The authorized semantic state exposed by `chess.state`."""
    pos = position(game)
    st = status(pos)
    return {
        "game_id": game["game_id"],
        "revision": game["revision"],
        "fen": pos.fen(),
        "side_to_move": side_to_move(pos),
        "status": st,
        "result": pos.result(),
        "in_check": pos.in_check(pos.white_to_move),
        "seats": dict(game["seats"]),
        "moves": [
            {k: m[k] for k in ("revision", "uci", "san", "actor", "operation_id")}
            for m in game["moves"]
        ],
    }


def legal_moves(game: dict, game_id: str, revision: int) -> dict:
    _check_game(game, game_id)
    _check_revision(game, revision)
    pos = position(game)
    return {
        "game_id": game["game_id"],
        "revision": game["revision"],
        "side_to_move": side_to_move(pos),
        "moves": sorted(m.uci for m in pos.legal_moves()),
    }


def play(
    game: dict,
    *,
    actor: str,
    game_id: str,
    expected_revision: int,
    operation_id: str,
    move: str,
) -> PlayOutcome:
    """Validate and commit one move. Raises ChessError without mutating.

    Order of checks is part of the contract: the game must match, a repeated
    operation id returns its original receipt (or conflicts), then side
    ownership, revision, and legality are checked against current state."""
    if not actor:
        raise ChessError("unauthorized", "no caller identity")
    if not operation_id:
        raise ChessError("invalid_argument", "operation_id is required")
    _check_game(game, game_id)
    uci = move.strip().lower()

    prior = game["receipts"].get(operation_id)
    if prior is not None:
        same = (
            prior["actor"] == actor
            and prior["move"] == uci
            and prior["revision_before"] == expected_revision
        )
        if not same:
            raise ChessError(
                "operation_conflict",
                f"operation {operation_id!r} was already used for a different move",
                current_revision=game["revision"],
            )
        return PlayOutcome(game=game, receipt=dict(prior, duplicate=True), duplicate=True)

    pos = position(game)
    st = status(pos)
    if st != "ongoing":
        raise ChessError("game_over", f"game is over ({st})", current_revision=game["revision"])

    side = side_to_move(pos)
    _authorize(game, actor, side)
    _check_revision(game, expected_revision)

    try:
        played = pos.make_move(uci)
    except ValueError:
        raise ChessError(
            "illegal_move",
            f"{uci!r} is not legal for {side} at revision {game['revision']}",
            current_revision=game["revision"],
        ) from None
    if played.uci != uci:
        # SAN or a promotion-less pawn push matched; require canonical UCI so
        # the receipt and dedup key name exactly what was committed.
        raise ChessError(
            "illegal_move",
            f"use UCI notation including promotion; did you mean {played.uci!r}?",
            current_revision=game["revision"],
        )

    after = status(pos)
    revision_after = game["revision"] + 1
    receipt = {
        "game_id": game["game_id"],
        "operation_id": operation_id,
        "actor": actor,
        "side": side,
        "move": uci,
        "san": played.san,
        "revision_before": game["revision"],
        "revision_after": revision_after,
        "side_to_move": side_to_move(pos),
        "status": after,
        "result": pos.result(),
        "fen": pos.fen(),
    }
    event = {"event": MOVE_COMMITTED, **receipt}
    committed = dict(game)
    committed["moves"] = game["moves"] + [{
        "revision": revision_after,
        "uci": uci,
        "san": played.san,
        "actor": actor,
        "operation_id": operation_id,
    }]
    committed["revision"] = revision_after
    committed["receipts"] = dict(game["receipts"], **{operation_id: receipt})
    committed["outbox"] = list(game.get("outbox", [])) + [event]
    return PlayOutcome(game=committed, receipt=dict(receipt, duplicate=False), events=[event])


def start_new_game(
    game: Optional[dict],
    *,
    actor: str,
    game_id: str,
    seats: Optional[dict] = None,
    start_fen: str = START_FEN,
) -> dict:
    """Privileged reset. Only the local user may replace the current game; a
    new id invalidates every reference to the old one."""
    if actor != LOCAL_USER:
        raise ChessError("unauthorized", f"{actor} may not start a new game")
    if game is not None and game.get("game_id") == game_id:
        raise ChessError("invalid_argument", "a new game needs a new game_id")
    return new_game(game_id, seats, start_fen)


def drain_outbox(game: dict) -> tuple[dict, list]:
    """Mark every outbox event as handed to the publisher."""
    return dict(game, outbox=[]), list(game.get("outbox", []))


def _started_event(game: dict) -> dict:
    pos = position(game)
    return {
        "event": GAME_STARTED,
        "game_id": game["game_id"],
        "revision_before": None,
        "revision_after": game["revision"],
        "seats": dict(game["seats"]),
        "side_to_move": side_to_move(pos),
        "status": status(pos),
        "fen": pos.fen(),
        "operation_id": f"new:{game['game_id']}",
        "actor": LOCAL_USER,
    }


def _check_game(game: dict, game_id: str) -> None:
    if game.get("game_id") != game_id:
        # Do not disclose which game exists; just refuse the reference.
        raise ChessError("unknown_game", f"no game {game_id!r} in this instance")


def _check_revision(game: dict, revision: int) -> None:
    if revision != game["revision"]:
        raise ChessError(
            "stale_revision",
            f"expected revision {revision} but the game is at {game['revision']}",
            current_revision=game["revision"],
        )


def _authorize(game: dict, actor: str, side: str) -> None:
    if actor == LOCAL_USER:
        return
    holder = game["seats"].get(side)
    if holder == actor:
        return
    other = "black" if side == "white" else "white"
    if game["seats"].get(other) == actor:
        raise ChessError(
            "wrong_side",
            f"{actor} holds {other}; it is {side}'s move",
            current_revision=game["revision"],
        )
    raise ChessError("unauthorized", f"{actor} holds no seat in this game")
