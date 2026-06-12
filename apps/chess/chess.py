#!/usr/bin/env python3
"""Chess — agent-platform proof of concept (docs/prm/chess-agent-poc.md).

The app owns the game state and is the legality authority. It:
  - declares the five event streams (game.started, turn.ready, move.played,
    move.undone, game.ended) and emits them with revision + rollback metadata
    so every move creates a host undo checkpoint;
  - exposes the five chess.* tools (current_state, legal_moves, make_move,
    undo_move, resign) so a broker-gated agent can play;
  - answers host rollback verification and applies verified rollbacks
    (the host undo timeline path) — chess.undo_move is the app-owned path.
"""

import uuid

from chess_engine import Move, Position
from plexi_sdk import App
from plexi_sdk.ui import AppBar, Column, Component, FooterKeys, Label, TextInput

PIECE_GLYPHS = {
    "K": "♔", "Q": "♕", "R": "♖", "B": "♗", "N": "♘", "P": "♙",
    "k": "♚", "q": "♛", "r": "♜", "b": "♝", "n": "♞", "p": "♟",
}

EVENT_STREAMS = [
    {"name": "game.started", "schema": {"type": "object"},
     "description": "A new game began."},
    {"name": "turn.ready", "schema": {"type": "object"},
     "description": "A side is to move; payload carries fen + legal moves."},
    {"name": "move.played", "schema": {"type": "object"},
     "description": "A move was applied. Reversible (rollback_token)."},
    {"name": "move.undone", "schema": {"type": "object"},
     "description": "The last move was undone."},
    {"name": "game.ended", "schema": {"type": "object"},
     "description": "The game ended (mate, stalemate, or resignation)."},
]


class _BoardCanvas(Component):
    """Grow-to-fill canvas rendering the 8x8 board with unicode pieces."""

    def __init__(self, app: "ChessApp") -> None:
        self._app = app

    def is_grow(self) -> bool:
        return True

    def measure(self, _avail_w: float) -> float:
        return 0.0

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        cell = max(24.0, min(w - 24.0, h - 8.0) / 8.0)
        ox = x + (w - 8 * cell) / 2
        oy = y + (h - 8 * cell) / 2
        for rank in range(8):
            for f in range(8):
                cx = ox + f * cell
                cy = oy + (7 - rank) * cell
                light = (f + rank) % 2 == 1
                ctx.rect(cx, cy, cell, cell,
                         fill=ctx.theme.muted if light else ctx.theme.bg_darkest)
                piece = self._app._pos.board.get((f, rank))
                if piece:
                    ctx.text(cx + cell / 2, cy + cell / 2, PIECE_GLYPHS[piece],
                             size=cell * 0.72,
                             color="#f5f5f5" if piece.isupper() else "#1e1e2e",
                             align="center_center")


