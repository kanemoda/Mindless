//! # Mindless
//!
//! Mindless is a UCI chess engine written in Rust. This crate is both the
//! engine binary and a reusable library: the binary in `main.rs` is a thin
//! wrapper around [`uci::run`], while everything needed to represent positions
//! and generate moves is exposed here for other Rust projects to build on.
//!
//! ## Quick start
//!
//! ```
//! use mindless::{Board, legal_moves};
//! use mindless::perft::perft;
//!
//! // Set up the standard starting position.
//! let mut board = Board::startpos();
//!
//! // Generate the 20 legal opening moves.
//! let moves = legal_moves(&board);
//! assert_eq!(moves.len(), 20);
//!
//! // Verify a small perft count.
//! assert_eq!(perft(&mut board, 3), 8902);
//! ```
//!
//! ## Module map
//!
//! * [`types`] — colors, pieces, squares, castling rights.
//! * [`bitboard`] — the `u64` board-set type and its operations.
//! * [`board`] — the [`Board`] position: state, FEN, make/unmake, hashing.
//! * [`moves`] — the compact 16-bit [`Move`] encoding and the move list.
//! * [`movegen`] — fully legal move generation.
//! * [`magic`] / [`attacks`] — sliding and leaping attack tables.
//! * [`zobrist`] — incremental position-hash keys.
//! * [`perft`] — the correctness/performance harness.
//! * [`uci`] — the UCI protocol front-end.

pub mod attacks;
pub mod bitboard;
pub mod board;
pub mod eval;
pub mod magic;
pub mod movegen;
pub mod moves;
pub mod perft;
pub mod search;
pub mod see;
pub mod tt;
pub mod types;
pub mod uci;
pub mod zobrist;

// A clean, minimal surface for crate consumers. The perft entry points keep
// their module path (`mindless::perft::perft`) to avoid shadowing the module.
pub use bitboard::Bitboard;
pub use board::{Board, FenError, STARTPOS_FEN};
pub use eval::{Evaluator, HandCrafted};
pub use movegen::{generate_legal, generate_noisy, legal_moves};
pub use moves::{Move, MoveList};
pub use search::{think, SearchLimits};
pub use tt::Tt;
pub use types::{CastlingRights, Color, Piece, PieceType, Square};
