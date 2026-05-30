//! NNUE evaluation: a small quantized neural network that replaces the
//! hand-crafted evaluation while sitting behind the same [`Evaluator`] trait.
//!
//! # Architecture
//!
//! The network is the standard "first real NNUE": a perspective feature
//! transformer feeding a single hidden layer,
//!
//! ```text
//! (768 -> HIDDEN) x2 -> 1
//! ```
//!
//! The 768 inputs are plain piece-square features (`64 * piece_type + square`,
//! six piece types per colour). There are **two** accumulators — one from
//! White's view and one from Black's — each the `HIDDEN`-vector produced by the
//! shared first layer. At evaluation time the side-to-move's accumulator is
//! "us" and the opponent's is "them"; the hidden values pass through a *squared
//! clipped ReLU* and the output layer combines them into one centipawn score.
//!
//! Storing the two accumulators by **absolute colour** (rather than us/them) is
//! what makes incremental updates clean: a move only changes the handful of
//! feature columns for the pieces that moved, with no per-ply perspective flip.
//!
//! # Matching the trainer
//!
//! The weights are trained by [bullet](https://github.com/jw1912/bullet) using
//! its `Chess768` input and the `examples/simple.rs` architecture. Every numeric
//! choice here — the feature indexing, the `QA`/`QB`/`SCALE` quantization
//! constants, the `quantised.bin` byte layout, and the exact integer order of
//! operations in [`Network::evaluate`] — mirrors that trainer so the engine's
//! evaluation matches the trainer's reference to the last centipawn. bullet
//! stores positions side-to-move-relative (mirroring the board when Black is to
//! move); evaluating the side-to-move accumulator as "us" reproduces that
//! convention exactly without ever mirroring the engine's board.

use crate::board::Board;
use crate::eval::Evaluator;
use crate::types::{Color, Piece, Square};

/// Size of the hidden layer (per perspective). Must match the trained net.
pub const HIDDEN: usize = 128;
/// Number of input features per perspective (`2 colours * 6 pieces * 64`).
pub const INPUT: usize = 768;

/// Feature-transformer quantization factor (`QA` in the trainer).
const QA: i32 = 255;
/// Output-layer quantization factor (`QB` in the trainer).
const QB: i32 = 64;
/// Evaluation scale (`SCALE` in the trainer): maps the network's `[0,1]`-ish
/// output back to centipawns.
const SCALE: i32 = 400;

/// Squared clipped ReLU activation, matching the trainer exactly: clamp the
/// (quantized) accumulator value to `[0, QA]` and square it, widening to `i32`.
#[inline]
fn screlu(x: i16) -> i32 {
    let y = (x as i32).clamp(0, QA);
    y * y
}

/// The quantized network weights, laid out to match bullet's `quantised.bin`.
///
/// The byte layout written by the trainer (all little-endian `i16`) is, in
/// order: the `HIDDEN x 768` feature weights in column-major order (768
/// consecutive `HIDDEN`-vectors, one per input feature), the `HIDDEN` feature
/// biases, the `2 * HIDDEN` output weights, and the single output bias. That is
/// exactly the field order below, so the file reads straight into this struct.
#[repr(C)]
pub struct Network {
    /// Feature weights, one `HIDDEN`-column per input feature (quantized by `QA`).
    feature_weights: [[i16; HIDDEN]; INPUT],
    /// Feature biases, the initial accumulator value (quantized by `QA`).
    feature_bias: [i16; HIDDEN],
    /// Output weights: first `HIDDEN` apply to "us", the rest to "them"
    /// (quantized by `QB`).
    output_weights: [i16; 2 * HIDDEN],
    /// Output bias (quantized by `QA * QB`).
    output_bias: i16,
}

/// Number of bytes in a serialized [`Network`] (used to validate a loaded file).
pub const NETWORK_BYTES: usize = std::mem::size_of::<Network>();

/// bullet pads `quantised.bin` up to a multiple of 64 bytes, so a valid file is
/// either exactly [`NETWORK_BYTES`] or that rounded up to the next multiple of
/// 64 (the extra bytes are trailing zero padding we ignore).
pub const NETWORK_BYTES_PADDED: usize = NETWORK_BYTES.next_multiple_of(64);

