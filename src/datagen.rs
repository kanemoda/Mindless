//! Training-data generation by self-play.
//!
//! This produces labelled positions for NNUE training in the simple text format
//! the [bullet](https://github.com/jw1912/bullet) trainer ingests: one position
//! per line,
//!
//! ```text
//! <FEN> | <score> | <result>
//! ```
//!
//! where `score` is the engine's search evaluation in centipawns **from White's
//! point of view** and `result` is the eventual game result, also from White
//! (`1.0` win, `0.5` draw, `0.0` loss).
//!
//! # How a game is generated
//!
//! Each game starts from the standard position, is diversified by a handful of
//! uniformly-random legal opening plies, then played out by the engine against
//! itself at a small fixed node budget per move. After the game ends (mate,
//! draw, or adjudication) every *recorded* position is written with that game's
//! result attached.
//!
//! # Filtering
//!
//! Only "quiet, settled" positions make good evaluation targets, so a position
//! is recorded only when it is past the random opening, the side to move is not
//! in check, the best move is not a capture or promotion, and the score is not a
//! (near-)mate. This mirrors the standard filtering used by engine data
//! generators and keeps the network from training on tactical noise that the
//! quiescence search is responsible for, not the static evaluation.
//!
//! The work is split across threads, each generating games independently into
//! its own part file; the parts are concatenated into the final output at the
//! end.

use crate::board::Board;
use crate::movegen::legal_moves;
use crate::nnue::Eval;
use crate::search::{search_sync_tt, SearchLimits, MATE_IN_MAX};
use crate::tt::Tt;
use crate::types::{Color, PieceType};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Default number of games to generate.
const DEFAULT_GAMES: u64 = 5_000;
/// Default per-move node budget for the labelling search.
const DEFAULT_NODES: u64 = 5_000;
/// Default number of random opening plies used to diversify games.
const DEFAULT_RANDOM_PLIES: u32 = 8;
/// Hard cap on game length (plies) before adjudicating a draw.
const MAX_GAME_PLIES: u32 = 320;
/// Skip recording near-decisive/mate scores above this magnitude (centipawns).
const SCORE_RECORD_CAP: i32 = 8_000;
/// Resign adjudication: a side leading by at least this for
/// [`ADJUDICATE_PLIES`] consecutive plies wins.
const RESIGN_CP: i32 = 2_000;
/// Draw adjudication: scores within this band for [`ADJUDICATE_PLIES`]
/// consecutive plies after move 30 are called a draw.
const DRAW_CP: i32 = 8;
/// Consecutive plies an adjudication condition must hold.
const ADJUDICATE_PLIES: u32 = 6;
/// Ply after which draw adjudication is allowed.
const DRAW_ADJ_MIN_PLY: u32 = 60;

/// A small, fast pseudo-random generator (SplitMix64) for opening randomization.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    #[inline]
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// Parsed `datagen` command-line options.
struct Config {
    games: u64,
    nodes: u64,
    random_plies: u32,
    threads: usize,
    out: String,
}

impl Config {
    fn parse(args: &[String]) -> Config {
        let mut cfg = Config {
            games: DEFAULT_GAMES,
            nodes: DEFAULT_NODES,
            random_plies: DEFAULT_RANDOM_PLIES,
            threads: (std::thread::available_parallelism().map_or(4, |n| n.get()))
                .saturating_sub(1),
            out: "data/mindless.txt".to_string(),
        };
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--games" => {
                    cfg.games = next_arg(args, &mut i)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(cfg.games)
                }
                "--nodes" => {
                    cfg.nodes = next_arg(args, &mut i)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(cfg.nodes)
                }
                "--random-plies" => {
                    cfg.random_plies = next_arg(args, &mut i)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(cfg.random_plies)
                }
                "--threads" => {
                    cfg.threads = next_arg(args, &mut i)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(cfg.threads)
                        .max(1)
                }
                "--out" => {
                    if let Some(s) = next_arg(args, &mut i) {
                        cfg.out = s.to_string();
                    }
                }
                _ => {}
            }
            i += 1;
        }
        cfg.threads = cfg.threads.max(1);
        cfg
    }
}

fn next_arg<'a>(args: &'a [String], i: &mut usize) -> Option<&'a str> {
    if *i + 1 < args.len() {
        *i += 1;
        Some(args[*i].as_str())
    } else {
        None
    }
}

