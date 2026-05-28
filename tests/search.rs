//! Search correctness: tactical solutions, draw handling, and limit compliance.

use mindless::board::Board;
use mindless::search::{search_sync, SearchLimits, MATE, MATE_IN_MAX};

fn depth_limit(depth: u32) -> SearchLimits {
    SearchLimits {
        depth: Some(depth),
        ..Default::default()
    }
}

fn root(board: &Board) -> Vec<u64> {
    vec![board.zobrist_key()]
}

#[test]
fn finds_mate_in_one() {
    // 1. Ra8# — the rook checks along the 8th rank and the king is boxed in by
    // its own pawns.
    let board = Board::from_fen("6k1/5ppp/8/8/8/8/8/R6K w - - 0 1").unwrap();
    let result = search_sync(&board, &root(&board), depth_limit(3));
    assert_eq!(result.best_move.to_uci(), "a1a8");
    assert!(
        result.score >= MATE_IN_MAX,
        "expected a mate score, got {}",
        result.score
    );
    assert_eq!(MATE - result.score, 1, "expected mate in one ply");
}

#[test]
fn finds_mate_in_two() {
    // King and queen vs lone king: forced mate in two, where the natural-looking
    // king approach (Kb6) is actually stalemate and must be avoided.
    let board = Board::from_fen("k7/8/8/2K5/8/8/8/1Q6 w - - 0 1").unwrap();
    let result = search_sync(&board, &root(&board), depth_limit(6));
    assert!(
        result.score >= MATE_IN_MAX,
        "expected a mate score, got {}",
        result.score
    );
    assert_eq!(
        MATE - result.score,
        3,
        "expected mate in three plies (two moves)"
    );
}

#[test]
fn wins_hanging_queen() {
    // Queens face off on the d-file; the black queen is undefended, so QxQ wins
    // a full queen while White's own queen survives (defended by its king).
    let board = Board::from_fen("4k3/8/8/8/3q4/8/8/3QK3 w - - 0 1").unwrap();
    let result = search_sync(&board, &root(&board), depth_limit(6));
    assert_eq!(
        result.best_move.to_uci(),
        "d1d4",
        "should capture the free queen"
    );
    assert!(
        result.score > 500,
        "should be up ~a queen, got {}",
        result.score
    );
}

#[test]
fn returns_no_move_in_stalemate() {
    // Side to move has no legal move and is not in check.
    let board = Board::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();
    let result = search_sync(&board, &root(&board), depth_limit(3));
    assert!(result.best_move.is_null());
}

#[test]
fn respects_node_limit() {
    let board = Board::startpos();
    let limits = SearchLimits {
        nodes: Some(50_000),
        ..Default::default()
    };
    let result = search_sync(&board, &root(&board), limits);
    assert!(!result.best_move.is_null());
    // A little overshoot is allowed (the limit is checked every 2048 nodes).
    assert!(
        result.nodes < 200_000,
        "node limit overrun: {}",
        result.nodes
    );
}

#[test]
fn respects_movetime() {
    let board = Board::startpos();
    let limits = SearchLimits {
        movetime: Some(100),
        ..Default::default()
    };
    let start = std::time::Instant::now();
    let result = search_sync(&board, &root(&board), limits);
    let elapsed = start.elapsed().as_millis();
    assert!(!result.best_move.is_null());
    // Generous bound: a search that ignored movetime would run for many seconds
    // (deepening toward MAX_PLY). The slack absorbs scheduling jitter when the
    // test suite runs in parallel.
    assert!(elapsed < 5000, "movetime not respected: {elapsed}ms");
}

#[test]
fn quiescence_terminates_on_tactical_position() {
    // A sharp middlegame with many captures must not hang quiescence search.
    let board =
        Board::from_fen("r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4")
            .unwrap();
    let result = search_sync(&board, &root(&board), depth_limit(6));
    assert!(!result.best_move.is_null());
    assert!(result.depth >= 6);
}