impl Network {
    /// Interpret a byte slice as a [`Network`]. Accepts a slice that is exactly
    /// [`NETWORK_BYTES`] or the 64-byte-padded [`NETWORK_BYTES_PADDED`] that
    /// bullet writes, reading only the first [`NETWORK_BYTES`]; returns `None`
    /// for any other length. The bytes are copied into a boxed network so the
    /// result is owned and correctly aligned.
    pub fn from_bytes(bytes: &[u8]) -> Option<Box<Network>> {
        if bytes.len() != NETWORK_BYTES && bytes.len() != NETWORK_BYTES_PADDED {
            return None;
        }
        let bytes = &bytes[..NETWORK_BYTES];
        // `Network` is `repr(C)` over `i16` arrays, for which every byte pattern
        // is a valid value; allocate zeroed (respecting alignment) and copy in.
        let mut net: Box<Network> = unsafe {
            let layout = std::alloc::Layout::new::<Network>();
            let ptr = std::alloc::alloc_zeroed(layout) as *mut Network;
            if ptr.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            Box::from_raw(ptr)
        };
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                (net.as_mut() as *mut Network) as *mut u8,
                NETWORK_BYTES,
            );
        }
        Some(net)
    }

    /// Evaluate from the two coloured accumulators with `stm` to move, returning
    /// a centipawn score from the side-to-move's perspective. The integer order
    /// of operations mirrors the trainer's reference inference exactly.
    pub fn evaluate(&self, white: &Accumulator, black: &Accumulator, stm: Color) -> i32 {
        let (us, them) = match stm {
            Color::White => (white, black),
            Color::Black => (black, white),
        };
        let mut output: i32 = 0;
        for i in 0..HIDDEN {
            output += screlu(us.vals[i]) * self.output_weights[i] as i32;
            output += screlu(them.vals[i]) * self.output_weights[HIDDEN + i] as i32;
        }
        // Undo one factor of QA from the squared activation, add the bias (which
        // is quantized by QA*QB), scale to centipawns, then remove QA*QB.
        output /= QA;
        output += self.output_bias as i32;
        output *= SCALE;
        output /= QA * QB;
        output
    }
}

/// One colour's accumulator: the feature transformer's `HIDDEN`-vector for a
/// position from that colour's perspective, kept incrementally up to date.
/// Aligned to 64 bytes so the hot add/remove loops vectorize cleanly.
#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub struct Accumulator {
    vals: [i16; HIDDEN],
}

impl Accumulator {
    /// A fresh accumulator initialized to the feature bias (an empty board).
    #[inline]
    fn new(net: &Network) -> Accumulator {
        Accumulator {
            vals: net.feature_bias,
        }
    }

    /// Add a feature column.
    #[inline]
    fn add(&mut self, feature: usize, net: &Network) {
        let col = &net.feature_weights[feature];
        for (v, &w) in self.vals.iter_mut().zip(col.iter()) {
            *v += w;
        }
    }

    /// Remove a feature column.
    #[inline]
    fn remove(&mut self, feature: usize, net: &Network) {
        let col = &net.feature_weights[feature];
        for (v, &w) in self.vals.iter_mut().zip(col.iter()) {
            *v -= w;
        }
    }
}

/// The White-perspective and Black-perspective feature indices for a piece on a
/// square, matching bullet's `Chess768` convention. White's perspective lists
/// the board upright; Black's mirrors it vertically (`square ^ 56`). In each
/// perspective the friendly pieces occupy feature block `0..384` and the enemy
/// pieces `384..768`, where a block is `6 piece types * 64 squares`.
///
/// Returns `(white_index, black_index)`.
#[inline]
pub fn feature_indices(piece: Piece, sq: Square) -> (usize, usize) {
    let pt = piece.piece_type().index();
    let s = sq.index();
    match piece.color() {
        Color::White => (64 * pt + s, 384 + 64 * pt + (s ^ 56)),
        Color::Black => (384 + 64 * pt + s, 64 * pt + (s ^ 56)),
    }
}

/// Both coloured accumulators for one position. The search keeps a stack of
/// these and updates them incrementally as moves are made and unmade.
#[derive(Clone, Copy)]
pub struct Accumulators {
    /// White's perspective.
    pub white: Accumulator,
    /// Black's perspective.
    pub black: Accumulator,
}

impl Accumulators {
    /// Build both accumulators from scratch for `board`.
    pub fn refresh(board: &Board, net: &Network) -> Accumulators {
        let mut acc = Accumulators {
            white: Accumulator::new(net),
            black: Accumulator::new(net),
        };
        for sq_idx in 0..64 {
            let sq = Square::new(sq_idx);
            if let Some(piece) = board.piece_on(sq) {
                acc.add_piece(piece, sq, net);
            }
        }
        acc
    }