/// Entry point for `mindless datagen [...]`.
pub fn run(args: &[String]) {
    let cfg = Config::parse(args);

    // Ensure the output directory exists.
    if let Some(dir) = std::path::Path::new(&cfg.out).parent() {
        if !dir.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(dir);
        }
    }

    eprintln!(
        "datagen: games={} nodes/move={} random_plies={} threads={} out={}",
        cfg.games, cfg.nodes, cfg.random_plies, cfg.threads, cfg.out
    );

    let base_seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x1234_5678_9ABC_DEF0);

    let positions = Arc::new(AtomicU64::new(0));
    let games_done = Arc::new(AtomicU64::new(0));
    let start = Instant::now();

    let cfg = Arc::new(cfg);
    let mut handles = Vec::new();
    for t in 0..cfg.threads {
        let cfg = Arc::clone(&cfg);
        let positions = Arc::clone(&positions);
        let games_done = Arc::clone(&games_done);
        // Spread games across threads (the last thread takes the remainder).
        let share = cfg.games / cfg.threads as u64
            + if (t as u64) < cfg.games % cfg.threads as u64 {
                1
            } else {
                0
            };
        let seed = base_seed ^ ((t as u64).wrapping_mul(0xD1B5_4A32_D192_ED03));
        let part_path = format!("{}.part{t}", cfg.out);
        handles.push(std::thread::spawn(move || {
            worker(&cfg, share, seed, &part_path, &positions, &games_done);
            part_path
        }));
    }

    // Progress reporting on the main thread.
    let total_games = cfg.games;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let g = games_done.load(Ordering::Relaxed);
        let p = positions.load(Ordering::Relaxed);
        let secs = start.elapsed().as_secs_f64().max(0.001);
        eprintln!(
            "  {g}/{total_games} games, {p} positions, {:.0} pos/s, {:.0} games/s",
            p as f64 / secs,
            g as f64 / secs
        );
        if g >= total_games {
            break;
        }
        if handles.iter().all(|h| h.is_finished()) {
            break;
        }
    }

    let mut parts = Vec::new();
    for h in handles {
        if let Ok(part) = h.join() {
            parts.push(part);
        }
    }

    // Concatenate part files into the final output, then remove the parts.
    concatenate(&cfg.out, &parts);

    let secs = start.elapsed().as_secs_f64().max(0.001);
    let p = positions.load(Ordering::Relaxed);
    let g = games_done.load(Ordering::Relaxed);
    eprintln!(
        "datagen done: {g} games, {p} positions in {secs:.1}s ({:.0} pos/s) -> {}",
        p as f64 / secs,
        cfg.out
    );
}

/// One worker thread: generate `games` games into `part_path`.
fn worker(
    cfg: &Config,
    games: u64,
    seed: u64,
    part_path: &str,
    positions: &AtomicU64,
    games_done: &AtomicU64,
) {
    let file = match File::create(part_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("datagen: cannot create {part_path}: {e}");
            return;
        }
    };
    let mut out = BufWriter::new(file);
    let mut rng = Rng::new(seed | 1);
    let tt = Arc::new(Tt::new(16));
    let stop = Arc::new(AtomicBool::new(false));
    let eval = Eval::hand(); // bootstrap labels with the hand-crafted evaluation
    let mut buffer: Vec<(String, i32)> = Vec::new();

    for _ in 0..games {
        buffer.clear();
        tt.clear();
        let result = play_game(cfg, &mut rng, &tt, &stop, &eval, &mut buffer);
        // Write every recorded position with the game's white-relative result.
        let result_str = match result {
            GameResult::WhiteWin => "1.0",
            GameResult::Draw => "0.5",
            GameResult::BlackWin => "0.0",
        };
        for (fen, score) in &buffer {
            // `writeln!` to a BufWriter only errors on a full disk etc.; bail out.
            if writeln!(out, "{fen} | {score} | {result_str}").is_err() {
                return;
            }
        }
        positions.fetch_add(buffer.len() as u64, Ordering::Relaxed);
        games_done.fetch_add(1, Ordering::Relaxed);
    }
    let _ = out.flush();
}

/// The eventual result of a generated game, from White's perspective.
#[derive(Clone, Copy)]
enum GameResult {
    WhiteWin,
    Draw,
    BlackWin,
}

