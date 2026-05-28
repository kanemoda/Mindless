//! The board representation: bitboards plus a square→piece mailbox, full game
//! state, FEN parsing/serialization, and `make`/`unmake` with incremental
//! Zobrist hashing.

use crate::attacks::{KING_ATTACKS, KNIGHT_ATTACKS, PAWN_ATTACKS};
use crate::bitboard::Bitboard;
use crate::magic::{bishop_attacks, rook_attacks};
use crate::moves::{flag, Move};
use crate::types::{CastlingRights, Color, Piece, PieceType, Square};
use crate::zobrist::ZOBRIST;
use std::fmt;

/// The standard chess starting position in FEN.
pub const STARTPOS_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/// For each square, the castling-rights bits that *survive* a move touching it.
/// Applied to both the origin and destination square of every move, this clears
/// rights when a king or rook leaves home, or when a home rook is captured.
const CASTLE_MASK: [u8; 64] = castle_mask_table();

const fn castle_mask_table() -> [u8; 64] {
    let wk = CastlingRights::WHITE_KING;
    let wq = CastlingRights::WHITE_QUEEN;
    let bk = CastlingRights::BLACK_KING;
    let bq = CastlingRights::BLACK_QUEEN;
    let mut t = [0b1111u8; 64];
    t[0] = t[0] & !wq; // a1 rook home
    t[4] = t[4] & !(wk | wq); // e1 white king home
    t[7] = t[7] & !wk; // h1 rook home
    t[56] = t[56] & !bq; // a8 rook home
    t[60] = t[60] & !(bk | bq); // e8 black king home
    t[63] = t[63] & !bk; // h8 rook home
    t
}

/// Information captured before a move so it can be exactly undone.
#[derive(Clone, Copy)]
pub struct Undo {
    captured: Option<Piece>,
    castling: CastlingRights,
    ep_square: Option<Square>,
    halfmove: u16,
    key: u64,
}

/// A full chess position.
///
/// Pieces are stored both as bitboards (by type and by color) for fast set
/// operations and as a 64-entry mailbox for fast "what is on this square"
/// lookups; the two are kept consistent by the private `add`/`remove`/`move`
/// helpers, which also maintain the incremental Zobrist key.
#[derive(Clone, PartialEq, Eq)]
pub struct Board {
    by_type: [Bitboard; PieceType::COUNT],
    by_color: [Bitboard; Color::COUNT],
    mailbox: [Option<Piece>; Square::COUNT],
    stm: Color,
    castling: CastlingRights,
    ep_square: Option<Square>,
    halfmove: u16,
    fullmove: u16,
    key: u64,
}

impl Board {
    /// An empty board with White to move and no rights.
    pub fn empty() -> Board {
        Board {
            by_type: [Bitboard::EMPTY; PieceType::COUNT],
            by_color: [Bitboard::EMPTY; Color::COUNT],
            mailbox: [None; Square::COUNT],
            stm: Color::White,
            castling: CastlingRights::NONE,
            ep_square: None,
            halfmove: 0,
            fullmove: 1,
            key: 0,
        }
    }

    /// The standard starting position.
    pub fn startpos() -> Board {
        Board::from_fen(STARTPOS_FEN).expect("startpos FEN is valid")
    }

    // --- accessors ---

    /// Side to move.
    #[inline]
    pub fn side_to_move(&self) -> Color {
        self.stm
    }

    /// Castling rights.
    #[inline]
    pub fn castling_rights(&self) -> CastlingRights {
        self.castling
    }

    /// The en-passant target square, if any.
    #[inline]
    pub fn en_passant(&self) -> Option<Square> {
        self.ep_square
    }

    /// The halfmove clock (plies since the last pawn move or capture).
    #[inline]
    pub fn halfmove_clock(&self) -> u16 {
        self.halfmove
    }

    /// The fullmove number (starts at 1, increments after Black moves).
    #[inline]
    pub fn fullmove_number(&self) -> u16 {
        self.fullmove
    }

    /// The incrementally-maintained Zobrist hash of the position.
    #[inline]
    pub fn zobrist_key(&self) -> u64 {
        self.key
    }

