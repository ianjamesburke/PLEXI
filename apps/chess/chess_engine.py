"""Pure-python chess move legality engine for the Plexi chess app.

No dependencies. Implements full legality: piece movement, check detection,
castling, en passant, promotion, SAN + UCI notation, checkmate/stalemate,
and the 50-move and threefold-repetition draw observations are intentionally
omitted (out of scope for the POC — games end on mate, stalemate, or
resignation).

Board representation: dict from (file, rank) 0-based tuples to piece chars.
White pieces are uppercase ("PNBRQK"), black lowercase.
"""

from __future__ import annotations

from dataclasses import dataclass, field

FILES = "abcdefgh"
START_FEN = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"

KNIGHT_DELTAS = ((1, 2), (2, 1), (2, -1), (1, -2), (-1, -2), (-2, -1), (-2, 1), (-1, 2))
KING_DELTAS = ((1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1))
BISHOP_DIRS = ((1, 1), (1, -1), (-1, 1), (-1, -1))
ROOK_DIRS = ((1, 0), (-1, 0), (0, 1), (0, -1))


def sq_name(sq: tuple[int, int]) -> str:
    return f"{FILES[sq[0]]}{sq[1] + 1}"


def parse_sq(name: str) -> tuple[int, int]:
    return (FILES.index(name[0]), int(name[1]) - 1)


@dataclass
class Move:
    src: tuple[int, int]
    dst: tuple[int, int]
    piece: str
    capture: bool = False
    promotion: str | None = None  # "Q" | "R" | "B" | "N" (case-insensitive in)
    is_castle_kingside: bool = False
    is_castle_queenside: bool = False
    is_en_passant: bool = False
    san: str = ""  # filled by legal_moves()

    @property
    def uci(self) -> str:
        promo = self.promotion.lower() if self.promotion else ""
        return f"{sq_name(self.src)}{sq_name(self.dst)}{promo}"


