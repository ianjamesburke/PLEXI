#!/usr/bin/env python3
"""Chess — SDK v3 state-backed canvas board."""

from __future__ import annotations

from chess_engine import FILES, START_FEN, Position, parse_sq, sq_name
from plexi_sdk import log, state
from plexi_sdk.effects import SetState, SetStatus, SetTitle
from plexi_sdk.events import KeyEvent
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

CANVAS_W = 480.0
CANVAS_H = 480.0
CELL = 54.0
BOARD = CELL * 8

PIECE_GLYPHS = {
    "K": "♔",
    "Q": "♕",
    "R": "♖",
    "B": "♗",
    "N": "♘",
    "P": "♙",
    "k": "♚",
    "q": "♛",
    "r": "♜",
    "b": "♝",
    "n": "♞",
    "p": "♟",
}


def _initial() -> dict:
    return {
        "fen": START_FEN,
        "cursor": [4, 1],
        "selected": None,
        "status": "White to move",
        "last_move": "",
    }


def _game() -> dict:
    data = _initial()
    for key, default in data.items():
        data[key] = state.get(key, default)
    data["cursor"] = list(data["cursor"])
    data["selected"] = list(data["selected"]) if data["selected"] is not None else None
    data["fen"] = str(data["fen"])
    data["status"] = str(data["status"])
    data["last_move"] = str(data["last_move"])
    return data


def init(size, args) -> list:
    data = _game()
    missing = {
        key: value for key, value in _initial().items() if state.get(key, None) is None
    }
    effects: list = [SetTitle("Chess"), SetStatus(data["status"])]
    if missing:
        effects.append(SetState(missing))
    log.info("chess: SDK v3 canvas initialized")
    return effects


def update(event) -> list:
    if not isinstance(event, KeyEvent) or not event.pressed:
        return []
    data = _game()
    key = event.key
    if key in ("n", "N"):
        data = _initial()
        log.info("chess: new game")
        return _set(data)
    if key in ("escape", "Escape"):
        data["selected"] = None
        return _set(data)
    if key in ("left", "ArrowLeft", "h"):
        data["cursor"][0] = max(0, data["cursor"][0] - 1)
        return _set(data)
    if key in ("right", "ArrowRight", "l"):
        data["cursor"][0] = min(7, data["cursor"][0] + 1)
        return _set(data)
    if key in ("up", "ArrowUp", "k"):
        data["cursor"][1] = min(7, data["cursor"][1] + 1)
        return _set(data)
    if key in ("down", "ArrowDown", "j"):
        data["cursor"][1] = max(0, data["cursor"][1] - 1)
        return _set(data)
    if key in ("enter", "return", "Enter", "space"):
        return _set(_select_or_move(data))
    return []


def _set(data: dict) -> list:
    return [SetState(data), SetStatus(data["status"])]


def _select_or_move(data: dict) -> dict:
    pos = _position_from_fen(data["fen"])
    cursor = tuple(data["cursor"])
    if data["selected"] is None:
        piece = pos.board.get(cursor)
        if not piece:
            data["status"] = f"No piece on {sq_name(cursor)}"
            return data
        if piece.isupper() != pos.white_to_move:
            data["status"] = f"{'White' if pos.white_to_move else 'Black'} to move"
            return data
        data["selected"] = list(cursor)
        data["status"] = f"Selected {sq_name(cursor)}"
        return data

    selected = tuple(data["selected"])
    if selected == cursor:
        data["selected"] = None
        data["status"] = f"Cleared {sq_name(cursor)}"
        return data
    move_text = sq_name(selected) + sq_name(cursor)
    piece = pos.board.get(selected)
    if piece and piece.upper() == "P" and cursor[1] in (0, 7):
        move_text += "q"
    try:
        move = pos.make_move(move_text)
    except ValueError as exc:
        data["status"] = str(exc).split(" - ")[0].split(" — ")[0]
        data["selected"] = None
        return data
    data["fen"] = pos.fen()
    data["selected"] = None
    data["last_move"] = move.san
    result = pos.result()
    if result:
        data["status"] = f"Game over: {result}"
    else:
        side = "White" if pos.white_to_move else "Black"
        data["status"] = f"{move.san} played - {side} to move"
    log.info(f"chess: played {move.san}")
    return data


def _position_from_fen(fen: str) -> Position:
    fields = fen.split()
    board_part = fields[0] if fields else START_FEN.split()[0]
    side = fields[1] if len(fields) > 1 else "w"
    castling = fields[2] if len(fields) > 2 else "KQkq"
    ep = fields[3] if len(fields) > 3 else "-"
    pos = Position()
    pos.board.clear()
    for rank_index, row in enumerate(board_part.split("/")):
        rank = 7 - rank_index
        file_idx = 0
        for char in row:
            if char.isdigit():
                file_idx += int(char)
            else:
                pos.board[(file_idx, rank)] = char
                file_idx += 1
    pos.white_to_move = side == "w"
    pos.castling = set() if castling == "-" else set(castling)
    pos.en_passant = None if ep == "-" else parse_sq(ep)
    pos.history = []
    return pos


def view():
    data = _game()
    pos = _position_from_fen(data["fen"])
    side = "White" if pos.white_to_move else "Black"
    return Column(
        [
            AppBar("Chess", f"{side} to move"),
            Canvas(_draw_board(pos, data), width=CANVAS_W, height=CANVAS_H, grow=True),
            Text(data["status"], size=12.0),
            Text(data["last_move"] or data["fen"], size=11.0, truncate=True),
            Spacer(6.0),
            FooterKeys([("arrows", "move"), ("enter", "select"), ("n", "new")]),
        ],
        padding=0,
        gap=4.0,
        grow=True,
    )


def _draw_board(pos: Position, data: dict) -> list:
    ox = (CANVAS_W - BOARD) / 2
    oy = (CANVAS_H - BOARD) / 2
    commands: list = [CanvasRect(0, 0, CANVAS_W, CANVAS_H, "#11111b")]
    selected = tuple(data["selected"]) if data["selected"] is not None else None
    cursor = tuple(data["cursor"])
    for rank in range(8):
        for file_idx in range(8):
            square = (file_idx, rank)
            x = ox + file_idx * CELL
            y = oy + (7 - rank) * CELL
            light = (file_idx + rank) % 2 == 1
            fill = "#a6adc8" if light else "#45475a"
            if square == selected:
                fill = "#f9e2af"
            elif square == cursor:
                fill = "#89b4fa"
            commands.append(CanvasRect(x, y, CELL, CELL, fill))
            piece = pos.board.get(square)
            if piece:
                color = "#f5f5f5" if piece.isupper() else "#11111b"
                commands.append(
                    CanvasText(
                        x + CELL / 2,
                        y + CELL / 2,
                        PIECE_GLYPHS[piece],
                        size=38.0,
                        color=color,
                        align="center_center",
                    )
                )
    for idx, file_name in enumerate(FILES):
        commands.append(
            CanvasText(
                ox + idx * CELL + 4,
                oy + BOARD - 5,
                file_name,
                size=10.0,
                color="#11111b",
            )
        )
    return commands
