//! The UCI (Universal Chess Interface) front-end.
//!
//! The search runs on its own thread so the engine stays responsive to `stop`
//! (and `go infinite`) while it thinks. The main thread reads commands, owns the
//! game position and the transposition table, and hands clones to each search.

use crate::board::Board;
use crate::movegen::legal_moves;
use crate::moves::Move;
use crate::perft::perft_divide;
use crate::search::{self, SearchLimits};
use crate::tt::Tt;
use crate::types::Square;
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

const ENGINE_NAME: &str = "Mindless";
const ENGINE_AUTHOR: &str = "kanemoda";
const DEFAULT_HASH_MB: usize = 64;
const MAX_HASH_MB: usize = 4096;
/// Generous stack for the recursive search thread.
const SEARCH_STACK: usize = 16 * 1024 * 1024;

/// Engine state held by the main UCI thread.
struct Engine {
    board: Board,
    /// Zobrist keys of every game position so far (ending with `board`), for
    /// repetition detection during search.
    history: Vec<u64>,
    tt: Arc<Tt>,
    stop: Arc<AtomicBool>,
    search: Option<JoinHandle<()>>,
    hash_mb: usize,
}

impl Engine {
    fn new() -> Engine {
        let board = Board::startpos();
        let history = vec![board.zobrist_key()];
        Engine {
            board,
            history,
            tt: Arc::new(Tt::new(DEFAULT_HASH_MB)),
            stop: Arc::new(AtomicBool::new(false)),
            search: None,
            hash_mb: DEFAULT_HASH_MB,
        }
    }

    /// Signal any running search to stop and wait for it to finish.
    fn stop_search(&mut self) {
        if let Some(handle) = self.search.take() {
            self.stop.store(true, Ordering::Relaxed);
            let _ = handle.join();
        }
    }

    fn new_game(&mut self) {
        self.stop_search();
        self.tt.clear();
        self.set_position(Board::startpos(), Vec::new());
    }

    fn set_position(&mut self, board: Board, moves: Vec<Move>) {
        let mut board = board;
        let mut history = vec![board.zobrist_key()];
        for mv in moves {
            board.make_move(mv);
            history.push(board.zobrist_key());
        }
        self.board = board;
        self.history = history;
    }
}

/// Run the UCI command loop until `quit` or end of input.
pub fn run() {
    crate::magic::init();
    let mut engine = Engine::new();

    let stdin = io::stdin();
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
            "uci" => print_id(),
            "isready" => println!("readyok"),
            "ucinewgame" => engine.new_game(),
            "setoption" => handle_setoption(&mut engine, &tokens),
            "position" => {
                engine.stop_search();
                handle_position(&mut engine, &tokens[1..]);
            }
            "go" => handle_go(&mut engine, &tokens[1..]),
            "stop" => engine.stop.store(true, Ordering::Relaxed),
            "ponderhit" => {}
            "d" | "print" => println!("{}", engine.board),
            "perft" => {
                if let Some(depth) = tokens.get(1).and_then(|s| s.parse().ok()) {
                    run_divide(&mut engine.board, depth);
                }
            }
            "quit" => {
                engine.stop_search();
                break;
            }
            _ => {}
        }
        let _ = io::stdout().flush();
    }
}

fn print_id() {
    println!("id name {ENGINE_NAME}");
    println!("id author {ENGINE_AUTHOR}");
    println!("option name Hash type spin default {DEFAULT_HASH_MB} min 1 max {MAX_HASH_MB}");
    println!("option name Threads type spin default 1 min 1 max 1");
    println!("option name Clear Hash type button");
    println!("uciok");
}

