//! Perft correctness tests against the standard reference suite.
//!
//! The default test run (debug) verifies every reference depth whose node count
//! is small enough to stay fast. The exhaustive check, including the deepest
//! published counts, is gated behind `#[ignore]`; run it with:
//!
//! ```text
//! cargo test --release -- --ignored
//! ```

use mindless::board::Board;
use mindless::perft::{perft, SUITE};

/// Skip depths heavier than this in the default (debug) test run.
const FAST_NODE_LIMIT: u64 = 3_000_000;

#[test]
fn perft_suite_shallow() {
    for pos in SUITE {
        let mut board = Board::from_fen(pos.fen).expect("valid suite FEN");
        for &(depth, expected) in pos.counts {
            if expected > FAST_NODE_LIMIT {
                continue;
            }
            let nodes = perft(&mut board, depth);
            assert_eq!(nodes, expected, "{} at depth {depth}", pos.name);
        }
    }
}

#[test]
#[ignore = "exhaustive perft; run with `cargo test --release -- --ignored`"]
fn perft_suite_deep() {
    for pos in SUITE {
        let mut board = Board::from_fen(pos.fen).expect("valid suite FEN");
        for &(depth, expected) in pos.counts {
            let nodes = perft(&mut board, depth);
            assert_eq!(nodes, expected, "{} at depth {depth}", pos.name);
        }
    }
}