    /// The piece on `sq`, if any.
    #[inline]
    pub fn piece_on(&self, sq: Square) -> Option<Piece> {
        self.mailbox[sq.index()]
    }

    /// All occupied squares.
    #[inline]
    pub fn occupied(&self) -> Bitboard {
        self.by_color[0] | self.by_color[1]
    }

    /// All squares occupied by `color`.
    #[inline]
    pub fn color_bb(&self, color: Color) -> Bitboard {
        self.by_color[color.index()]
    }

    /// All pieces of a kind, both colors.
    #[inline]
    pub fn pieces(&self, kind: PieceType) -> Bitboard {
        self.by_type[kind.index()]
    }

    /// All pieces of a kind belonging to `color`.
    #[inline]
    pub fn pieces_colored(&self, color: Color, kind: PieceType) -> Bitboard {
        self.by_type[kind.index()] & self.by_color[color.index()]
    }

    /// The square of `color`'s king. Assumes a king is present (always true for
    /// legal positions).
    #[inline]
    pub fn king_square(&self, color: Color) -> Square {
        self.pieces_colored(color, PieceType::King).lsb()
    }

    // --- attack / check queries ---

    /// All pieces of either color that attack `sq` given occupancy `occ`.
    pub fn attackers_to(&self, sq: Square, occ: Bitboard) -> Bitboard {
        let s = sq.index();
        (PAWN_ATTACKS[Color::White.index()][s] & self.pieces_colored(Color::Black, PieceType::Pawn))
            | (PAWN_ATTACKS[Color::Black.index()][s]
                & self.pieces_colored(Color::White, PieceType::Pawn))
            | (KNIGHT_ATTACKS[s] & self.pieces(PieceType::Knight))
            | (KING_ATTACKS[s] & self.pieces(PieceType::King))
            | (rook_attacks(sq, occ)
                & (self.pieces(PieceType::Rook) | self.pieces(PieceType::Queen)))
            | (bishop_attacks(sq, occ)
                & (self.pieces(PieceType::Bishop) | self.pieces(PieceType::Queen)))
    }

    /// Enemy pieces giving check to the side to move.
    #[inline]
    pub fn checkers(&self) -> Bitboard {
        let king = self.king_square(self.stm);
        self.attackers_to(king, self.occupied()) & self.color_bb(self.stm.flip())
    }

    /// True if the side to move is in check.
    #[inline]
    pub fn in_check(&self) -> bool {
        self.checkers().any()
    }

    // --- internal piece manipulation (keeps bitboards, mailbox and key in sync) ---

    #[inline]
    fn add_piece(&mut self, pc: Piece, sq: Square) {
        let bb = Bitboard::from_square(sq);
        self.by_type[pc.piece_type().index()] |= bb;
        self.by_color[pc.color().index()] |= bb;
        self.mailbox[sq.index()] = Some(pc);
        self.key ^= ZOBRIST.piece[pc.index()][sq.index()];
    }

    #[inline]
    fn remove_piece(&mut self, sq: Square) -> Piece {
        let pc = self.mailbox[sq.index()].expect("remove_piece on empty square");
        let bb = Bitboard::from_square(sq);
        self.by_type[pc.piece_type().index()] ^= bb;
        self.by_color[pc.color().index()] ^= bb;
        self.mailbox[sq.index()] = None;
        self.key ^= ZOBRIST.piece[pc.index()][sq.index()];
        pc
    }

    #[inline]
    fn move_piece(&mut self, from: Square, to: Square) {
        let pc = self.mailbox[from.index()].expect("move_piece from empty square");
        let mask = Bitboard::from_square(from) | Bitboard::from_square(to);
        self.by_type[pc.piece_type().index()] ^= mask;
        self.by_color[pc.color().index()] ^= mask;
        self.mailbox[from.index()] = None;
        self.mailbox[to.index()] = Some(pc);
        self.key ^= ZOBRIST.piece[pc.index()][from.index()] ^ ZOBRIST.piece[pc.index()][to.index()];
    }