/// Play one self-play game, pushing recorded `(fen, white_relative_score)` pairs
/// into `buffer`, and return the game result.
fn play_game(
    cfg: &Config,
    rng: &mut Rng,
    tt: &Arc<Tt>,
    stop: &Arc<AtomicBool>,
    eval: &Eval,
    buffer: &mut Vec<(String, i32)>,
) -> GameResult {
    // Random opening. Retry the whole opening if a random line ends the game.
    let board = loop {
        let mut b = Board::startpos();
        let mut ok = true;
        for _ in 0..cfg.random_plies {
            let moves = legal_moves(&b);
            if moves.is_empty() {
                ok = false;
                break;
            }
            let mv = moves.as_slice()[rng.below(moves.len())];
            b.make_move(mv);
        }
        if ok && !legal_moves(&b).is_empty() {
            break b;
        }
    };

    let mut board = board;
    let mut keys: Vec<u64> = vec![board.zobrist_key()];
    let limits = SearchLimits {
        nodes: Some(cfg.nodes),
        ..Default::default()
    };

    let mut ply = 0u32;
    let mut decisive_streak = 0u32; // consecutive plies one side is winning big
    let mut decisive_sign = 0i32;
    let mut draw_streak = 0u32;

    loop {
        // Terminal checks before searching.
        let moves = legal_moves(&board);
        if moves.is_empty() {
            return if board.in_check() {
                // Side to move is checkmated.
                match board.side_to_move() {
                    Color::White => GameResult::BlackWin,
                    Color::Black => GameResult::WhiteWin,
                }
            } else {
                GameResult::Draw // stalemate
            };
        }
        if board.halfmove_clock() >= 100
            || is_threefold(&keys, board.zobrist_key())
            || insufficient_material(&board)
        {
            return GameResult::Draw;
        }
        if ply >= MAX_GAME_PLIES {
            return GameResult::Draw;
        }

        // Labelling search.
        stop.store(false, Ordering::Relaxed);
        let res = search_sync_tt(
            &board,
            &keys,
            limits.clone(),
            eval.clone(),
            Arc::clone(tt),
            Arc::clone(stop),
        );
        let best = res.best_move;
        if best.is_null() {
            return GameResult::Draw;
        }

        // Score from White's point of view.
        let score_white = match board.side_to_move() {
            Color::White => res.score,
            Color::Black => -res.score,
        };

        // Record this position if it is quiet, settled, and non-mate.
        let quiet = !best.is_capture() && !best.is_promotion();
        if ply >= cfg.random_plies
            && !board.in_check()
            && quiet
            && score_white.abs() < SCORE_RECORD_CAP
            && res.score.abs() < MATE_IN_MAX
        {
            buffer.push((board.to_fen(), score_white));
        }

        // Adjudication bookkeeping.
        if score_white.abs() >= RESIGN_CP {
            let sign = score_white.signum();
            if sign == decisive_sign {
                decisive_streak += 1;
            } else {
                decisive_sign = sign;
                decisive_streak = 1;
            }
            if decisive_streak >= ADJUDICATE_PLIES {
                return if sign > 0 {
                    GameResult::WhiteWin
                } else {
                    GameResult::BlackWin
                };
            }
        } else {
            decisive_streak = 0;
            decisive_sign = 0;
        }
        if ply >= DRAW_ADJ_MIN_PLY && score_white.abs() <= DRAW_CP {
            draw_streak += 1;
            if draw_streak >= ADJUDICATE_PLIES {
                return GameResult::Draw;
            }
        } else {
            draw_streak = 0;
        }

        // Play the move.
        board.make_move(best);
        keys.push(board.zobrist_key());
        ply += 1;
    }
}

/// True if `key` already appears at least twice in `keys` (so making the move
/// that produced it is the third occurrence — a threefold repetition).
fn is_threefold(keys: &[u64], key: u64) -> bool {
    keys.iter().filter(|&&k| k == key).count() >= 2
}

/// Minimal insufficient-material check used to end dead games quickly: only
/// kings, or one side has just a single minor piece and the other only its king.
fn insufficient_material(board: &Board) -> bool {
    if board.pieces(PieceType::Pawn).any()
        || board.pieces(PieceType::Rook).any()
        || board.pieces(PieceType::Queen).any()
    {
        return false;
    }
    let minors = board.pieces(PieceType::Knight).count() + board.pieces(PieceType::Bishop).count();
    minors <= 1
}

/// Concatenate `parts` into `out`, then delete the parts.
fn concatenate(out: &str, parts: &[String]) {
    let Ok(dest) = File::create(out) else {
        eprintln!("datagen: cannot create final output {out}");
        return;
    };
    let mut dest = BufWriter::new(dest);
    for part in parts {
        if let Ok(mut f) = File::open(part) {
            let _ = std::io::copy(&mut f, &mut dest);
        }
    }
    let _ = dest.flush();
    for part in parts {
        let _ = std::fs::remove_file(part);
    }
}