@dataclass
class Position:
    """Mutable game position with move history for undo."""

    board: dict[tuple[int, int], str] = field(default_factory=dict)
    white_to_move: bool = True
    castling: set[str] = field(default_factory=lambda: set("KQkq"))
    en_passant: tuple[int, int] | None = None
    history: list[dict] = field(default_factory=list)  # undo records

    @classmethod
    def initial(cls) -> "Position":
        pos = cls()
        back = "RNBQKBNR"
        for f in range(8):
            pos.board[(f, 0)] = back[f]
            pos.board[(f, 1)] = "P"
            pos.board[(f, 6)] = "p"
            pos.board[(f, 7)] = back[f].lower()
        return pos

    # ── FEN ────────────────────────────────────────────────────────────────

    def fen(self) -> str:
        rows = []
        for rank in range(7, -1, -1):
            row = ""
            empty = 0
            for f in range(8):
                piece = self.board.get((f, rank))
                if piece is None:
                    empty += 1
                else:
                    if empty:
                        row += str(empty)
                        empty = 0
                    row += piece
            if empty:
                row += str(empty)
            rows.append(row)
        castle = "".join(c for c in "KQkq" if c in self.castling) or "-"
        ep = sq_name(self.en_passant) if self.en_passant else "-"
        side = "w" if self.white_to_move else "b"
        fullmove = len(self.history) // 2 + 1
        return f"{'/'.join(rows)} {side} {castle} {ep} 0 {fullmove}"

    # ── Attack / check detection ───────────────────────────────────────────

    def _is_white(self, piece: str) -> bool:
        return piece.isupper()

    def square_attacked_by(self, sq: tuple[int, int], by_white: bool) -> bool:
        f, r = sq
        # Pawn attacks.
        pawn_rank = r - 1 if by_white else r + 1
        pawn = "P" if by_white else "p"
        for df in (-1, 1):
            if self.board.get((f + df, pawn_rank)) == pawn:
                return True
        # Knights.
        knight = "N" if by_white else "n"
        for df, dr in KNIGHT_DELTAS:
            if self.board.get((f + df, r + dr)) == knight:
                return True
        # King (adjacency).
        king = "K" if by_white else "k"
        for df, dr in KING_DELTAS:
            if self.board.get((f + df, r + dr)) == king:
                return True
        # Sliders.
        for dirs, sliders in (
            (BISHOP_DIRS, ("B", "Q") if by_white else ("b", "q")),
            (ROOK_DIRS, ("R", "Q") if by_white else ("r", "q")),
        ):
            for df, dr in dirs:
                nf, nr = f + df, r + dr
                while 0 <= nf < 8 and 0 <= nr < 8:
                    piece = self.board.get((nf, nr))
                    if piece is not None:
                        if piece in sliders:
                            return True
                        break
                    nf += df
                    nr += dr
        return False

    def king_square(self, white: bool) -> tuple[int, int]:
        target = "K" if white else "k"
        for sq, piece in self.board.items():
            if piece == target:
                return sq
        raise ValueError(f"no {'white' if white else 'black'} king on the board")

    def in_check(self, white: bool) -> bool:
        return self.square_attacked_by(self.king_square(white), by_white=not white)

    # ── Move generation ────────────────────────────────────────────────────

    def _pseudo_legal(self) -> list[Move]:
        moves: list[Move] = []
        white = self.white_to_move
        for (f, r), piece in list(self.board.items()):
            if self._is_white(piece) != white:
                continue
            kind = piece.upper()
            if kind == "P":
                self._pawn_moves(f, r, white, moves)
            elif kind == "N":
                for df, dr in KNIGHT_DELTAS:
                    self._step_move(piece, f, r, f + df, r + dr, moves)
            elif kind == "K":
                for df, dr in KING_DELTAS:
                    self._step_move(piece, f, r, f + df, r + dr, moves)
            else:
                dirs = []
                if kind in ("B", "Q"):
                    dirs += BISHOP_DIRS
                if kind in ("R", "Q"):
                    dirs += ROOK_DIRS
                for df, dr in dirs:
                    nf, nr = f + df, r + dr
                    while 0 <= nf < 8 and 0 <= nr < 8:
                        target = self.board.get((nf, nr))
                        if target is None:
                            moves.append(Move((f, r), (nf, nr), piece))
                        else:
                            if self._is_white(target) != white:
                                moves.append(Move((f, r), (nf, nr), piece, capture=True))
                            break
                        nf += df
                        nr += dr
        self._castle_moves(white, moves)
        return moves

    def _step_move(self, piece: str, f: int, r: int, nf: int, nr: int,
                   moves: list[Move]) -> None:
        if not (0 <= nf < 8 and 0 <= nr < 8):
            return
        target = self.board.get((nf, nr))
        if target is None:
            moves.append(Move((f, r), (nf, nr), piece))
        elif self._is_white(target) != self._is_white(piece):
            moves.append(Move((f, r), (nf, nr), piece, capture=True))

    def _pawn_moves(self, f: int, r: int, white: bool, moves: list[Move]) -> None:
        piece = "P" if white else "p"
        step = 1 if white else -1
        start_rank = 1 if white else 6
        promo_rank = 7 if white else 0

        def add(dst: tuple[int, int], capture: bool, en_passant: bool = False) -> None:
            if dst[1] == promo_rank:
                for promo in ("Q", "R", "B", "N"):
                    moves.append(Move((f, r), dst, piece, capture=capture,
                                      promotion=promo, is_en_passant=en_passant))
            else:
                moves.append(Move((f, r), dst, piece, capture=capture,
                                  is_en_passant=en_passant))

        # Forward.
        one = (f, r + step)
        if 0 <= one[1] < 8 and self.board.get(one) is None:
            add(one, capture=False)
            two = (f, r + 2 * step)
            if r == start_rank and self.board.get(two) is None:
                moves.append(Move((f, r), two, piece))
        # Captures + en passant.
        for df in (-1, 1):
            dst = (f + df, r + step)
            if not (0 <= dst[0] < 8 and 0 <= dst[1] < 8):
                continue
            target = self.board.get(dst)
            if target is not None and self._is_white(target) != white:
                add(dst, capture=True)
            elif dst == self.en_passant:
                add(dst, capture=True, en_passant=True)

    def _castle_moves(self, white: bool, moves: list[Move]) -> None:
        rank = 0 if white else 7
        king = "K" if white else "k"
        if self.board.get((4, rank)) != king or self.in_check(white):
            return
        kingside, queenside = ("K", "Q") if white else ("k", "q")
        if kingside in self.castling \
                and self.board.get((5, rank)) is None \
                and self.board.get((6, rank)) is None \
                and not self.square_attacked_by((5, rank), not white) \
                and not self.square_attacked_by((6, rank), not white):
            moves.append(Move((4, rank), (6, rank), king, is_castle_kingside=True))
        if queenside in self.castling \
                and self.board.get((3, rank)) is None \
                and self.board.get((2, rank)) is None \
                and self.board.get((1, rank)) is None \
                and not self.square_attacked_by((3, rank), not white) \
                and not self.square_attacked_by((2, rank), not white):
            moves.append(Move((4, rank), (2, rank), king, is_castle_queenside=True))

    def legal_moves(self) -> list[Move]:
        """All legal moves for the side to move, with SAN filled in."""
        legal: list[Move] = []
        for move in self._pseudo_legal():
            self._apply(move)
            if not self.in_check(not self.white_to_move):
                legal.append(move)
            self._undo_apply()
        for move in legal:
            move.san = self._san(move, legal)
        return legal

    # ── Apply / undo ───────────────────────────────────────────────────────

    def _apply(self, move: Move) -> None:
        record = {
            "move": move,
            "captured": None,
            "captured_sq": None,
            "castling": set(self.castling),
            "en_passant": self.en_passant,
        }
        src, dst = move.src, move.dst
        piece = self.board.pop(src)
        if move.is_en_passant:
            cap_sq = (dst[0], src[1])
            record["captured"] = self.board.pop(cap_sq)
            record["captured_sq"] = cap_sq
        elif dst in self.board:
            record["captured"] = self.board[dst]
            record["captured_sq"] = dst
        if move.promotion:
            piece = move.promotion.upper() if piece.isupper() else move.promotion.lower()
        self.board[dst] = piece
        # Castle rook hop.
        rank = src[1]
        if move.is_castle_kingside:
            self.board[(5, rank)] = self.board.pop((7, rank))
        elif move.is_castle_queenside:
            self.board[(3, rank)] = self.board.pop((0, rank))
        # Castling rights.
        for sq, rights in (((4, 0), "KQ"), ((4, 7), "kq"), ((7, 0), "K"),
                           ((0, 0), "Q"), ((7, 7), "k"), ((0, 7), "q")):
            if src == sq or dst == sq:
                self.castling -= set(rights)
        # En passant target.
        if move.piece.upper() == "P" and abs(dst[1] - src[1]) == 2:
            self.en_passant = (src[0], (src[1] + dst[1]) // 2)
        else:
            self.en_passant = None
        self.white_to_move = not self.white_to_move
        self.history.append(record)

    def _undo_apply(self) -> None:
        record = self.history.pop()
        move: Move = record["move"]
        src, dst = move.src, move.dst
        piece = self.board.pop(dst)
        if move.promotion:
            piece = "P" if piece.isupper() else "p"
        self.board[src] = piece
        if record["captured"] is not None:
            self.board[record["captured_sq"]] = record["captured"]
        rank = src[1]
        if move.is_castle_kingside:
            self.board[(7, rank)] = self.board.pop((5, rank))
        elif move.is_castle_queenside:
            self.board[(0, rank)] = self.board.pop((3, rank))
        self.castling = record["castling"]
        self.en_passant = record["en_passant"]
        self.white_to_move = not self.white_to_move

    # ── SAN ────────────────────────────────────────────────────────────────

    def _san(self, move: Move, all_legal: list[Move]) -> str:
        if move.is_castle_kingside:
            base = "O-O"
        elif move.is_castle_queenside:
            base = "O-O-O"
        else:
            kind = move.piece.upper()
            dst = sq_name(move.dst)
            if kind == "P":
                base = (FILES[move.src[0]] + "x" if move.capture else "") + dst
                if move.promotion:
                    base += "=" + move.promotion.upper()
            else:
                # Disambiguation among same-kind pieces reaching the same dst.
                rivals = [m for m in all_legal
                          if m.piece == move.piece and m.dst == move.dst
                          and m.src != move.src]
                disambig = ""
                if rivals:
                    same_file = any(m.src[0] == move.src[0] for m in rivals)
                    same_rank = any(m.src[1] == move.src[1] for m in rivals)
                    if not same_file:
                        disambig = FILES[move.src[0]]
                    elif not same_rank:
                        disambig = str(move.src[1] + 1)
                    else:
                        disambig = sq_name(move.src)
                base = kind + disambig + ("x" if move.capture else "") + dst
        # Check / mate suffix.
        self._apply(move)
        if self.in_check(self.white_to_move):
            base += "#" if not self.legal_moves_exist() else "+"
        self._undo_apply()
        return base

    def legal_moves_exist(self) -> bool:
        for move in self._pseudo_legal():
            self._apply(move)
            ok = not self.in_check(not self.white_to_move)
            self._undo_apply()
            if ok:
                return True
        return False

    # ── Game-level API ─────────────────────────────────────────────────────

    def make_move(self, notation: str) -> Move:
        """Apply a move given in SAN or UCI. Raises ValueError when illegal."""
        wanted = notation.strip()
        legal = self.legal_moves()
        for move in legal:
            if wanted in (move.san, move.san.rstrip("+#"), move.uci):
                self._apply(move)
                return move
        raise ValueError(
            f"illegal move {notation!r} — legal moves: "
            f"{', '.join(m.san for m in legal)}"
        )

    def undo_move(self) -> Move:
        """Undo the last applied move. Raises ValueError when no history."""
        if not self.history:
            raise ValueError("no moves to undo")
        move = self.history[-1]["move"]
        self._undo_apply()
        return move

    def result(self) -> str | None:
        """`"1-0"`, `"0-1"`, `"1/2-1/2"` when the game is over, else None."""
        if self.legal_moves_exist():
            return None
        if self.in_check(self.white_to_move):
            return "0-1" if self.white_to_move else "1-0"
        return "1/2-1/2"
