//! Board-level correctness: FEN round-trips, make/unmake symmetry, incremental
//! Zobrist hashing, and checkmate / stalemate detection.

use mindless::board::Board;
use mindless::legal_moves;
use mindless::perft::SUITE;

const ROUNDTRIP_FENS: &[&str] = &[
    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
    "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
    "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
    // En-passant square present.
    "rnbqkbnr/ppp1pppp/8/3pP3/8/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 3",
    // Clocks well above the defaults.
    "8/8/8/4k3/8/4K3/8/8 w - - 49 120",
];

#[test]
fn fen_roundtrips() {
    for &fen in ROUNDTRIP_FENS {
        let board = Board::from_fen(fen).expect("valid FEN");
        assert_eq!(board.to_fen(), fen, "round-trip mismatch for {fen}");
    }
}

#[test]
fn fen_rejects_invalid() {
    assert!(Board::from_fen("").is_err());
    assert!(Board::from_fen("not a fen string").is_err());
    // Bad side-to-move field.
    assert!(Board::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR x KQkq - 0 1").is_err());
    // Only seven ranks.
    assert!(Board::from_fen("rnbqkbnr/pppppppp/8/8/8/PPPPPPPP/RNBQKBNR w - - 0 1").is_err());
}

/// Applying then undoing every move (recursively) must restore the position
/// bit-for-bit, including castling rights, en passant and the Zobrist key.
#[test]
fn make_unmake_is_symmetric() {
    fn walk(board: &mut Board, depth: u32) {
        if depth == 0 {
            return;
        }
        let snapshot = board.clone();
        for &mv in legal_moves(board).as_slice() {
            let undo = board.make_move(mv);
            walk(board, depth - 1);
            board.unmake_move(mv, undo);
            assert!(
                *board == snapshot,
                "make/unmake asymmetry on {}",
                mv.to_uci()
            );
        }
    }
    for pos in SUITE {
        let mut board = Board::from_fen(pos.fen).expect("valid suite FEN");
        walk(&mut board, 3);
    }
}

/// The incrementally-maintained Zobrist key must always equal the key computed
/// from scratch for the same position (obtained here via a FEN round-trip).
#[test]
fn zobrist_is_consistent() {
    fn walk(board: &mut Board, depth: u32) {
        if depth == 0 {
            return;
        }
        for &mv in legal_moves(board).as_slice() {
            let undo = board.make_move(mv);
            let recomputed = Board::from_fen(&board.to_fen()).expect("valid FEN");
            assert_eq!(
                board.zobrist_key(),
                recomputed.zobrist_key(),
                "Zobrist mismatch after {}",
                mv.to_uci()
            );
            walk(board, depth - 1);
            board.unmake_move(mv, undo);
        }
    }
    for pos in SUITE {
        let mut board = Board::from_fen(pos.fen).expect("valid suite FEN");
        walk(&mut board, 3);
    }
}

#[test]
fn detects_checkmate() {
    // Black king on h8 mated by a king-supported queen on g7.
    let board = Board::from_fen("7k/6Q1/6K1/8/8/8/8/8 b - - 0 1").expect("valid FEN");
    assert!(board.in_check());
    assert!(
        legal_moves(&board).is_empty(),
        "expected checkmate (no legal moves)"
    );
}

#[test]
fn detects_stalemate() {
    // Black king on h8 is not in check but has no legal move.
    let board = Board::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").expect("valid FEN");
    assert!(!board.in_check());
    assert!(
        legal_moves(&board).is_empty(),
        "expected stalemate (no legal moves)"
    );
}