/// Handle `setoption name <Name> [value <Value>]`.
fn handle_setoption(engine: &mut Engine, tokens: &[&str]) {
    if tokens.get(1) != Some(&"name") {
        return;
    }
    let mut i = 2;
    let mut name_parts = Vec::new();
    while i < tokens.len() && tokens[i] != "value" {
        name_parts.push(tokens[i]);
        i += 1;
    }
    let name = name_parts.join(" ");
    let value = if i < tokens.len() && tokens[i] == "value" {
        tokens[i + 1..].join(" ")
    } else {
        String::new()
    };

    match name.as_str() {
        "Hash" => {
            if let Ok(mb) = value.parse::<usize>() {
                engine.stop_search();
                engine.hash_mb = mb.clamp(1, MAX_HASH_MB);
                engine.tt = Arc::new(Tt::new(engine.hash_mb));
            }
        }
        "Clear Hash" => {
            engine.stop_search();
            engine.tt.clear();
        }
        // Accepted for compatibility; the search is single-threaded for now.
        "Threads" => {}
        _ => {}
    }
}

/// Handle `position [startpos | fen <FEN>] [moves <m1> <m2> ...]`.
fn handle_position(engine: &mut Engine, tokens: &[&str]) {
    if tokens.is_empty() {
        return;
    }
    let (board, rest) = match tokens[0] {
        "startpos" => (Board::startpos(), &tokens[1..]),
        "fen" => {
            let mut end = 1;
            while end < tokens.len() && tokens[end] != "moves" {
                end += 1;
            }
            match Board::from_fen(&tokens[1..end].join(" ")) {
                Ok(b) => (b, &tokens[end..]),
                Err(_) => return,
            }
        }
        _ => return,
    };

    let mut applied = Vec::new();
    let mut scratch = board.clone();
    if rest.first() == Some(&"moves") {
        for &token in &rest[1..] {
            match parse_move(&scratch, token) {
                Some(mv) => {
                    scratch.make_move(mv);
                    applied.push(mv);
                }
                None => break,
            }
        }
    }
    engine.set_position(board, applied);
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

/// Handle `go`. `go perft N` runs perft divide synchronously; otherwise a search
/// is launched on a background thread.
fn handle_go(engine: &mut Engine, tokens: &[&str]) {
    if let Some(pos) = tokens.iter().position(|&t| t == "perft") {
        if let Some(depth) = tokens.get(pos + 1).and_then(|s| s.parse().ok()) {
            run_divide(&mut engine.board, depth);
        }
        return;
    }

    engine.stop_search();
    let limits = parse_limits(tokens);
    engine.stop.store(false, Ordering::Relaxed);

    let board = engine.board.clone();
    let history = engine.history.clone();
    let tt = Arc::clone(&engine.tt);
    let stop = Arc::clone(&engine.stop);

    let handle = thread::Builder::new()
        .stack_size(SEARCH_STACK)
        .spawn(move || {
            let best = search::think(board, history, tt, stop, limits);
            let mv = if best.is_null() {
                "0000".to_string()
            } else {
                best.to_uci()
            };
            println!("bestmove {mv}");
            let _ = io::stdout().flush();
        })
        .expect("failed to spawn search thread");
    engine.search = Some(handle);
}

/// Parse the parameters of a `go` command into [`SearchLimits`].
fn parse_limits(tokens: &[&str]) -> SearchLimits {
    let mut limits = SearchLimits::default();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            "depth" => limits.depth = tokens.get(i + 1).and_then(|s| s.parse().ok()),
            "nodes" => limits.nodes = tokens.get(i + 1).and_then(|s| s.parse().ok()),
            "movetime" => limits.movetime = tokens.get(i + 1).and_then(|s| s.parse().ok()),
            "wtime" => limits.wtime = tokens.get(i + 1).and_then(|s| s.parse().ok()),
            "btime" => limits.btime = tokens.get(i + 1).and_then(|s| s.parse().ok()),
            "winc" => limits.winc = tokens.get(i + 1).and_then(|s| s.parse().ok()),
            "binc" => limits.binc = tokens.get(i + 1).and_then(|s| s.parse().ok()),
            "movestogo" => limits.movestogo = tokens.get(i + 1).and_then(|s| s.parse().ok()),
            "infinite" => {
                limits.infinite = true;
                i += 1;
                continue;
            }
            _ => {
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    limits
}

/// Print a perft divide breakdown in the conventional format.
fn run_divide(board: &mut Board, depth: u32) {
    let (moves, total) = perft_divide(board, depth);
    for (mv, count) in &moves {
        println!("{}: {count}", mv.to_uci());
    }
    println!();
    println!("Nodes searched: {total}");
}