    /// Add a piece to both perspectives.
    #[inline]
    pub fn add_piece(&mut self, piece: Piece, sq: Square, net: &Network) {
        let (w, b) = feature_indices(piece, sq);
        self.white.add(w, net);
        self.black.add(b, net);
    }

    /// Remove a piece from both perspectives.
    #[inline]
    pub fn remove_piece(&mut self, piece: Piece, sq: Square, net: &Network) {
        let (w, b) = feature_indices(piece, sq);
        self.white.remove(w, net);
        self.black.remove(b, net);
    }

    /// Evaluate `board`'s side to move from these accumulators.
    #[inline]
    pub fn evaluate(&self, stm: Color, net: &Network) -> i32 {
        net.evaluate(&self.white, &self.black, stm)
    }
}

/// An [`Evaluator`] backed by an NNUE [`Network`].
///
/// This trait implementation refreshes both accumulators from the board on each
/// call — correct, and how the static-eval interface is satisfied. The search's
/// hot path can instead keep accumulators incrementally; both produce identical
/// scores. The network lives on the heap (it is large) behind a shared handle so
/// cloning the evaluator is cheap.
#[derive(Clone)]
pub struct Nnue {
    net: std::sync::Arc<Network>,
}

impl Nnue {
    /// Build an evaluator from an in-memory network.
    pub fn new(net: std::sync::Arc<Network>) -> Nnue {
        Nnue { net }
    }

    /// Load a network from `quantised.bin` bytes.
    pub fn from_bytes(bytes: &[u8]) -> Option<Nnue> {
        Network::from_bytes(bytes).map(|net| Nnue {
            net: std::sync::Arc::from(net),
        })
    }

    /// Borrow the underlying network (shared handle).
    pub fn network(&self) -> &std::sync::Arc<Network> {
        &self.net
    }
}

impl Evaluator for Nnue {
    fn evaluate(&self, board: &Board) -> i32 {
        Accumulators::refresh(board, &self.net).evaluate(board.side_to_move(), &self.net)
    }
}

/// The evaluator the search actually runs, chosen at runtime: the hand-crafted
/// PeSTO evaluation (the default and fallback) or a loaded NNUE network. Keeping
/// both behind one enum lets the search stay generic over a single `Evaluator`
/// while the UCI layer swaps the active evaluation without any search changes.
#[derive(Clone)]
pub enum Eval {
    /// The classical hand-crafted evaluation.
    Hand(crate::eval::HandCrafted),
    /// A trained NNUE network.
    Net(Nnue),
}

/// The network the engine evaluates with by default, embedded directly in the
/// binary so `mindless` plays at full strength out of the box with no external
/// file. This is the committed `tools/nnue/nets/mindless-v1.nnue`; the UCI
/// `EvalFile` option overrides it with a different net at runtime.
pub const DEFAULT_NET: &[u8] = include_bytes!("../tools/nnue/nets/mindless-v1.nnue");

impl Eval {
    /// The hand-crafted PeSTO evaluator — the dependency-free fallback, and what
    /// the library's [`crate::search::search_sync`] convenience uses.
    pub fn hand() -> Eval {
        Eval::Hand(crate::eval::HandCrafted::new())
    }

    /// The engine's default evaluator: the embedded [`DEFAULT_NET`] NNUE network,
    /// falling back to the hand-crafted evaluation only if the embedded bytes
    /// fail to parse (guarded by a unit test, so that fallback should never fire).
    pub fn default_net() -> Eval {
        Nnue::from_bytes(DEFAULT_NET).map_or_else(Eval::hand, Eval::Net)
    }

    /// A human-readable name for the active evaluator (for UCI logging).
    pub fn name(&self) -> &'static str {
        match self {
            Eval::Hand(_) => "hand-crafted (PeSTO)",
            Eval::Net(_) => "NNUE",
        }
    }
}

impl Default for Eval {
    fn default() -> Eval {
        Eval::hand()
    }
}

