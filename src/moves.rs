//! Compact 16-bit move encoding and a fixed-capacity move list.
//!
//! A [`Move`] packs the origin square, destination square and a 4-bit flag:
//!
//! ```text
//! bits  0..6   from square (0..64)
//! bits  6..12  to square   (0..64)
//! bits 12..16  flag        (see the `flag` module)
//! ```
//!
//! The flag layout follows the common Chess Programming Wiki scheme, where bit
//! 2 marks captures, bit 3 marks promotions, and the low two bits of a
//! promotion select the piece. This makes `is_capture` / `is_promotion`
//! single-bit tests.

use crate::types::{PieceType, Square};
use std::fmt;

/// Move flag values stored in the top nibble of a [`Move`].
pub mod flag {
    /// A non-capturing, non-special move.
    pub const QUIET: u16 = 0b0000;
    /// A pawn's initial two-square advance.
    pub const DOUBLE_PAWN: u16 = 0b0001;
    /// Kingside castling (`O-O`).
    pub const KING_CASTLE: u16 = 0b0010;
    /// Queenside castling (`O-O-O`).
    pub const QUEEN_CASTLE: u16 = 0b0011;
    /// A capture that is not a promotion or en passant.
    pub const CAPTURE: u16 = 0b0100;
    /// An en-passant capture.
    pub const EN_PASSANT: u16 = 0b0101;
    /// Promotion to knight (no capture).
    pub const PROMO_KNIGHT: u16 = 0b1000;
    /// Promotion to bishop (no capture).
    pub const PROMO_BISHOP: u16 = 0b1001;
    /// Promotion to rook (no capture).
    pub const PROMO_ROOK: u16 = 0b1010;
    /// Promotion to queen (no capture).
    pub const PROMO_QUEEN: u16 = 0b1011;
    /// Capture with promotion to knight.
    pub const PROMO_KNIGHT_CAPTURE: u16 = 0b1100;
    /// Capture with promotion to bishop.
    pub const PROMO_BISHOP_CAPTURE: u16 = 0b1101;
    /// Capture with promotion to rook.
    pub const PROMO_ROOK_CAPTURE: u16 = 0b1110;
    /// Capture with promotion to queen.
    pub const PROMO_QUEEN_CAPTURE: u16 = 0b1111;

    /// Bit set on every capturing flag (plain, en passant, promotion capture).
    pub const CAPTURE_BIT: u16 = 0b0100;
    /// Bit set on every promotion flag.
    pub const PROMO_BIT: u16 = 0b1000;
}

/// A chess move encoded in 16 bits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Move(u16);

impl Move {
    /// A sentinel "no move" value (encodes a1->a1, used as a null move marker).
    pub const NULL: Move = Move(0);

    /// Build a move from origin, destination and a [`flag`] value.
    #[inline]
    pub const fn new(from: Square, to: Square, flag: u16) -> Move {
        Move((from.0 as u16) | ((to.0 as u16) << 6) | (flag << 12))
    }

    /// Origin square.
    #[inline]
    pub const fn from(self) -> Square {
        Square((self.0 & 0x3F) as u8)
    }

    /// Destination square.
    #[inline]
    pub const fn to(self) -> Square {
        Square(((self.0 >> 6) & 0x3F) as u8)
    }

    /// The 4-bit flag.
    #[inline]
    pub const fn flag(self) -> u16 {
        self.0 >> 12
    }

    /// The raw 16-bit encoding (for serialization, e.g. into a hash table).
    #[inline]
    pub const fn to_bits(self) -> u16 {
        self.0
    }

    /// Reconstruct a move from its raw 16-bit encoding.
    #[inline]
    pub const fn from_bits(bits: u16) -> Move {
        Move(bits)
    }

    /// True if this move captures (including en passant and promotion captures).
    #[inline]
    pub const fn is_capture(self) -> bool {
        self.flag() & flag::CAPTURE_BIT != 0
    }

    /// True if this move promotes a pawn.
    #[inline]
    pub const fn is_promotion(self) -> bool {
        self.flag() & flag::PROMO_BIT != 0
    }

    /// True if this move is an en-passant capture.
    #[inline]
    pub const fn is_en_passant(self) -> bool {
        self.flag() == flag::EN_PASSANT
    }

    /// True if this move is a double pawn push.
    #[inline]
    pub const fn is_double_pawn(self) -> bool {
        self.flag() == flag::DOUBLE_PAWN
    }

    /// True if this move castles.
    #[inline]
    pub const fn is_castle(self) -> bool {
        matches!(self.flag(), flag::KING_CASTLE | flag::QUEEN_CASTLE)
    }

    /// True if this move is the [`NULL`](Self::NULL) sentinel.
    #[inline]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }

    /// The promoted-to piece kind, if this is a promotion.
    #[inline]
    pub const fn promotion(self) -> Option<PieceType> {
        if self.is_promotion() {
            // Low two flag bits select knight/bishop/rook/queen.
            Some(PieceType::from_index((self.flag() & 0b11) as usize + 1))
        } else {
            None
        }
    }

    /// Render in UCI long algebraic notation (e.g. `e2e4`, `e7e8q`, `0000`).
    pub fn to_uci(self) -> String {
        if self.is_null() {
            return "0000".to_string();
        }
        let mut s = format!("{}{}", self.from(), self.to());
        if let Some(promo) = self.promotion() {
            s.push(promo.to_char());
        }
        s
    }
}

impl fmt::Display for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_uci())
    }
}

impl fmt::Debug for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_uci())
    }
}

/// Maximum number of legal moves in any chess position is 218; round up to a
/// comfortable power of two so the list never needs to allocate.
const MAX_MOVES: usize = 256;

/// A stack-allocated, fixed-capacity list of moves.
///
/// Generating into a `MoveList` avoids heap traffic on the search hot path. It
/// dereferences to `&[Move]`, so slice methods (`iter`, indexing, `len`) work
/// directly.
pub struct MoveList {
    moves: [Move; MAX_MOVES],
    len: usize,
}

impl MoveList {
    /// Create an empty list.
    #[inline]
    pub fn new() -> MoveList {
        MoveList {
            moves: [Move::NULL; MAX_MOVES],
            len: 0,
        }
    }

    /// Append a move.
    #[inline]
    pub fn push(&mut self, m: Move) {
        debug_assert!(self.len < MAX_MOVES);
        self.moves[self.len] = m;
        self.len += 1;
    }

    /// Number of moves in the list.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True if the list is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The moves as a slice.
    #[inline]
    pub fn as_slice(&self) -> &[Move] {
        &self.moves[..self.len]
    }

    /// The moves as a mutable slice (used for in-place move ordering).
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [Move] {
        &mut self.moves[..self.len]
    }
}

impl Default for MoveList {
    fn default() -> MoveList {
        MoveList::new()
    }
}

impl std::ops::Deref for MoveList {
    type Target = [Move];
    #[inline]
    fn deref(&self) -> &[Move] {
        self.as_slice()
    }
}

impl<'a> IntoIterator for &'a MoveList {
    type Item = &'a Move;
    type IntoIter = std::slice::Iter<'a, Move>;
    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}