class ChessApp(App):
    # ── Lifecycle ────────────────────────────────────────────────────────────

    def on_init(self) -> None:
        self._input = TextInput("chess-move", placeholder="Move (e.g. e4, Nf3, e2e4)…",
                                height=48.0)
        self.emit.declare_event_streams(EVENT_STREAMS)
        self._new_game()
        self._expose_tools()
        self.emit.info(f"chess: ready — game {self._game_id}")

    def _new_game(self) -> None:
        self._pos = Position.initial()
        self._game_id = f"game-{uuid.uuid4().hex[:8]}"
        self._status = ""
        self._result: "str | None" = None
        self.emit.emit_event(
            event="game.started", actor="user",
            summary=f"New chess game {self._game_id} started",
            resource_id=self._game_id, resource_scope="game",
            revision_after=self._revision(),
            payload={"game_id": self._game_id, "fen": self._pos.fen()},
        )
        self._emit_turn_ready()
        self.emit.info(f"chess: new game {self._game_id}")

    # ── Revisions / events ───────────────────────────────────────────────────

    def _revision(self) -> str:
        return f"rev-{len(self._pos.history)}"

    def _emit_turn_ready(self) -> None:
        if self._result is not None:
            return
        side = "white" if self._pos.white_to_move else "black"
        self.emit.emit_event(
            event="turn.ready", actor="app",
            summary=f"{side} to move in {self._game_id}",
            resource_id=self._game_id, resource_scope="game",
            revision_after=self._revision(),
            payload={
                "game_id": self._game_id,
                "side_to_move": side,
                "fen": self._pos.fen(),
                "legal_moves": [m.san for m in self._pos.legal_moves()],
            },
            suggested_trigger="conversation",
        )

    def _emit_move_played(self, move: Move, actor: str,
                          rev_before: str, actor_id: "str | None" = None) -> None:
        ply = len(self._pos.history)
        self.emit.emit_event(
            event="move.played", actor=actor, actor_id=actor_id,
            summary=f"{'White' if ply % 2 == 1 else 'Black'} played {move.san}",
            resource_id=self._game_id, resource_scope="game",
            revision_before=rev_before, revision_after=self._revision(),
            rollback_token=f"move-{ply}",
            changed_resources=[self._game_id],
            payload={"game_id": self._game_id, "san": move.san, "uci": move.uci,
                     "fen": self._pos.fen()},
        )

    def _finish_if_over(self, actor: str) -> None:
        result = self._pos.result()
        if result is None:
            self._emit_turn_ready()
            return
        self._result = result
        self._status = f"Game over: {result}"
        self.emit.emit_event(
            event="game.ended", actor=actor,
            summary=f"Game {self._game_id} ended {result}",
            resource_id=self._game_id, resource_scope="game",
            revision_after=self._revision(),
            payload={"game_id": self._game_id, "result": result},
        )
        self.emit.info(f"chess: game {self._game_id} ended {result}")

    # ── Core mutations (UI and tools share these) ────────────────────────────

    def _apply_move(self, notation: str, actor: str,
                    actor_id: "str | None" = None) -> dict:
        if self._result is not None:
            raise ValueError(f"game {self._game_id} is over ({self._result})")
        rev_before = self._revision()
        move = self._pos.make_move(notation)  # raises ValueError when illegal
        ply = len(self._pos.history)
        self._status = f"{move.san} played"
        self._emit_move_played(move, actor, rev_before, actor_id)
        self._finish_if_over(actor)
        self.emit.schedule_render()
        self.emit.info(f"chess: {actor} played {move.san} (rev {rev_before} -> {self._revision()})")
        return {
            "ok": True,
            "summary": f"{'White' if ply % 2 == 1 else 'Black'} played {move.san}",
            "revision_before": rev_before,
            "revision_after": self._revision(),
            "rollback_token": f"move-{ply}",
            "changed_resources": [self._game_id],
        }

    def _undo_last(self, actor: str) -> dict:
        rev_before = self._revision()
        move = self._pos.undo_move()  # raises ValueError when no history
        self._result = None
        self._status = f"undid {move.san}"
        self.emit.emit_event(
            event="move.undone", actor=actor,
            summary=f"Move {move.san} undone",
            resource_id=self._game_id, resource_scope="game",
            revision_before=rev_before, revision_after=self._revision(),
            payload={"game_id": self._game_id, "san": move.san,
                     "fen": self._pos.fen()},
        )
        self._emit_turn_ready()
        self.emit.schedule_render()
        self.emit.info(f"chess: {actor} undid {move.san}")
        return {"ok": True, "undone": move.san,
                "revision_before": rev_before,
                "revision_after": self._revision()}

    def _state_payload(self) -> dict:
        return {
            "game_id": self._game_id,
            "side_to_move": "white" if self._pos.white_to_move else "black",
            "fen": self._pos.fen(),
            "move_list": [rec["move"].san for rec in self._pos.history],
            "legal_moves": [m.san for m in self._pos.legal_moves()]
            if self._result is None else [],
            "result": self._result,
            "revision_id": self._revision(),
        }

    # ── Tools (docs/prm/chess-agent-poc.md "App Contract") ──────────────────

    def _expose_tools(self) -> None:
        @self.tool("chess.current_state",
                   description="Current chess game state: side to move, FEN, "
                               "move list, legal moves, result, revision id.",
                   schema={"type": "object", "properties": {}})
        def current_state(_args: dict) -> dict:
            return self._state_payload()

        @self.tool("chess.legal_moves",
                   description="Legal moves (SAN) for the side to move.",
                   schema={"type": "object", "properties": {}})
        def legal_moves(_args: dict) -> dict:
            return {"game_id": self._game_id,
                    "legal_moves": [m.san for m in self._pos.legal_moves()]
                    if self._result is None else []}

        @self.tool("chess.make_move",
                   description="Play a move. The app validates legality; an "
                               "illegal move returns an error.",
                   schema={
                       "type": "object",
                       "properties": {
                           "game_id": {"type": "string"},
                           "move": {"type": "string",
                                    "description": "Move in SAN (Nf6) or UCI (g8f6)."},
                           "notation": {"type": "string", "enum": ["san", "uci"]},
                       },
                       "required": ["game_id", "move"],
                   })
        def make_move(args: dict) -> dict:
            game_id = args.get("game_id", "")
            if game_id != self._game_id:
                return {"ok": False,
                        "error": f"unknown game_id {game_id!r} (current: {self._game_id})"}
            try:
                return self._apply_move(str(args.get("move", "")), actor="agent")
            except ValueError as exc:
                self.emit.warn(f"chess.make_move rejected: {exc}")
                return {"ok": False, "error": str(exc)}

        @self.tool("chess.undo_move",
                   description="Undo the last move (app-owned rollback path).",
                   schema={"type": "object",
                           "properties": {"game_id": {"type": "string"}},
                           "required": ["game_id"]})
        def undo_move(args: dict) -> dict:
            if args.get("game_id", "") != self._game_id:
                return {"ok": False, "error": f"unknown game_id (current: {self._game_id})"}
            try:
                return self._undo_last(actor="agent")
            except ValueError as exc:
                return {"ok": False, "error": str(exc)}

        @self.tool("chess.resign",
                   description="Resign the game for the side to move.",
                   schema={"type": "object",
                           "properties": {"game_id": {"type": "string"}},
                           "required": ["game_id"]})
        def resign(args: dict) -> dict:
            if args.get("game_id", "") != self._game_id:
                return {"ok": False, "error": f"unknown game_id (current: {self._game_id})"}
            if self._result is not None:
                return {"ok": False, "error": f"game already over ({self._result})"}
            self._result = "1-0" if not self._pos.white_to_move else "0-1"
            self._status = f"Resignation: {self._result}"
            self.emit.emit_event(
                event="game.ended", actor="agent",
                summary=f"Game {self._game_id} ended by resignation: {self._result}",
                resource_id=self._game_id, resource_scope="game",
                revision_after=self._revision(),
                payload={"game_id": self._game_id, "result": self._result,
                         "by": "resignation"},
            )
            self.emit.schedule_render()
            self.emit.info(f"chess: resignation -> {self._result}")
            return {"ok": True, "result": self._result}

        self.emit.info("chess: exposed chess.* tools")

    # ── Host undo timeline (rollback verify/apply) ───────────────────────────

    def on_rollback_verify(self, _checkpoint_id: str, resource_id: str,
                           _expected_revision: str) -> str:
        if resource_id != self._game_id:
            return ""  # stale checkpoint from a previous game — never matches
        return self._revision()

    def on_rollback_apply(self, checkpoint_id: str, resource_id: str,
                          rollback_token: str) -> None:
        # Token "move-N" identifies the move that created the checkpoint.
        # Verification guaranteed we are still at that move's revision_after,
        # so exactly one undo restores revision_before.
        if resource_id != self._game_id:
            self.emit.error(
                f"rollback_apply {checkpoint_id}: resource {resource_id!r} is not "
                f"the current game {self._game_id!r} — ignoring")
            return
        try:
            self._undo_last(actor="system")
            self.emit.info(f"chess: applied rollback {checkpoint_id} ({rollback_token})")
        except ValueError as exc:
            self.emit.error(f"rollback_apply {checkpoint_id} failed: {exc}")

    # ── UI ───────────────────────────────────────────────────────────────────

    def on_text_submitted(self, id: str, text: str) -> None:
        # The move input is always focused, so commands flow through it too:
        # "undo" / "new" / "resign" alongside SAN or UCI moves.
        if id != "chess-move":
            return
        text = text.strip()
        if not text:
            return
        if text.lower() == "undo":
            try:
                self._undo_last(actor="user")
            except ValueError as exc:
                self._status = str(exc)
                self.emit.schedule_render()
            return
        if text.lower() == "new":
            self._new_game()
            self.emit.schedule_render()
            return
        try:
            self._apply_move(text, actor="user")
        except ValueError as exc:
            self._status = str(exc).split(" — ")[0]
            self.emit.schedule_render()

    def view(self):
        side = "White" if self._pos.white_to_move else "Black"
        status = self._status or f"{side} to move"
        if self._result is not None:
            status = f"Game over: {self._result}"
        moves = " ".join(rec["move"].san for rec in self._pos.history[-12:])
        return Column([
            AppBar(title=f"Chess — {self._game_id}"),
            _BoardCanvas(self),
            Label(status, tone="caption"),
            Label(moves or "No moves yet.", tone="hint"),
            self._input,
            FooterKeys([
                ("Enter", "play move"),
                ("undo", "undo last"),
                ("new", "new game"),
            ]),
        ])


if __name__ == "__main__":
    ChessApp().run()
