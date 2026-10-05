//! Chess rules and the game store. The store is the only mutator.
//! Grants are matched here on actor, game, and side. `GrantRecord::matches`
//! is not used: it does not compare `resource_id`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub const START_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

const WK: u8 = 1;
const WQ: u8 = 2;
const BK: u8 = 4;
const BQ: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    White,
    Black,
}

impl Side {
    fn opposite(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Black => "black",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Pawn,
    Knight,
    Bishop,
    Rook,
    Queen,
    King,
}

impl Kind {
    fn promo_char(self) -> Option<char> {
        match self {
            Self::Knight => Some('n'),
            Self::Bishop => Some('b'),
            Self::Rook => Some('r'),
            Self::Queen => Some('q'),
            Self::Pawn | Self::King => None,
        }
    }

    fn from_promo(ch: char) -> Option<Self> {
        match ch {
            'n' => Some(Self::Knight),
            'b' => Some(Self::Bishop),
            'r' => Some(Self::Rook),
            'q' => Some(Self::Queen),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Piece {
    side: Side,
    kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Move {
    from: u8,
    to: u8,
    promotion: Option<Kind>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Position {
    board: [Option<Piece>; 64],
    side: Side,
    castling: u8,
    ep: Option<u8>,
    halfmove: u16,
    fullmove: u16,
}

impl Position {
    pub fn start() -> Self {
        Self::from_fen(START_FEN).expect("start fen")
    }

    pub fn from_fen(fen: &str) -> Result<Self, String> {
        let mut parts = fen.split_whitespace();
        let placement = parts.next().ok_or("fen: missing placement")?;
        let side = match parts.next() {
            Some("w") => Side::White,
            Some("b") => Side::Black,
            _ => return Err("fen: missing side".into()),
        };
        let castling = parse_castling(parts.next().unwrap_or("-"))?;
        let ep = match parts.next().unwrap_or("-") {
            "-" => None,
            name => Some(parse_square(name).ok_or("fen: bad en passant")?),
        };
        let halfmove = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let fullmove = parts.next().unwrap_or("1").parse().unwrap_or(1);
        let mut board = [None; 64];
        let mut rank = 7i32;
        let mut file = 0i32;
        for ch in placement.chars() {
            if ch == '/' {
                rank -= 1;
                file = 0;
                continue;
            }
            if ch.is_ascii_digit() {
                file += ch.to_digit(10).unwrap() as i32;
                continue;
            }
            if !(0..8).contains(&file) || !(0..8).contains(&rank) {
                return Err("fen: placement overflow".into());
            }
            let (side, kind) = fen_piece(ch).ok_or("fen: bad piece")?;
            board[(rank * 8 + file) as usize] = Some(Piece { side, kind });
            file += 1;
        }
        Ok(Self {
            board,
            side,
            castling,
            ep,
            halfmove,
            fullmove,
        })
    }

    pub fn to_fen(&self) -> String {
        let mut placement = String::new();
        for rank in (0..8).rev() {
            let mut empty = 0;
            for file in 0..8 {
                match self.board[rank * 8 + file] {
                    None => empty += 1,
                    Some(piece) => {
                        if empty > 0 {
                            placement.push(char::from_digit(empty, 10).unwrap());
                            empty = 0;
                        }
                        placement.push(fen_char(piece));
                    }
                }
            }
            if empty > 0 {
                placement.push(char::from_digit(empty, 10).unwrap());
            }
            if rank > 0 {
                placement.push('/');
            }
        }
        let mut castle = String::new();
        if self.castling & WK != 0 {
            castle.push('K');
        }
        if self.castling & WQ != 0 {
            castle.push('Q');
        }
        if self.castling & BK != 0 {
            castle.push('k');
        }
        if self.castling & BQ != 0 {
            castle.push('q');
        }
        if castle.is_empty() {
            castle.push('-');
        }
        let ep = self.ep.map(square_name).unwrap_or_else(|| "-".into());
        format!(
            "{placement} {} {castle} {ep} {} {}",
            match self.side {
                Side::White => "w",
                Side::Black => "b",
            },
            self.halfmove,
            self.fullmove
        )
    }

    pub fn side(&self) -> Side {
        self.side
    }

    pub fn status(&self) -> GameStatus {
        if legal_moves(self).is_empty() {
            if king_in_check(self, self.side) {
                GameStatus::Checkmate
            } else {
                GameStatus::Stalemate
            }
        } else {
            GameStatus::Ongoing
        }
    }

    pub fn legal_uci(&self) -> Vec<String> {
        let mut moves: Vec<String> = legal_moves(self).into_iter().map(move_uci).collect();
        moves.sort();
        moves.dedup();
        moves
    }

    pub fn play_uci(&self, uci: &str) -> Result<Self, &'static str> {
        if self.status() != GameStatus::Ongoing {
            return Err("game_over");
        }
        let mv = parse_uci(uci).ok_or("illegal_move")?;
        if !legal_moves(self).into_iter().any(|legal| legal == mv) {
            return Err("illegal_move");
        }
        Ok(apply(self, mv))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameStatus {
    Ongoing,
    Checkmate,
    Stalemate,
}

impl GameStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ongoing => "ongoing",
            Self::Checkmate => "checkmate",
            Self::Stalemate => "stalemate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorGrant {
    pub actor_id: String,
    pub game_id: String,
    pub inspect: bool,
    #[serde(default)]
    pub play: Option<Side>,
    pub reset: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Play,
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveReceipt {
    pub actor_id: String,
    pub operation_id: String,
    pub game_id: String,
    pub kind: OpKind,
    pub uci: String,
    pub expected_revision: u64,
    pub revision_before: u64,
    pub revision_after: u64,
    pub generation: u64,
    pub side_played: Option<Side>,
    pub side_to_move: Side,
    pub status: GameStatus,
    pub fen_after: String,
    /// Fen supplied to a reset. `None` for a move or a standard-start reset.
    #[serde(default)]
    pub setup_fen: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayResult {
    Committed(MoveReceipt),
    Replayed(MoveReceipt),
}

impl PlayResult {
    pub fn receipt(&self) -> &MoveReceipt {
        match self {
            Self::Committed(receipt) | Self::Replayed(receipt) => receipt,
        }
    }

    pub fn is_fresh(&self) -> bool {
        matches!(self, Self::Committed(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChessError {
    Denied,
    Unauthorized { revision: u64 },
    StaleRevision { current: u64 },
    DuplicateConflict,
    IllegalMove,
    GameOver { revision: u64 },
    InvalidInput(&'static str),
    InstanceStopped,
}

impl ChessError {
    pub fn message(&self) -> String {
        match self {
            Self::Denied => "denied".into(),
            Self::Unauthorized { revision } => format!("unauthorized: revision {revision}"),
            Self::StaleRevision { current } => format!("stale_revision: current {current}"),
            Self::DuplicateConflict => "duplicate_conflict".into(),
            Self::IllegalMove => "illegal_move".into(),
            Self::GameOver { revision } => format!("game_over: revision {revision}"),
            Self::InvalidInput(detail) => format!("invalid_input: {detail}"),
            Self::InstanceStopped => "instance_stopped".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Game {
    revision: u64,
    generation: u64,
    position: Position,
    history: Vec<String>,
    receipts: BTreeMap<String, MoveReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChessStore {
    games: BTreeMap<String, Game>,
    grants: Vec<ActorGrant>,
    unpublished: BTreeSet<String>,
}

impl ChessStore {
    pub fn new() -> Self {
        Self {
            games: BTreeMap::new(),
            grants: Vec::new(),
            unpublished: BTreeSet::new(),
        }
    }

    pub fn provision(&mut self, game_id: &str, grants: Vec<ActorGrant>) {
        self.games.insert(
            game_id.to_string(),
            Game {
                revision: 0,
                generation: 1,
                position: Position::start(),
                history: Vec::new(),
                receipts: BTreeMap::new(),
            },
        );
        self.grants.retain(|grant| grant.game_id != game_id);
        self.grants.extend(grants);
    }

    pub fn grant_for(&self, actor_id: &str, game_id: &str) -> Option<&ActorGrant> {
        self.grants
            .iter()
            .find(|grant| grant.actor_id == actor_id && grant.game_id == game_id)
    }

    pub fn revision(&self, game_id: &str) -> Option<u64> {
        self.games.get(game_id).map(|game| game.revision)
    }

    pub fn fen(&self, game_id: &str) -> Option<String> {
        self.games.get(game_id).map(|game| game.position.to_fen())
    }

    pub fn history(&self, game_id: &str) -> Option<&[String]> {
        self.games.get(game_id).map(|game| game.history.as_slice())
    }

    pub fn unpublished(&self) -> &BTreeSet<String> {
        &self.unpublished
    }

    pub fn receipt(&self, key: &str) -> Option<&MoveReceipt> {
        self.games.values().find_map(|game| game.receipts.get(key))
    }

    pub fn mark_published(&mut self, key: &str) {
        self.unpublished.remove(key);
    }

    pub fn to_journal(&self) -> ChessJournal {
        ChessJournal {
            games: self
                .games
                .iter()
                .map(|(id, game)| JournalGame {
                    id: id.clone(),
                    revision: game.revision,
                    generation: game.generation,
                    fen: game.position.to_fen(),
                    history: game.history.clone(),
                    receipts: game.receipts.values().cloned().collect(),
                })
                .collect(),
            grants: self.grants.clone(),
            unpublished: self.unpublished.iter().cloned().collect(),
        }
    }

    pub fn from_journal(journal: ChessJournal) -> Result<Self, String> {
        let mut games = BTreeMap::new();
        for game in journal.games {
            let position = Position::from_fen(&game.fen)?;
            let mut receipts = BTreeMap::new();
            for receipt in game.receipts {
                receipts.insert(op_key(&receipt.actor_id, &receipt.operation_id), receipt);
            }
            games.insert(
                game.id,
                Game {
                    revision: game.revision,
                    generation: game.generation,
                    position,
                    history: game.history,
                    receipts,
                },
            );
        }
        Ok(Self {
            games,
            grants: journal.grants,
            unpublished: journal.unpublished.into_iter().collect(),
        })
    }

    pub fn play(
        &mut self,
        actor_id: &str,
        game_id: &str,
        expected_revision: u64,
        operation_id: &str,
        uci: &str,
    ) -> Result<PlayResult, ChessError> {
        self.mutate(OpRequest {
            actor_id,
            game_id,
            expected_revision,
            operation_id,
            kind: OpKind::Play,
            uci,
            fen: None,
        })
    }

    pub fn reset(
        &mut self,
        actor_id: &str,
        game_id: &str,
        operation_id: &str,
        fen: Option<&str>,
    ) -> Result<PlayResult, ChessError> {
        let current = self
            .games
            .get(game_id)
            .map(|game| game.revision)
            .ok_or(ChessError::Denied)?;
        self.mutate(OpRequest {
            actor_id,
            game_id,
            expected_revision: current,
            operation_id,
            kind: OpKind::Reset,
            uci: "",
            fen,
        })
    }

    pub fn state_for(&self, actor_id: &str, game_id: &str) -> Result<StateView, ChessError> {
        let grant = self
            .grant_for(actor_id, game_id)
            .ok_or(ChessError::Denied)?;
        if !grant.inspect && grant.play.is_none() && !grant.reset {
            return Err(ChessError::Denied);
        }
        let game = self.games.get(game_id).ok_or(ChessError::Denied)?;
        Ok(StateView::from_game(game_id, game))
    }

    pub fn legal_for(
        &self,
        actor_id: &str,
        game_id: &str,
        revision: u64,
    ) -> Result<Vec<String>, ChessError> {
        let view = self.state_for(actor_id, game_id)?;
        if view.revision != revision {
            return Err(ChessError::StaleRevision {
                current: view.revision,
            });
        }
        Ok(view.legal_moves)
    }

    fn mutate(&mut self, request: OpRequest<'_>) -> Result<PlayResult, ChessError> {
        if request.operation_id.is_empty() || request.game_id.is_empty() {
            return Err(ChessError::InvalidInput(
                "operation_id and game_id are required",
            ));
        }
        let key = op_key(request.actor_id, request.operation_id);
        if let Some(existing) = self.receipt(&key).cloned() {
            return if same_payload(&existing, &request) {
                Ok(PlayResult::Replayed(existing))
            } else {
                Err(ChessError::DuplicateConflict)
            };
        }
        let grant = self
            .grant_for(request.actor_id, request.game_id)
            .cloned()
            .ok_or(ChessError::Denied)?;
        let game = self.games.get(request.game_id).ok_or(ChessError::Denied)?;
        match request.kind {
            OpKind::Play => {
                if game.revision != request.expected_revision {
                    return Err(ChessError::StaleRevision {
                        current: game.revision,
                    });
                }
                let play_side = grant.play.ok_or(ChessError::Unauthorized {
                    revision: game.revision,
                })?;
                if play_side != game.position.side {
                    return Err(ChessError::Unauthorized {
                        revision: game.revision,
                    });
                }
                if game.position.status() != GameStatus::Ongoing {
                    return Err(ChessError::GameOver {
                        revision: game.revision,
                    });
                }
                let next = game.position.play_uci(request.uci).map_err(|code| {
                    if code == "game_over" {
                        ChessError::GameOver {
                            revision: game.revision,
                        }
                    } else {
                        ChessError::IllegalMove
                    }
                })?;
                let receipt = MoveReceipt {
                    actor_id: request.actor_id.to_string(),
                    operation_id: request.operation_id.to_string(),
                    game_id: request.game_id.to_string(),
                    kind: OpKind::Play,
                    uci: request.uci.to_string(),
                    expected_revision: request.expected_revision,
                    revision_before: game.revision,
                    revision_after: game.revision + 1,
                    generation: game.generation,
                    side_played: Some(play_side),
                    side_to_move: next.side,
                    status: next.status(),
                    fen_after: next.to_fen(),
                    setup_fen: None,
                };
                let game = self.games.get_mut(request.game_id).expect("game present");
                game.revision += 1;
                game.position = next;
                game.history.push(request.uci.to_string());
                game.receipts.insert(key.clone(), receipt.clone());
                self.unpublished.insert(key);
                Ok(PlayResult::Committed(receipt))
            }
            OpKind::Reset => {
                if !grant.reset {
                    return Err(ChessError::Unauthorized {
                        revision: game.revision,
                    });
                }
                let position = match request.fen {
                    None => Position::start(),
                    Some(fen) => {
                        Position::from_fen(fen).map_err(|_| ChessError::InvalidInput("fen"))?
                    }
                };
                let next_revision = game.revision + 1;
                let next_generation = game.generation + 1;
                let receipt = MoveReceipt {
                    actor_id: request.actor_id.to_string(),
                    operation_id: request.operation_id.to_string(),
                    game_id: request.game_id.to_string(),
                    kind: OpKind::Reset,
                    uci: String::new(),
                    expected_revision: game.revision,
                    revision_before: game.revision,
                    revision_after: next_revision,
                    generation: next_generation,
                    side_played: None,
                    side_to_move: position.side,
                    status: position.status(),
                    fen_after: position.to_fen(),
                    setup_fen: request.fen.map(str::to_string),
                };
                let game = self.games.get_mut(request.game_id).expect("game present");
                game.revision = next_revision;
                game.generation = next_generation;
                game.position = position;
                game.history.clear();
                game.receipts.insert(key.clone(), receipt.clone());
                self.unpublished.insert(key);
                Ok(PlayResult::Committed(receipt))
            }
        }
    }
}

impl Default for ChessStore {
    fn default() -> Self {
        Self::new()
    }
}

struct OpRequest<'a> {
    actor_id: &'a str,
    game_id: &'a str,
    expected_revision: u64,
    operation_id: &'a str,
    kind: OpKind,
    uci: &'a str,
    fen: Option<&'a str>,
}

fn same_payload(existing: &MoveReceipt, request: &OpRequest<'_>) -> bool {
    if existing.game_id != request.game_id || existing.kind != request.kind {
        return false;
    }
    match request.kind {
        OpKind::Play => {
            existing.uci == request.uci && existing.expected_revision == request.expected_revision
        }
        OpKind::Reset => existing.setup_fen.as_deref() == request.fen,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateView {
    pub game_id: String,
    pub revision: u64,
    pub generation: u64,
    pub fen: String,
    pub side_to_move: Side,
    pub status: GameStatus,
    pub history: Vec<String>,
    pub legal_moves: Vec<String>,
}

impl StateView {
    fn from_game(game_id: &str, game: &Game) -> Self {
        Self {
            game_id: game_id.to_string(),
            revision: game.revision,
            generation: game.generation,
            fen: game.position.to_fen(),
            side_to_move: game.position.side,
            status: game.position.status(),
            history: game.history.clone(),
            legal_moves: game.position.legal_uci(),
        }
    }
}

pub fn op_key(actor_id: &str, operation_id: &str) -> String {
    format!("{actor_id}\u{1}{operation_id}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChessJournal {
    pub games: Vec<JournalGame>,
    pub grants: Vec<ActorGrant>,
    pub unpublished: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalGame {
    pub id: String,
    pub revision: u64,
    pub generation: u64,
    pub fen: String,
    pub history: Vec<String>,
    pub receipts: Vec<MoveReceipt>,
}

fn file_of(sq: u8) -> u8 {
    sq % 8
}

fn rank_of(sq: u8) -> u8 {
    sq / 8
}

fn square(file: u8, rank: u8) -> u8 {
    rank * 8 + file
}

fn square_name(sq: u8) -> String {
    format!(
        "{}{}",
        (b'a' + file_of(sq)) as char,
        (b'1' + rank_of(sq)) as char
    )
}

fn parse_square(name: &str) -> Option<u8> {
    let bytes = name.as_bytes();
    if bytes.len() != 2 {
        return None;
    }
    let file = bytes[0];
    let rank = bytes[1];
    if !(b'a'..=b'h').contains(&file) || !(b'1'..=b'8').contains(&rank) {
        return None;
    }
    Some((rank - b'1') * 8 + (file - b'a'))
}

fn parse_castling(text: &str) -> Result<u8, String> {
    if text == "-" {
        return Ok(0);
    }
    let mut bits = 0u8;
    for ch in text.chars() {
        bits |= match ch {
            'K' => WK,
            'Q' => WQ,
            'k' => BK,
            'q' => BQ,
            _ => return Err("fen: bad castling".into()),
        };
    }
    Ok(bits)
}

fn fen_piece(ch: char) -> Option<(Side, Kind)> {
    let side = if ch.is_ascii_uppercase() {
        Side::White
    } else {
        Side::Black
    };
    let kind = match ch.to_ascii_lowercase() {
        'p' => Kind::Pawn,
        'n' => Kind::Knight,
        'b' => Kind::Bishop,
        'r' => Kind::Rook,
        'q' => Kind::Queen,
        'k' => Kind::King,
        _ => return None,
    };
    Some((side, kind))
}

fn fen_char(piece: Piece) -> char {
    let ch = match piece.kind {
        Kind::Pawn => 'p',
        Kind::Knight => 'n',
        Kind::Bishop => 'b',
        Kind::Rook => 'r',
        Kind::Queen => 'q',
        Kind::King => 'k',
    };
    if piece.side == Side::White {
        ch.to_ascii_uppercase()
    } else {
        ch
    }
}

fn parse_uci(uci: &str) -> Option<Move> {
    let bytes = uci.as_bytes();
    if bytes.len() != 4 && bytes.len() != 5 {
        return None;
    }
    let from = parse_square(&uci[0..2])?;
    let to = parse_square(&uci[2..4])?;
    let promotion = if bytes.len() == 5 {
        Some(Kind::from_promo(bytes[4] as char)?)
    } else {
        None
    };
    Some(Move {
        from,
        to,
        promotion,
    })
}

fn move_uci(mv: Move) -> String {
    let mut uci = format!("{}{}", square_name(mv.from), square_name(mv.to));
    if let Some(ch) = mv.promotion.and_then(Kind::promo_char) {
        uci.push(ch);
    }
    uci
}

fn on_board(file: i8, rank: i8) -> bool {
    (0..8).contains(&file) && (0..8).contains(&rank)
}

fn attacks(pos: &Position, side: Side, target: u8) -> bool {
    let tf = file_of(target) as i8;
    let tr = rank_of(target) as i8;
    for sq in 0..64u8 {
        let Some(piece) = pos.board[sq as usize] else {
            continue;
        };
        if piece.side != side {
            continue;
        }
        let f = file_of(sq) as i8;
        let r = rank_of(sq) as i8;
        let hits = match piece.kind {
            Kind::Pawn => {
                let dir: i8 = if side == Side::White { 1 } else { -1 };
                (tr - r) == dir && (tf - f).abs() == 1
            }
            Kind::Knight => {
                let df = (tf - f).abs();
                let dr = (tr - r).abs();
                (df == 1 && dr == 2) || (df == 2 && dr == 1)
            }
            Kind::King => (tf - f).abs() <= 1 && (tr - r).abs() <= 1 && sq != target,
            Kind::Bishop => ray(pos, f, r, tf, tr, true, false),
            Kind::Rook => ray(pos, f, r, tf, tr, false, true),
            Kind::Queen => ray(pos, f, r, tf, tr, true, true),
        };
        if hits {
            return true;
        }
    }
    false
}

fn ray(pos: &Position, f: i8, r: i8, tf: i8, tr: i8, diagonal: bool, straight: bool) -> bool {
    let df = tf - f;
    let dr = tr - r;
    if df == 0 && dr == 0 {
        return false;
    }
    let step_f = df.signum();
    let step_r = dr.signum();
    let diagonal_move = df.abs() == dr.abs();
    let straight_move = df == 0 || dr == 0;
    if diagonal_move && !diagonal {
        return false;
    }
    if straight_move && !straight {
        return false;
    }
    if !diagonal_move && !straight_move {
        return false;
    }
    let mut cf = f + step_f;
    let mut cr = r + step_r;
    while on_board(cf, cr) {
        let sq = square(cf as u8, cr as u8);
        if sq == square(tf as u8, tr as u8) {
            return true;
        }
        if pos.board[sq as usize].is_some() {
            return false;
        }
        cf += step_f;
        cr += step_r;
    }
    false
}

fn king_square(pos: &Position, side: Side) -> Option<u8> {
    (0..64u8).find(|&sq| {
        pos.board[sq as usize].is_some_and(|piece| piece.side == side && piece.kind == Kind::King)
    })
}

fn king_in_check(pos: &Position, side: Side) -> bool {
    match king_square(pos, side) {
        Some(sq) => attacks(pos, side.opposite(), sq),
        None => true,
    }
}

fn legal_moves(pos: &Position) -> Vec<Move> {
    pseudo_moves(pos)
        .into_iter()
        .filter(|mv| {
            if is_castle(pos, *mv) {
                let step = if mv.to > mv.from { 1 } else { -1 };
                let mut sq = mv.from;
                loop {
                    if attacks(pos, pos.side.opposite(), sq) {
                        return false;
                    }
                    if sq == mv.to {
                        break;
                    }
                    sq = sq.wrapping_add(step as u8);
                }
            }
            let next = apply(pos, *mv);
            !king_in_check(&next, pos.side)
        })
        .collect()
}

fn is_castle(pos: &Position, mv: Move) -> bool {
    pos.board[mv.from as usize].is_some_and(|piece| piece.kind == Kind::King)
        && mv.promotion.is_none()
        && mv.from.abs_diff(mv.to) == 2
}

fn pseudo_moves(pos: &Position) -> Vec<Move> {
    let mut moves = Vec::new();
    for sq in 0..64u8 {
        let Some(piece) = pos.board[sq as usize] else {
            continue;
        };
        if piece.side != pos.side {
            continue;
        }
        match piece.kind {
            Kind::Pawn => pawn_moves(pos, sq, &mut moves),
            Kind::Knight => step_moves(pos, sq, &KNIGHT, false, &mut moves),
            Kind::Bishop => slide_moves(pos, sq, &BISHOP, &mut moves),
            Kind::Rook => slide_moves(pos, sq, &ROOK, &mut moves),
            Kind::Queen => slide_moves(pos, sq, &QUEEN, &mut moves),
            Kind::King => {
                step_moves(pos, sq, &KING, false, &mut moves);
                castle_moves(pos, sq, &mut moves);
            }
        }
    }
    moves
}

const KNIGHT: [(i8, i8); 8] = [
    (1, 2),
    (2, 1),
    (-1, 2),
    (-2, 1),
    (1, -2),
    (2, -1),
    (-1, -2),
    (-2, -1),
];
const BISHOP: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
const ROOK: [(i8, i8); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
const QUEEN: [(i8, i8); 8] = [
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
];
const KING: [(i8, i8); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

fn pawn_moves(pos: &Position, from: u8, out: &mut Vec<Move>) {
    let side = pos.side;
    let dir: i8 = if side == Side::White { 1 } else { -1 };
    let start_rank: u8 = if side == Side::White { 1 } else { 6 };
    let promo_rank: u8 = if side == Side::White { 7 } else { 0 };
    let file = file_of(from) as i8;
    let rank = rank_of(from) as i8;
    let push_rank = rank + dir;
    if on_board(file, push_rank) {
        let to = square(file as u8, push_rank as u8);
        if pos.board[to as usize].is_none() {
            push_pawn(from, to, push_rank as u8 == promo_rank, out);
            if rank_of(from) == start_rank {
                let double_rank = push_rank + dir;
                let double = square(file as u8, double_rank as u8);
                if pos.board[double as usize].is_none() {
                    out.push(Move {
                        from,
                        to: double,
                        promotion: None,
                    });
                }
            }
        }
    }
    for df in [-1, 1] {
        let cf = file + df;
        let cr = rank + dir;
        if !on_board(cf, cr) {
            continue;
        }
        let to = square(cf as u8, cr as u8);
        let capture = match pos.board[to as usize] {
            Some(piece) if piece.side != side => true,
            None => pos.ep == Some(to),
            _ => false,
        };
        if capture {
            push_pawn(from, to, cr as u8 == promo_rank, out);
        }
    }
}

fn push_pawn(from: u8, to: u8, promote: bool, out: &mut Vec<Move>) {
    if promote {
        for kind in [Kind::Queen, Kind::Rook, Kind::Bishop, Kind::Knight] {
            out.push(Move {
                from,
                to,
                promotion: Some(kind),
            });
        }
    } else {
        out.push(Move {
            from,
            to,
            promotion: None,
        });
    }
}

fn step_moves(pos: &Position, from: u8, deltas: &[(i8, i8)], _slide: bool, out: &mut Vec<Move>) {
    let file = file_of(from) as i8;
    let rank = rank_of(from) as i8;
    for (df, dr) in deltas {
        let cf = file + df;
        let cr = rank + dr;
        if !on_board(cf, cr) {
            continue;
        }
        let to = square(cf as u8, cr as u8);
        if let Some(piece) = pos.board[to as usize] {
            if piece.side == pos.side {
                continue;
            }
        }
        out.push(Move {
            from,
            to,
            promotion: None,
        });
    }
}

fn slide_moves(pos: &Position, from: u8, deltas: &[(i8, i8)], out: &mut Vec<Move>) {
    let file = file_of(from) as i8;
    let rank = rank_of(from) as i8;
    for (df, dr) in deltas {
        let mut cf = file + df;
        let mut cr = rank + dr;
        while on_board(cf, cr) {
            let to = square(cf as u8, cr as u8);
            match pos.board[to as usize] {
                None => out.push(Move {
                    from,
                    to,
                    promotion: None,
                }),
                Some(piece) if piece.side != pos.side => {
                    out.push(Move {
                        from,
                        to,
                        promotion: None,
                    });
                    break;
                }
                Some(_) => break,
            }
            cf += df;
            cr += dr;
        }
    }
}

fn castle_moves(pos: &Position, from: u8, out: &mut Vec<Move>) {
    let (king_side, queen_side, home) = match pos.side {
        Side::White => (WK, WQ, 4),
        Side::Black => (BK, BQ, 60),
    };
    if from != home {
        return;
    }
    if pos.castling & king_side != 0 && path_clear(pos, from + 1, from + 2) {
        out.push(Move {
            from,
            to: from + 2,
            promotion: None,
        });
    }
    if pos.castling & queen_side != 0 && path_clear(pos, from - 3, from - 1) {
        out.push(Move {
            from,
            to: from - 2,
            promotion: None,
        });
    }
}

fn path_clear(pos: &Position, start: u8, end: u8) -> bool {
    (start..=end).all(|sq| pos.board[sq as usize].is_none())
}

fn apply(pos: &Position, mv: Move) -> Position {
    let mut next = pos.clone();
    let piece = next.board[mv.from as usize].expect("mover present");
    let captured = next.board[mv.to as usize];
    let mut ep_capture = false;
    if piece.kind == Kind::Pawn && pos.ep == Some(mv.to) && captured.is_none() {
        let cap_sq = if piece.side == Side::White {
            mv.to - 8
        } else {
            mv.to + 8
        };
        next.board[cap_sq as usize] = None;
        ep_capture = true;
    }
    let mut placed = piece;
    if let Some(kind) = mv.promotion {
        placed.kind = kind;
    }
    next.board[mv.to as usize] = Some(placed);
    next.board[mv.from as usize] = None;
    if piece.kind == Kind::King && mv.from.abs_diff(mv.to) == 2 {
        let (rook_from, rook_to) = if mv.to > mv.from {
            (mv.from + 3, mv.from + 1)
        } else {
            (mv.from - 4, mv.from - 1)
        };
        let rook = next.board[rook_from as usize];
        next.board[rook_to as usize] = rook;
        next.board[rook_from as usize] = None;
    }
    if piece.kind == Kind::King {
        next.castling &= if piece.side == Side::White {
            !(WK | WQ)
        } else {
            !(BK | BQ)
        };
    }
    if piece.kind == Kind::Rook {
        clear_rook_right(&mut next.castling, mv.from);
    }
    if captured.is_some_and(|piece| piece.kind == Kind::Rook) {
        clear_rook_right(&mut next.castling, mv.to);
    }
    next.ep = None;
    if piece.kind == Kind::Pawn && mv.from.abs_diff(mv.to) == 16 {
        next.ep = Some(if piece.side == Side::White {
            mv.from + 8
        } else {
            mv.from - 8
        });
    }
    let pawn_or_capture = piece.kind == Kind::Pawn || captured.is_some() || ep_capture;
    next.halfmove = if pawn_or_capture { 0 } else { pos.halfmove + 1 };
    if piece.side == Side::Black {
        next.fullmove = pos.fullmove + 1;
    }
    next.side = piece.side.opposite();
    next
}

fn clear_rook_right(castling: &mut u8, sq: u8) {
    *castling &= match sq {
        0 => !WQ,
        7 => !WK,
        56 => !BQ,
        63 => !BK,
        _ => 0xff,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play_line(mut pos: Position, moves: &[&str]) -> Position {
        for uci in moves {
            pos = pos
                .play_uci(uci)
                .unwrap_or_else(|_| panic!("legal {uci} from {}", pos.to_fen()));
        }
        pos
    }

    #[test]
    fn start_fen_round_trips_and_e4_is_legal() {
        let pos = Position::start();
        assert_eq!(pos.to_fen(), START_FEN);
        let next = pos.play_uci("e2e4").unwrap();
        assert_eq!(
            next.to_fen(),
            "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1"
        );
        assert!(pos.play_uci("e2e5").is_err());
        assert_eq!(pos.to_fen(), START_FEN);
    }

    #[test]
    fn scholars_mate_en_passant_castling_promotion_and_pin() {
        let mate = play_line(
            Position::start(),
            &["e2e4", "e7e5", "d1h5", "b8c6", "f1c4", "g8f6", "h5f7"],
        );
        assert_eq!(mate.status(), GameStatus::Checkmate);
        assert_eq!(mate.side(), Side::Black);
        assert!(mate.play_uci("e8d7").is_err());

        let ep = play_line(Position::start(), &["e2e4", "a7a6", "e4e5", "d7d5", "e5d6"]);
        assert!(ep
            .to_fen()
            .starts_with("rnbqkbnr/1pp1pppp/p2P4/8/8/8/PPPP1PPP/RNBQKBNR"));
        assert!(!ep.to_fen().contains("d5"));

        let castle = play_line(
            Position::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap(),
            &["e1g1", "e8c8"],
        );
        assert!(castle.to_fen().starts_with("2kr3r/8/8/8/8/8/8/R4RK1"));
        let blocked = Position::from_fen("r3k2r/8/8/8/8/5r2/8/R3K2R w KQkq - 0 1").unwrap();
        assert!(!blocked.legal_uci().iter().any(|uci| uci == "e1g1"));

        let promo = Position::from_fen("k7/4P3/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        assert!(promo.play_uci("e7e8").is_err());
        let queen = promo.play_uci("e7e8q").unwrap();
        assert!(queen.to_fen().starts_with("k3Q3/"));

        let pin = Position::from_fen("4r3/8/8/8/4N3/8/8/4K3 w - - 0 1").unwrap();
        assert!(pin.legal_uci().iter().all(|uci| !uci.starts_with("e4")));

        let stale = Position::from_fen("k7/8/1Q6/8/8/8/8/7K b - - 0 1").unwrap();
        assert_eq!(stale.status(), GameStatus::Stalemate);
        assert!(stale.legal_uci().is_empty());
    }

    #[test]
    fn store_idempotency_grant_stale_and_reset_are_the_only_mutators() {
        let mut store = ChessStore::new();
        store.provision(
            "game-fixture",
            vec![ActorGrant {
                actor_id: "agent:white".into(),
                game_id: "game-fixture".into(),
                inspect: true,
                play: Some(Side::White),
                reset: false,
            }],
        );
        let first = store
            .play("agent:white", "game-fixture", 0, "op-1", "e2e4")
            .unwrap();
        assert!(first.is_fresh());
        assert_eq!(store.revision("game-fixture"), Some(1));
        let again = store
            .play("agent:white", "game-fixture", 0, "op-1", "e2e4")
            .unwrap();
        assert!(!again.is_fresh());
        assert_eq!(store.revision("game-fixture"), Some(1));
        assert_eq!(
            store.history("game-fixture").unwrap(),
            &["e2e4".to_string()]
        );
        let conflict = store.play("agent:white", "game-fixture", 0, "op-1", "d2d4");
        assert!(matches!(conflict, Err(ChessError::DuplicateConflict)));
        assert_eq!(store.revision("game-fixture"), Some(1));
        let stranger = store.play("agent:black", "game-fixture", 1, "op-b", "e7e5");
        assert!(matches!(stranger, Err(ChessError::Denied)));
        let stale = store.play("agent:white", "game-fixture", 0, "op-2", "d2d4");
        assert!(matches!(
            stale,
            Err(ChessError::StaleRevision { current: 1 })
        ));
        assert!(store.fen("game-fixture").unwrap().contains("4P3"));
    }
}
