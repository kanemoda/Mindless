//! Perft (performance test): the move generator's correctness harness.
//!
//! `perft(depth)` counts the number of leaf nodes reachable in exactly `depth`
//! plies. Because it exercises every move-generation rule, matching the
//! published node counts for standard positions is the gold-standard proof that
//! generation, make and unmake are all correct.

use crate::board::Board;
use crate::movegen::{generate_legal, legal_moves};
use crate::moves::{Move, MoveList};

/// Count the leaf nodes at `depth` plies from `board`.
///
/// Uses bulk counting at depth 1 (the legal move count is the node count),
/// which is valid precisely because generation is fully legal.
pub fn perft(board: &mut Board, depth: u32) -> u64 {
    if depth == 0 {
        return 1;
    }
    let mut list = MoveList::new();
    generate_legal(board, &mut list);
    if depth == 1 {
        return list.len() as u64;
    }
    let mut nodes = 0;
    for &mv in list.as_slice() {
        let undo = board.make_move(mv);
        nodes += perft(board, depth - 1);
        board.unmake_move(mv, undo);
    }
    nodes
}

/// Perft "divide": the per-root-move node counts plus the total. Useful for
/// pinpointing a generation bug by comparing against a reference engine.
pub fn perft_divide(board: &mut Board, depth: u32) -> (Vec<(Move, u64)>, u64) {
    let mut results = Vec::new();
    let mut total = 0;
    if depth == 0 {
        return (results, 1);
    }
    let list = legal_moves(board);
    for &mv in list.as_slice() {
        let undo = board.make_move(mv);
        let count = perft(board, depth - 1);
        board.unmake_move(mv, undo);
        results.push((mv, count));
        total += count;
    }
    (results, total)
}

/// A reference position with known perft counts at successive depths.
pub struct PerftPosition {
    /// Human-readable name.
    pub name: &'static str,
    /// FEN of the position.
    pub fen: &'static str,
    /// `(depth, expected node count)` pairs, ascending by depth.
    pub counts: &'static [(u32, u64)],
}

/// The standard perft reference suite: the start position, "Kiwipete", and
/// Chess Programming Wiki positions 3–6. These collectively exercise castling,
/// en passant (including discovered-check edge cases), all four promotions,
/// pins and check evasions.
pub const SUITE: &[PerftPosition] = &[
    PerftPosition {
        name: "startpos",
        fen: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        counts: &[
            (1, 20),
            (2, 400),
            (3, 8902),
            (4, 197281),
            (5, 4865609),
            (6, 119060324),
        ],
    },
    PerftPosition {
        name: "kiwipete",
        fen: "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        counts: &[(1, 48), (2, 2039), (3, 97862), (4, 4085603), (5, 193690690)],
    },
    PerftPosition {
        name: "position3",
        fen: "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        counts: &[
            (1, 14),
            (2, 191),
            (3, 2812),
            (4, 43238),
            (5, 674624),
            (6, 11030083),
        ],
    },
    PerftPosition {
        name: "position4",
        fen: "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
        counts: &[(1, 6), (2, 264), (3, 9467), (4, 422333), (5, 15833292)],
    },
    PerftPosition {
        name: "position5",
        fen: "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
        counts: &[(1, 44), (2, 1486), (3, 62379), (4, 2103487), (5, 89941194)],
    },
    PerftPosition {
        name: "position6",
        fen: "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
        counts: &[(1, 46), (2, 2079), (3, 89890), (4, 3894594), (5, 164075551)],
    },
];
