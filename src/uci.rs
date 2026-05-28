//! A minimal but correct UCI (Universal Chess Interface) front-end.
//!
//! Milestone 1 implements the protocol skeleton: a GUI can load the engine,
//! set up positions, and the engine replies to `go` with a legal move (chosen
//! at random for now — search arrives in a later milestone). The debugging
//! commands `d` and `perft N` are also supported.

use crate::board::Board;
use crate::movegen::legal_moves;
use crate::moves::Move;
use crate::perft::perft_divide;
use crate::types::Square;
use std::io::{self, BufRead, Write};
use std::time::{SystemTime, UNIX_EPOCH};

const ENGINE_NAME: &str = "Mindless";
const ENGINE_AUTHOR: &str = "kanemoda";

/// A tiny xorshift64 PRNG used to vary the engine's (currently random) moves.
struct Rng(u64);

impl Rng {
    fn new() -> Rng {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x1234_5678)
            | 1; // ensure nonzero
        Rng(seed)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// Run the UCI command loop, reading commands from stdin until `quit` or EOF.
pub fn run() {
    // Build the magic tables now so the first `go`/`perft` is not slowed by it.
    crate::magic::init();

    let stdin = io::stdin();
    let mut out = io::stdout();
    let mut board = Board::startpos();
    let mut rng = Rng::new();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(&cmd) = tokens.first() else {
            continue;
        };

        match cmd {
            "uci" => {
                let _ = writeln!(out, "id name {ENGINE_NAME}");
                let _ = writeln!(out, "id author {ENGINE_AUTHOR}");
                let _ = writeln!(out, "uciok");
            }
            "isready" => {
                let _ = writeln!(out, "readyok");
            }
            "ucinewgame" => {
                board = Board::startpos();
            }
            "position" => {
                handle_position(&mut board, &tokens[1..]);
            }
            "go" => {
                handle_go(&mut board, &mut rng, &tokens[1..], &mut out);
            }
            "stop" => {
                // No asynchronous search is running yet; nothing to stop.
            }
            "d" | "print" => {
                let _ = writeln!(out, "{board}");
            }
            "perft" => {
                if let Some(depth) = tokens.get(1).and_then(|s| s.parse::<u32>().ok()) {
                    run_divide(&mut board, depth, &mut out);
                }
            }
            "quit" => break,
            _ => {
                // Unknown / unsupported commands are ignored, per the protocol.
            }
        }
        let _ = out.flush();
    }
}

/// Parse `position [startpos | fen <FEN>] [moves <m1> <m2> ...]`.
fn handle_position(board: &mut Board, tokens: &[&str]) {
    if tokens.is_empty() {
        return;
    }

    let (mut new_board, rest) = match tokens[0] {
        "startpos" => (Board::startpos(), &tokens[1..]),
        "fen" => {
            // The FEN runs until the optional "moves" keyword or the end.
            let mut end = 1;
            while end < tokens.len() && tokens[end] != "moves" {
                end += 1;
            }
            let fen = tokens[1..end].join(" ");
            match Board::from_fen(&fen) {
                Ok(b) => (b, &tokens[end..]),
                Err(_) => return,
            }
        }
        _ => return,
    };

    if rest.first() == Some(&"moves") {
        for &token in &rest[1..] {
            match parse_move(&new_board, token) {
                Some(mv) => {
                    new_board.make_move(mv);
                }
                None => break, // stop at the first move we cannot interpret
            }
        }
    }

    *board = new_board;
}

/// Match a UCI move string against the legal moves of `board`.
fn parse_move(board: &Board, s: &str) -> Option<Move> {
    if s.len() < 4 {
        return None;
    }
    let from = Square::from_uci(&s[0..2])?;
    let to = Square::from_uci(&s[2..4])?;
    let want_promo = s
        .as_bytes()
        .get(4)
        .map(|b| (*b as char).to_ascii_lowercase());

    for &mv in legal_moves(board).as_slice() {
        if mv.from() == from && mv.to() == to {
            let mv_promo = mv.promotion().map(|pt| pt.to_char());
            if mv_promo == want_promo {
                return Some(mv);
            }
        }
    }
    None
}

/// Handle `go`. Supports `go perft N`; otherwise replies with a legal move.
fn handle_go(board: &mut Board, rng: &mut Rng, tokens: &[&str], out: &mut impl Write) {
    if let Some(pos) = tokens.iter().position(|&t| t == "perft") {
        if let Some(depth) = tokens.get(pos + 1).and_then(|s| s.parse::<u32>().ok()) {
            run_divide(board, depth, out);
        }
        return;
    }

    let moves = legal_moves(board);
    if moves.is_empty() {
        // Checkmate or stalemate: there is no move to make.
        let _ = writeln!(out, "bestmove 0000");
        return;
    }

    let index = (rng.next_u64() % moves.len() as u64) as usize;
    let mv = moves.as_slice()[index];
    let _ = writeln!(out, "bestmove {}", mv.to_uci());
}

/// Print a perft divide breakdown in the conventional format.
fn run_divide(board: &mut Board, depth: u32, out: &mut impl Write) {
    let (moves, total) = perft_divide(board, depth);
    for (mv, count) in &moves {
        let _ = writeln!(out, "{}: {}", mv.to_uci(), count);
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "Nodes searched: {total}");
}
