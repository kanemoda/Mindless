//! Fundamental chess value types: colors, piece kinds, pieces, squares and
//! castling rights.
//!
//! Squares use the Little-Endian Rank-File (LERF) mapping that is standard for
//! bitboard engines: `a1 = 0`, `b1 = 1`, ... `h1 = 7`, `a2 = 8`, ... `h8 = 63`.
//! With this layout `file = index & 7` and `rank = index >> 3`, and a square's
//! bit in a [`Bitboard`](crate::bitboard::Bitboard) is simply `1 << index`.

use std::fmt;

/// Side to move / piece color.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum Color {
    White = 0,
    Black = 1,
}

impl Color {
    /// Number of colors.
    pub const COUNT: usize = 2;

    /// Index into color-keyed arrays (`White = 0`, `Black = 1`).
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// The opposing color.
    #[inline]
    pub const fn flip(self) -> Color {
        match self {
            Color::White => Color::Black,
            Color::Black => Color::White,
        }
    }

    /// Build a color from an index (`0 = White`, anything else `Black`).
    #[inline]
    pub const fn from_index(i: usize) -> Color {
        match i {
            0 => Color::White,
            _ => Color::Black,
        }
    }
}

/// The six kinds of chess piece, independent of color.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum PieceType {
    Pawn = 0,
    Knight = 1,
    Bishop = 2,
    Rook = 3,
    Queen = 4,
    King = 5,
}

impl PieceType {
    /// Number of distinct piece kinds.
    pub const COUNT: usize = 6;

    /// All piece kinds in index order.
    pub const ALL: [PieceType; 6] = [
        PieceType::Pawn,
        PieceType::Knight,
        PieceType::Bishop,
        PieceType::Rook,
        PieceType::Queen,
        PieceType::King,
    ];

    /// Index into piece-keyed arrays (`Pawn = 0` ... `King = 5`).
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Build a piece kind from an index in `0..6`.
    #[inline]
    pub const fn from_index(i: usize) -> PieceType {
        match i {
            0 => PieceType::Pawn,
            1 => PieceType::Knight,
            2 => PieceType::Bishop,
            3 => PieceType::Rook,
            4 => PieceType::Queen,
            5 => PieceType::King,
            _ => panic!("piece-type index out of range"),
        }
    }

    /// Lowercase piece letter as used in FEN / UCI promotion suffixes.
    #[inline]
    pub const fn to_char(self) -> char {
        match self {
            PieceType::Pawn => 'p',
            PieceType::Knight => 'n',
            PieceType::Bishop => 'b',
            PieceType::Rook => 'r',
            PieceType::Queen => 'q',
            PieceType::King => 'k',
        }
    }
}

/// A colored piece (color and kind combined).
///
/// The discriminant equals `color.index() * 6 + piece_type.index()`, so white
/// pieces occupy indices `0..6` and black pieces `6..12`. This keeps Zobrist
/// and other piece-keyed tables compact and cache friendly.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum Piece {
    WhitePawn = 0,
    WhiteKnight = 1,
    WhiteBishop = 2,
    WhiteRook = 3,
    WhiteQueen = 4,
    WhiteKing = 5,
    BlackPawn = 6,
    BlackKnight = 7,
    BlackBishop = 8,
    BlackRook = 9,
    BlackQueen = 10,
    BlackKing = 11,
}

impl Piece {
    /// Number of distinct colored pieces.
    pub const COUNT: usize = 12;

    /// Compose a colored piece from a color and a kind.
    #[inline]
    pub const fn make(color: Color, kind: PieceType) -> Piece {
        Piece::from_index(color.index() * 6 + kind.index())
    }

    /// Index into piece-keyed arrays (`0..12`).
    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Build a colored piece from an index in `0..12`.
    #[inline]
    pub const fn from_index(i: usize) -> Piece {
        match i {
            0 => Piece::WhitePawn,
            1 => Piece::WhiteKnight,
            2 => Piece::WhiteBishop,
            3 => Piece::WhiteRook,
            4 => Piece::WhiteQueen,
            5 => Piece::WhiteKing,
            6 => Piece::BlackPawn,
            7 => Piece::BlackKnight,
            8 => Piece::BlackBishop,
            9 => Piece::BlackRook,
            10 => Piece::BlackQueen,
            11 => Piece::BlackKing,
            _ => panic!("piece index out of range"),
        }
    }

    /// The color of this piece.
    #[inline]
    pub const fn color(self) -> Color {
        if (self as usize) < 6 {
            Color::White
        } else {
            Color::Black
        }
    }