    // --- make / unmake ---

    /// Apply a legal move, returning the information needed to undo it.
    pub fn make_move(&mut self, mv: Move) -> Undo {
        let us = self.stm;
        let them = us.flip();
        let from = mv.from();
        let to = mv.to();
        let move_flag = mv.flag();
        let moving = self.mailbox[from.index()].expect("make_move from empty square");

        let saved_castling = self.castling;
        let saved_ep = self.ep_square;
        let saved_halfmove = self.halfmove;
        let saved_key = self.key;

        // Clear any existing en-passant square (and its hash contribution).
        if let Some(ep) = self.ep_square {
            self.key ^= ZOBRIST.ep_file[ep.file() as usize];
            self.ep_square = None;
        }

        let mut captured = None;

        match move_flag {
            flag::EN_PASSANT => {
                let cap_sq = Square::from_file_rank(to.file(), from.rank());
                captured = Some(self.remove_piece(cap_sq));
                self.move_piece(from, to);
            }
            flag::KING_CASTLE => {
                self.move_piece(from, to);
                let rank = from.rank();
                self.move_piece(
                    Square::from_file_rank(7, rank),
                    Square::from_file_rank(5, rank),
                );
            }
            flag::QUEEN_CASTLE => {
                self.move_piece(from, to);
                let rank = from.rank();
                self.move_piece(
                    Square::from_file_rank(0, rank),
                    Square::from_file_rank(3, rank),
                );
            }
            _ => {
                if mv.is_capture() {
                    captured = Some(self.remove_piece(to));
                }
                if let Some(promo) = mv.promotion() {
                    self.remove_piece(from);
                    self.add_piece(Piece::make(us, promo), to);
                } else {
                    self.move_piece(from, to);
                    if move_flag == flag::DOUBLE_PAWN {
                        let ep = Square::new(((from.index() + to.index()) / 2) as u8);
                        self.ep_square = Some(ep);
                        self.key ^= ZOBRIST.ep_file[ep.file() as usize];
                    }
                }
            }
        }

        // Update castling rights from the squares this move touched.
        let new_castling =
            CastlingRights(self.castling.0 & CASTLE_MASK[from.index()] & CASTLE_MASK[to.index()]);
        if new_castling != self.castling {
            self.key ^= ZOBRIST.castling[self.castling.index()];
            self.key ^= ZOBRIST.castling[new_castling.index()];
            self.castling = new_castling;
        }

        // Halfmove clock: reset on a pawn move or capture, otherwise advance.
        if moving.piece_type() == PieceType::Pawn || mv.is_capture() {
            self.halfmove = 0;
        } else {
            self.halfmove += 1;
        }

        if us == Color::Black {
            self.fullmove += 1;
        }

        self.stm = them;
        self.key ^= ZOBRIST.side;

        Undo {
            captured,
            castling: saved_castling,
            ep_square: saved_ep,
            halfmove: saved_halfmove,
            key: saved_key,
        }
    }

    /// Undo a move previously applied with [`make_move`](Self::make_move).
    pub fn unmake_move(&mut self, mv: Move, undo: Undo) {
        let them = self.stm; // color whose turn it became after the move
        let us = them.flip(); // color that made the move
        let from = mv.from();
        let to = mv.to();
        let move_flag = mv.flag();

        self.stm = us;
        if us == Color::Black {
            self.fullmove -= 1;
        }

        match move_flag {
            flag::EN_PASSANT => {
                self.move_piece(to, from);
                let cap_sq = Square::from_file_rank(to.file(), from.rank());
                self.add_piece(Piece::make(them, PieceType::Pawn), cap_sq);
            }
            flag::KING_CASTLE => {
                self.move_piece(to, from);
                let rank = from.rank();
                self.move_piece(
                    Square::from_file_rank(5, rank),
                    Square::from_file_rank(7, rank),
                );
            }
            flag::QUEEN_CASTLE => {
                self.move_piece(to, from);
                let rank = from.rank();
                self.move_piece(
                    Square::from_file_rank(3, rank),
                    Square::from_file_rank(0, rank),
                );
            }
            _ => {
                if mv.is_promotion() {
                    self.remove_piece(to);
                    self.add_piece(Piece::make(us, PieceType::Pawn), from);
                } else {
                    self.move_piece(to, from);
                }
                if mv.is_capture() {
                    self.add_piece(undo.captured.expect("capture undo missing piece"), to);
                }
            }
        }

        // Restore aggregate state wholesale; the piece edits above churned the
        // key, but this overwrite makes it exact again.
        self.castling = undo.castling;
        self.ep_square = undo.ep_square;
        self.halfmove = undo.halfmove;
        self.key = undo.key;
    }