impl Evaluator for Eval {
    #[inline]
    fn evaluate(&self, board: &Board) -> i32 {
        match self {
            Eval::Hand(e) => e.evaluate(board),
            Eval::Net(e) => e.evaluate(board),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movegen::legal_moves;

    #[test]
    fn feature_indices_match_trainer_convention() {
        // A White pawn on a1 (sq 0): white-perspective friendly block 0 → 0;
        // black-perspective enemy block → 384 + 0 + (0^56) = 440.
        assert_eq!(feature_indices(Piece::WhitePawn, Square::A1), (0, 440));

        // A Black king on h8 (sq 63, type 5): white-perspective enemy →
        // 384 + 320 + 63 = 767; black-perspective friendly → 320 + (63^56)=327.
        assert_eq!(feature_indices(Piece::BlackKing, Square::H8), (767, 327));
    }

    #[test]
    fn stm_relative_eval_is_perspective_symmetric() {
        // The starting position is symmetric, so with a real (random) net the
        // score must be identical from either side to move — because swapping
        // stm swaps which coloured accumulator is "us", and the two are mirror
        // images here. Use a deterministic pseudo-random net.
        let mut bytes = vec![0u8; NETWORK_BYTES];
        let mut state: u32 = 0x1234_5678;
        for b in bytes.iter_mut() {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *b = (state >> 24) as u8;
        }
        let net = Network::from_bytes(&bytes).unwrap();

        let white_stm =
            Board::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w - - 0 1").unwrap();
        let black_stm =
            Board::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b - - 0 1").unwrap();
        let sw = Accumulators::refresh(&white_stm, &net).evaluate(Color::White, &net);
        let sb = Accumulators::refresh(&black_stm, &net).evaluate(Color::Black, &net);
        assert_eq!(
            sw, sb,
            "symmetric position must evaluate equally for either stm"
        );
    }

    #[test]
    fn empty_network_evaluates_to_zero() {
        let net = Network::from_bytes(&vec![0u8; NETWORK_BYTES]).expect("zeroed net");
        let board = Board::startpos();
        let score = Accumulators::refresh(&board, &net).evaluate(board.side_to_move(), &net);
        assert_eq!(score, 0);
    }

    #[test]
    fn network_size_is_as_expected() {
        assert_eq!(
            NETWORK_BYTES,
            INPUT * HIDDEN * 2 + HIDDEN * 2 + 2 * HIDDEN * 2 + 2
        );
    }

    #[test]
    fn embedded_default_net_is_valid() {
        // The binary embeds `DEFAULT_NET`; it must be a correctly-sized network so
        // the engine's default evaluator is the NNUE, never the silent fallback.
        assert!(
            Network::from_bytes(DEFAULT_NET).is_some(),
            "embedded default net is {} bytes; expected {NETWORK_BYTES} or its padded size",
            DEFAULT_NET.len()
        );
        assert!(matches!(Eval::default_net(), Eval::Net(_)));
    }

    /// Walking a short game, the incremental add/remove of moved pieces must
    /// keep both accumulators bit-identical to a from-scratch refresh. This is
    /// the core correctness guarantee for the search's incremental path.
    #[test]
    fn incremental_matches_refresh_over_a_game() {
        // Deterministic pseudo-random net so adds/removes actually differ.
        let mut bytes = vec![0u8; NETWORK_BYTES];
        let mut state: u32 = 0xC0FF_EE11;
        for b in bytes.iter_mut() {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *b = (state >> 23) as u8;
        }
        let net = Network::from_bytes(&bytes).unwrap();

        let mut board = Board::startpos();
        let mut acc = Accumulators::refresh(&board, &net);

        // Play 30 deterministic "first legal move" plies, updating incrementally
        // by diffing the mailbox across each make_move, and compare to refresh.
        for _ in 0..30 {
            let moves = legal_moves(&board);
            if moves.is_empty() {
                break;
            }
            let mv = moves.as_slice()[0];

            // Snapshot occupancy+piece before, apply, snapshot after; the
            // symmetric difference is exactly the set of feature changes.
            let before: Vec<(Piece, Square)> = (0..64)
                .filter_map(|i| {
                    let sq = Square::new(i);
                    board.piece_on(sq).map(|p| (p, sq))
                })
                .collect();
            board.make_move(mv);
            let after: Vec<(Piece, Square)> = (0..64)
                .filter_map(|i| {
                    let sq = Square::new(i);
                    board.piece_on(sq).map(|p| (p, sq))
                })
                .collect();

            for &(p, sq) in &before {
                if !after.contains(&(p, sq)) {
                    acc.remove_piece(p, sq, &net);
                }
            }
            for &(p, sq) in &after {
                if !before.contains(&(p, sq)) {
                    acc.add_piece(p, sq, &net);
                }
            }

            let fresh = Accumulators::refresh(&board, &net);
            assert_eq!(
                acc.white.vals, fresh.white.vals,
                "white accumulator drifted from refresh"
            );
            assert_eq!(
                acc.black.vals, fresh.black.vals,
                "black accumulator drifted from refresh"
            );
        }
    }
}