    /// The kind of this piece.
    #[inline]
    pub const fn piece_type(self) -> PieceType {
        PieceType::from_index((self as usize) % 6)
    }

    /// FEN letter for this piece (uppercase for white, lowercase for black).
    #[inline]
    pub const fn to_char(self) -> char {
        let lower = self.piece_type().to_char();
        match self.color() {
            Color::White => lower.to_ascii_uppercase(),
            Color::Black => lower,
        }
    }

    /// Parse a FEN piece letter into a colored piece.
    #[inline]
    pub const fn from_char(c: char) -> Option<Piece> {
        let (color, lower) = if c.is_ascii_uppercase() {
            (Color::White, c.to_ascii_lowercase())
        } else {
            (Color::Black, c)
        };
        let kind = match lower {
            'p' => PieceType::Pawn,
            'n' => PieceType::Knight,
            'b' => PieceType::Bishop,
            'r' => PieceType::Rook,
            'q' => PieceType::Queen,
            'k' => PieceType::King,
            _ => return None,
        };
        Some(Piece::make(color, kind))
    }
}

/// A board square, `0..64` in LERF order (`a1 = 0` ... `h8 = 63`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Square(pub u8);

impl Square {
    /// Number of squares on the board.
    pub const COUNT: usize = 64;

    /// Construct a square from a raw `0..64` index.
    #[inline]
    pub const fn new(index: u8) -> Square {
        Square(index)
    }

    /// Construct a square from file (`0..8`, a-h) and rank (`0..8`, 1-8).
    #[inline]
    pub const fn from_file_rank(file: u8, rank: u8) -> Square {
        Square(rank * 8 + file)
    }

    /// The square's `0..64` index.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// File of the square (`0 = a` ... `7 = h`).
    #[inline]
    pub const fn file(self) -> u8 {
        self.0 & 7
    }

    /// Rank of the square (`0 = rank 1` ... `7 = rank 8`).
    #[inline]
    pub const fn rank(self) -> u8 {
        self.0 >> 3
    }

    /// Parse a square from algebraic notation such as `"e4"`.
    pub fn from_uci(s: &str) -> Option<Square> {
        let bytes = s.as_bytes();
        if bytes.len() != 2 {
            return None;
        }
        let file = bytes[0];
        let rank = bytes[1];
        if !(b'a'..=b'h').contains(&file) || !(b'1'..=b'8').contains(&rank) {
            return None;
        }
        Some(Square::from_file_rank(file - b'a', rank - b'1'))
    }

    // Named squares used by castling logic.
    pub const A1: Square = Square(0);
    pub const B1: Square = Square(1);
    pub const C1: Square = Square(2);
    pub const D1: Square = Square(3);
    pub const E1: Square = Square(4);
    pub const F1: Square = Square(5);
    pub const G1: Square = Square(6);
    pub const H1: Square = Square(7);
    pub const A8: Square = Square(56);
    pub const B8: Square = Square(57);
    pub const C8: Square = Square(58);
    pub const D8: Square = Square(59);
    pub const E8: Square = Square(60);
    pub const F8: Square = Square(61);
    pub const G8: Square = Square(62);
    pub const H8: Square = Square(63);
}

impl fmt::Display for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let file = (b'a' + self.file()) as char;
        let rank = (b'1' + self.rank()) as char;
        write!(f, "{file}{rank}")
    }
}

impl fmt::Debug for Square {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

/// Castling availability, stored as four independent bit flags.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CastlingRights(pub u8);

impl CastlingRights {
    /// No castling rights.
    pub const NONE: CastlingRights = CastlingRights(0);
    /// White kingside (`O-O`).
    pub const WHITE_KING: u8 = 0b0001;
    /// White queenside (`O-O-O`).
    pub const WHITE_QUEEN: u8 = 0b0010;
    /// Black kingside.
    pub const BLACK_KING: u8 = 0b0100;
    /// Black queenside.
    pub const BLACK_QUEEN: u8 = 0b1000;

    /// True if the given flag (e.g. [`CastlingRights::WHITE_KING`]) is set.
    #[inline]
    pub const fn has(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    /// Raw bits, also used to index the 16-entry Zobrist castling table.
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for CastlingRights {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == 0 {
            return write!(f, "-");
        }
        if self.has(Self::WHITE_KING) {
            write!(f, "K")?;
        }
        if self.has(Self::WHITE_QUEEN) {
            write!(f, "Q")?;
        }
        if self.has(Self::BLACK_KING) {
            write!(f, "k")?;
        }
        if self.has(Self::BLACK_QUEEN) {
            write!(f, "q")?;
        }
        Ok(())
    }
}
