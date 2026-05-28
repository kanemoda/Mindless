//! A `u64`-backed bitboard with the LERF square mapping (see [`crate::types`]).
//!
//! Bit `i` corresponds to the square with index `i`. The type is a thin newtype
//! over `u64` so the compiler keeps board sets distinct from other integers, and
//! it implements [`Iterator`] so set squares can be walked with a plain `for`
//! loop (`for sq in some_bitboard { ... }`).

use crate::types::Square;
use std::fmt;

/// A set of squares represented as a 64-bit mask.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Bitboard(pub u64);

// File masks. `FILE_A` is the a-file, `FILE_H` the h-file.
const FILE_A_BITS: u64 = 0x0101_0101_0101_0101;
const FILE_H_BITS: u64 = 0x8080_8080_8080_8080;
const NOT_FILE_A: u64 = !FILE_A_BITS;
const NOT_FILE_H: u64 = !FILE_H_BITS;

impl Bitboard {
    /// The empty set.
    pub const EMPTY: Bitboard = Bitboard(0);
    /// Every square set.
    pub const FULL: Bitboard = Bitboard(!0);

    /// The a-file (`a1..a8`).
    pub const FILE_A: Bitboard = Bitboard(FILE_A_BITS);
    /// The h-file (`h1..h8`).
    pub const FILE_H: Bitboard = Bitboard(FILE_H_BITS);

    /// Rank 1 (`a1..h1`).
    pub const RANK_1: Bitboard = Bitboard(0x0000_0000_0000_00FF);
    /// Rank 3 — white pawns reach it on a single push from the start.
    pub const RANK_3: Bitboard = Bitboard(0x0000_0000_00FF_0000);
    /// Rank 6 — black pawns reach it on a single push from the start.
    pub const RANK_6: Bitboard = Bitboard(0x0000_FF00_0000_0000);
    /// Rank 8 (`a8..h8`).
    pub const RANK_8: Bitboard = Bitboard(0xFF00_0000_0000_0000);

    /// A bitboard containing only `sq`.
    #[inline]
    pub const fn from_square(sq: Square) -> Bitboard {
        Bitboard(1u64 << sq.0)
    }

    /// True if no squares are set.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True if at least one square is set.
    #[inline]
    pub const fn any(self) -> bool {
        self.0 != 0
    }

    /// True if `sq` is a member of the set.
    #[inline]
    pub const fn contains(self, sq: Square) -> bool {
        self.0 & (1u64 << sq.0) != 0
    }

    /// Number of squares in the set (population count).
    #[inline]
    pub const fn count(self) -> u32 {
        self.0.count_ones()
    }

    /// The least-significant set square. Only valid when [`any`](Self::any).
    #[inline]
    pub const fn lsb(self) -> Square {
        Square(self.0.trailing_zeros() as u8)
    }

    /// Remove and return the least-significant set square.
    #[inline]
    pub fn pop_lsb(&mut self) -> Square {
        let sq = self.lsb();
        self.0 &= self.0 - 1;
        sq
    }

    // --- single-step shifts (used for pawn moves and table generation) ---
    // Each masks off the wrap-around file so squares never leak across an edge.

    /// Shift one rank toward rank 8.
    #[inline]
    pub const fn north(self) -> Bitboard {
        Bitboard(self.0 << 8)
    }
    /// Shift one rank toward rank 1.
    #[inline]
    pub const fn south(self) -> Bitboard {
        Bitboard(self.0 >> 8)
    }
    /// Shift one file toward the h-file.
    #[inline]
    pub const fn east(self) -> Bitboard {
        Bitboard((self.0 << 1) & NOT_FILE_A)
    }
    /// Shift one file toward the a-file.
    #[inline]
    pub const fn west(self) -> Bitboard {
        Bitboard((self.0 >> 1) & NOT_FILE_H)
    }
    /// Shift one square toward h8.
    #[inline]
    pub const fn north_east(self) -> Bitboard {
        Bitboard((self.0 << 9) & NOT_FILE_A)
    }
    /// Shift one square toward a8.
    #[inline]
    pub const fn north_west(self) -> Bitboard {
        Bitboard((self.0 << 7) & NOT_FILE_H)
    }
    /// Shift one square toward h1.
    #[inline]
    pub const fn south_east(self) -> Bitboard {
        Bitboard((self.0 >> 7) & NOT_FILE_A)
    }
    /// Shift one square toward a1.
    #[inline]
    pub const fn south_west(self) -> Bitboard {
        Bitboard((self.0 >> 9) & NOT_FILE_H)
    }
}

/// Iterating a bitboard yields its set squares from low index to high.
impl Iterator for Bitboard {
    type Item = Square;

    #[inline]
    fn next(&mut self) -> Option<Square> {
        if self.0 == 0 {
            None
        } else {
            Some(self.pop_lsb())
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.count() as usize;
        (n, Some(n))
    }
}

impl ExactSizeIterator for Bitboard {}

// --- bitwise operators ---

macro_rules! impl_binop {
    ($trait:ident, $method:ident, $assign:ident, $assign_method:ident, $op:tt) => {
        impl std::ops::$trait for Bitboard {
            type Output = Bitboard;
            #[inline]
            fn $method(self, rhs: Bitboard) -> Bitboard {
                Bitboard(self.0 $op rhs.0)
            }
        }
        impl std::ops::$assign for Bitboard {
            #[inline]
            fn $assign_method(&mut self, rhs: Bitboard) {
                self.0 = self.0 $op rhs.0;
            }
        }
    };
}

impl_binop!(BitAnd, bitand, BitAndAssign, bitand_assign, &);
impl_binop!(BitOr, bitor, BitOrAssign, bitor_assign, |);
impl_binop!(BitXor, bitxor, BitXorAssign, bitxor_assign, ^);

impl std::ops::Not for Bitboard {
    type Output = Bitboard;
    #[inline]
    fn not(self) -> Bitboard {
        Bitboard(!self.0)
    }
}

impl From<Square> for Bitboard {
    #[inline]
    fn from(sq: Square) -> Bitboard {
        Bitboard::from_square(sq)
    }
}

impl fmt::Debug for Bitboard {
    /// Renders the board with rank 8 on top, `.` for empty and `X` for set.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f)?;
        for rank in (0..8).rev() {
            for file in 0..8 {
                let sq = Square::from_file_rank(file, rank);
                write!(f, "{} ", if self.contains(sq) { 'X' } else { '.' })?;
            }
            writeln!(f)?;
        }
        Ok(())
    }
}