    /// Apply a "null move" (pass the turn). For future search use; never part of
    /// legal move generation.
    pub fn make_null(&mut self) -> Undo {
        let saved = Undo {
            captured: None,
            castling: self.castling,
            ep_square: self.ep_square,
            halfmove: self.halfmove,
            key: self.key,
        };
        if let Some(ep) = self.ep_square {
            self.key ^= ZOBRIST.ep_file[ep.file() as usize];
            self.ep_square = None;
        }
        self.halfmove += 1;
        if self.stm == Color::Black {
            self.fullmove += 1;
        }
        self.stm = self.stm.flip();
        self.key ^= ZOBRIST.side;
        saved
    }

    /// Undo a [`make_null`](Self::make_null).
    pub fn unmake_null(&mut self, undo: Undo) {
        let us = self.stm.flip();
        if us == Color::Black {
            self.fullmove -= 1;
        }
        self.stm = us;
        self.castling = undo.castling;
        self.ep_square = undo.ep_square;
        self.halfmove = undo.halfmove;
        self.key = undo.key;
    }

    // --- FEN ---

    /// Recompute the Zobrist key from scratch (used after FEN parsing).
    fn compute_key(&self) -> u64 {
        let mut key = 0u64;
        for sq in 0..64 {
            if let Some(pc) = self.mailbox[sq] {
                key ^= ZOBRIST.piece[pc.index()][sq];
            }
        }
        if self.stm == Color::Black {
            key ^= ZOBRIST.side;
        }
        key ^= ZOBRIST.castling[self.castling.index()];
        if let Some(ep) = self.ep_square {
            key ^= ZOBRIST.ep_file[ep.file() as usize];
        }
        key
    }

    /// Parse a position from Forsyth–Edwards Notation. The halfmove and fullmove
    /// fields are optional and default to `0` and `1`.
    pub fn from_fen(fen: &str) -> Result<Board, FenError> {
        let mut fields = fen.split_whitespace();
        let placement = fields
            .next()
            .ok_or(FenError::MissingField("piece placement"))?;
        let side = fields
            .next()
            .ok_or(FenError::MissingField("side to move"))?;
        let castling = fields
            .next()
            .ok_or(FenError::MissingField("castling rights"))?;
        let ep = fields.next().ok_or(FenError::MissingField("en passant"))?;

        let mut board = Board::empty();

        // Piece placement: ranks listed from 8 down to 1.
        let ranks: Vec<&str> = placement.split('/').collect();
        if ranks.len() != 8 {
            return Err(FenError::BadPlacement("expected 8 ranks".into()));
        }
        for (i, rank_str) in ranks.iter().enumerate() {
            let rank = 7 - i as u8;
            let mut file = 0u8;
            for ch in rank_str.chars() {
                if let Some(skip) = ch.to_digit(10) {
                    file += skip as u8;
                } else {
                    let piece =
                        Piece::from_char(ch).ok_or(FenError::BadPlacement(format!("'{ch}'")))?;
                    if file >= 8 {
                        return Err(FenError::BadPlacement("rank too long".into()));
                    }
                    board.add_piece(piece, Square::from_file_rank(file, rank));
                    file += 1;
                }
            }
            if file != 8 {
                return Err(FenError::BadPlacement("rank wrong length".into()));
            }
        }

        board.stm = match side {
            "w" => Color::White,
            "b" => Color::Black,
            other => return Err(FenError::BadSideToMove(other.into())),
        };

        let mut rights = 0u8;
        if castling != "-" {
            for ch in castling.chars() {
                rights |= match ch {
                    'K' => CastlingRights::WHITE_KING,
                    'Q' => CastlingRights::WHITE_QUEEN,
                    'k' => CastlingRights::BLACK_KING,
                    'q' => CastlingRights::BLACK_QUEEN,
                    other => return Err(FenError::BadCastling(other.to_string())),
                };
            }
        }
        board.castling = CastlingRights(rights);

        board.ep_square = if ep == "-" {
            None
        } else {
            Some(Square::from_uci(ep).ok_or_else(|| FenError::BadEnPassant(ep.into()))?)
        };

        board.halfmove = match fields.next() {
            Some(s) => s
                .parse()
                .map_err(|_| FenError::BadNumber("halfmove clock", s.into()))?,
            None => 0,
        };
        board.fullmove = match fields.next() {
            Some(s) => s
                .parse()
                .map_err(|_| FenError::BadNumber("fullmove number", s.into()))?,
            None => 1,
        };

        board.key = board.compute_key();
        Ok(board)
    }

    /// Serialize the position to FEN.
    pub fn to_fen(&self) -> String {
        let mut s = String::new();
        for rank in (0..8).rev() {
            let mut empty = 0u8;
            for file in 0..8 {
                match self.mailbox[Square::from_file_rank(file, rank).index()] {
                    Some(pc) => {
                        if empty > 0 {
                            s.push_str(&empty.to_string());
                            empty = 0;
                        }
                        s.push(pc.to_char());
                    }
                    None => empty += 1,
                }
            }
            if empty > 0 {
                s.push_str(&empty.to_string());
            }
            if rank > 0 {
                s.push('/');
            }
        }
        s.push(' ');
        s.push(if self.stm == Color::White { 'w' } else { 'b' });
        s.push(' ');
        s.push_str(&self.castling.to_string());
        s.push(' ');
        match self.ep_square {
            Some(sq) => s.push_str(&sq.to_string()),
            None => s.push('-'),
        }
        s.push(' ');
        s.push_str(&self.halfmove.to_string());
        s.push(' ');
        s.push_str(&self.fullmove.to_string());
        s
    }
}

impl fmt::Display for Board {
    /// An ASCII diagram with rank 8 on top, plus the FEN and hash.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  +-----------------+")?;
        for rank in (0..8).rev() {
            write!(f, "{} | ", rank + 1)?;
            for file in 0..8 {
                let ch = match self.mailbox[Square::from_file_rank(file, rank).index()] {
                    Some(pc) => pc.to_char(),
                    None => '.',
                };
                write!(f, "{ch} ")?;
            }
            writeln!(f, "|")?;
        }
        writeln!(f, "  +-----------------+")?;
        writeln!(f, "    a b c d e f g h")?;
        writeln!(f, "FEN: {}", self.to_fen())?;
        write!(f, "Key: {:016x}", self.key)
    }
}

/// An error encountered while parsing FEN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FenError {
    /// A required field was absent.
    MissingField(&'static str),
    /// The piece-placement field was malformed.
    BadPlacement(String),
    /// The side-to-move field was not `w` or `b`.
    BadSideToMove(String),
    /// The castling field contained an unexpected character.
    BadCastling(String),
    /// The en-passant field was not `-` or a valid square.
    BadEnPassant(String),
    /// A numeric clock field could not be parsed.
    BadNumber(&'static str, String),
}

impl fmt::Display for FenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FenError::MissingField(name) => write!(f, "FEN missing field: {name}"),
            FenError::BadPlacement(msg) => write!(f, "invalid FEN placement: {msg}"),
            FenError::BadSideToMove(s) => write!(f, "invalid FEN side to move: {s}"),
            FenError::BadCastling(s) => write!(f, "invalid FEN castling: {s}"),
            FenError::BadEnPassant(s) => write!(f, "invalid FEN en passant: {s}"),
            FenError::BadNumber(field, s) => write!(f, "invalid FEN {field}: {s}"),
        }
    }
}

impl std::error::Error for FenError {}
