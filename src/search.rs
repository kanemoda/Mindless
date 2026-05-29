//! Search: iterative-deepening negamax with alpha-beta, quiescence, a
//! transposition table, and time management.
//!
//! The public entry point is [`think`], which runs a search on a position under
//! the given [`SearchLimits`], emits UCI `info` lines as it deepens, and returns
//! the best move. It is designed to run on its own thread; the `stop` flag lets
//! the UCI layer interrupt it.
//!
//! This is the Milestone 2 baseline: alpha-beta with principal-variation search
//! (an exact alpha-beta refinement), correct mate-distance scoring, quiescence
//! with delta pruning, and well-ordered moves. Aggressive heuristics
//! (null-move, late-move reductions, futility, ...) are intentionally deferred.

use crate::board::Board;
use crate::eval::{Evaluator, HandCrafted, PIECE_VALUE};
use crate::movegen::{generate_legal, generate_noisy, legal_moves};
use crate::moves::{Move, MoveList};
use crate::tt::{Bound, Tt};
use crate::types::{Color, PieceType};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Maximum search ply (bounds recursion and the PV / killer tables).
pub const MAX_PLY: usize = 128;
/// A value larger than any real evaluation, used as the alpha/beta infinity.
pub const INF: i32 = 32_001;
/// Score of an immediate checkmate (delivered at the current node).
pub const MATE: i32 = 32_000;
/// Scores with magnitude at or above this are "mate in N" scores.
pub const MATE_IN_MAX: i32 = MATE - MAX_PLY as i32;

const DRAW: i32 = 0;
/// Safety margin subtracted from clock time to avoid flagging (ms).
const MOVE_OVERHEAD: u64 = 15;
/// Quiescence delta-pruning margin (centipawns).
const DELTA_MARGIN: i32 = 100;

// Aspiration windows: from this depth on, the root searches inside a narrow
// window centred on the previous iteration's score and only widens when the
// true score falls outside it, instead of always using a full (-INF, INF)
// window. Most iterations land inside the window, making them much cheaper.
/// Iterative-deepening depth at which aspiration windows switch on.
const ASPIRATION_MIN_DEPTH: i32 = 4;
/// Initial half-width of the aspiration window (centipawns).
const ASPIRATION_DELTA: i32 = 16;
/// Once a window has widened past this half-width, reopen it fully rather than
/// keep re-searching (handles big score swings and mate finds cheaply).
const ASPIRATION_MAX_DELTA: i32 = 600;

// Move-ordering score tiers.
const TT_SCORE: i32 = 2_000_000;
const CAPTURE_BASE: i32 = 1_000_000;
const KILLER0_SCORE: i32 = 900_000;
const KILLER1_SCORE: i32 = 800_000;
const HISTORY_CAP: i32 = 700_000;

/// The limits requested by a `go` command.
#[derive(Clone, Default)]
pub struct SearchLimits {
    /// Fixed search depth.
    pub depth: Option<u32>,
    /// Node budget.
    pub nodes: Option<u64>,
    /// Fixed time per move (ms).
    pub movetime: Option<u64>,
    /// White's remaining clock (ms).
    pub wtime: Option<u64>,
    /// Black's remaining clock (ms).
    pub btime: Option<u64>,
    /// White's increment (ms).
    pub winc: Option<u64>,
    /// Black's increment (ms).
    pub binc: Option<u64>,
    /// Moves until the next time control.
    pub movestogo: Option<u32>,
    /// Search until explicitly stopped.
    pub infinite: bool,
}

/// Convert a search-relative mate score to a node-relative score for TT storage.
#[inline]
fn score_to_tt(score: i32, ply: usize) -> i32 {
    if score >= MATE_IN_MAX {
        score + ply as i32
    } else if score <= -MATE_IN_MAX {
        score - ply as i32
    } else {
        score
    }
}

/// Inverse of [`score_to_tt`]: node-relative score back to search-relative.
#[inline]
fn score_from_tt(score: i32, ply: usize) -> i32 {
    if score >= MATE_IN_MAX {
        score - ply as i32
    } else if score <= -MATE_IN_MAX {
        score + ply as i32
    } else {
        score
    }
}

/// Per-search state for one search thread.
struct Searcher<E: Evaluator> {
    eval: E,
    tt: Arc<Tt>,
    stop: Arc<AtomicBool>,
    limits: SearchLimits,

    start: Instant,
    soft_ms: u64,
    hard_ms: u64,

    nodes: u64,
    seldepth: usize,
    stopped: bool,

    killers: [[Move; 2]; MAX_PLY],
    history: [[i32; 64]; 64],
    pv: [[Move; MAX_PLY]; MAX_PLY],
    pv_len: [usize; MAX_PLY],

    // Position keys for repetition detection: the game history followed by the
    // current search path.
    keys: Vec<u64>,
    best_move: Move,
    last_score: i32,
    last_depth: u32,
}

/// The outcome of a completed (or interrupted) search.
pub struct SearchResult {
    /// Best move found.
    pub best_move: Move,
    /// Score of `best_move` from the side-to-move's perspective.
    pub score: i32,
    /// Deepest fully-completed iterative-deepening depth.
    pub depth: u32,
    /// Total nodes searched.
    pub nodes: u64,
}

/// Run a search synchronously (no thread, no `info` output) and return the
/// result. Convenient for tests and embedding.
pub fn search_sync(board: &Board, history: &[u64], limits: SearchLimits) -> SearchResult {
    let tt = Arc::new(Tt::new(16));
    let stop = Arc::new(AtomicBool::new(false));
    let mut searcher = Box::new(Searcher::new(
        HandCrafted::new(),
        tt,
        stop,
        limits,
        history.to_vec(),
    ));
    let mut board = board.clone();
    let best = searcher.run_quiet(&mut board);
    SearchResult {
        best_move: best,
        score: searcher.last_score,
        depth: searcher.last_depth,
        nodes: searcher.nodes,
    }
}

/// Run a search and return the best move, printing `info` lines as it deepens.
///
/// Intended to be called on a dedicated thread. `history` holds the Zobrist keys
/// of the game so far (ending with the root position) for repetition detection.
pub fn think(
    board: Board,
    history: Vec<u64>,
    tt: Arc<Tt>,
    stop: Arc<AtomicBool>,
    limits: SearchLimits,
) -> Move {
    let mut searcher = Box::new(Searcher::new(HandCrafted::new(), tt, stop, limits, history));
    let mut board = board;
    searcher.run(&mut board)
}

impl<E: Evaluator> Searcher<E> {
    fn new(
        eval: E,
        tt: Arc<Tt>,
        stop: Arc<AtomicBool>,
        limits: SearchLimits,
        keys: Vec<u64>,
    ) -> Searcher<E> {
        Searcher {
            eval,
            tt,
            stop,
            limits,
            start: Instant::now(),
            soft_ms: u64::MAX,
            hard_ms: u64::MAX,
            nodes: 0,
            seldepth: 0,
            stopped: false,
            killers: [[Move::NULL; 2]; MAX_PLY],
            history: [[0; 64]; 64],
            pv: [[Move::NULL; MAX_PLY]; MAX_PLY],
            pv_len: [0; MAX_PLY],
            keys,
            best_move: Move::NULL,
            last_score: 0,
            last_depth: 0,
        }
    }

    /// Iterative deepening driver (prints `info` lines).
    fn run(&mut self, board: &mut Board) -> Move {
        self.iterate(board, true)
    }

    /// Iterative deepening without `info` output (for embedding / tests).
    fn run_quiet(&mut self, board: &mut Board) -> Move {
        self.iterate(board, false)
    }

    fn iterate(&mut self, board: &mut Board, print: bool) -> Move {
        self.tt.new_search();
        self.setup_time(board.side_to_move());
        self.nodes = 0;
        self.stopped = false;

        // Guarantee a legal reply even if interrupted immediately.
        let root_moves = legal_moves(board);
        if root_moves.is_empty() {
            return Move::NULL;
        }
        self.best_move = root_moves[0];

        let max_depth = self
            .limits
            .depth
            .unwrap_or(MAX_PLY as u32 - 1)
            .min(MAX_PLY as u32 - 1);

        let mut score = 0;
        for depth in 1..=max_depth {
            self.seldepth = 0;
            score = self.search_root(board, depth as i32, score);

            if self.stopped {
                // Discard the interrupted iteration: keep the previous depth's
                // committed best move (so `bestmove` always matches the last
                // `info` line we printed).
                break;
            }

            // Commit the completed iteration's principal variation.
            self.best_move = self.pv[0][0];
            self.last_score = score;
            self.last_depth = depth;
            if print {
                self.print_info(depth, score);
            }

            // Stop between iterations on soft time / node budget / found mate.
            if self.elapsed_ms() >= self.soft_ms {
                break;
            }
            if let Some(n) = self.limits.nodes {
                if self.nodes >= n {
                    break;
                }
            }
            if score.abs() >= MATE_IN_MAX {
                break;
            }
        }

        self.best_move
    }

    /// Search the root to `depth`, using an aspiration window centred on the
    /// previous iteration's score `prev`. On a fail-high or fail-low the window
    /// is widened and the search retried; low depths (where the score is still
    /// noisy) use a full window. Returns the final, in-window score.
    fn search_root(&mut self, board: &mut Board, depth: i32, prev: i32) -> i32 {
        if depth < ASPIRATION_MIN_DEPTH {
            return self.negamax(board, -INF, INF, depth, 0);
        }

        let mut delta = ASPIRATION_DELTA;
        let mut alpha = (prev - delta).max(-INF);
        let mut beta = (prev + delta).min(INF);

        loop {
            let score = self.negamax(board, alpha, beta, depth, 0);
            if self.stopped {
                return score;
            }

            if score <= alpha {
                // Fail low: relax alpha downward and pull beta toward the centre
                // so the re-search re-establishes the upper bound quickly.
                beta = (alpha + beta) / 2;
                alpha = (score - delta).max(-INF);
            } else if score >= beta {
                // Fail high: relax beta upward.
                beta = (score + delta).min(INF);
            } else {
                return score; // score is inside the window: accept it.
            }

            if delta >= ASPIRATION_MAX_DELTA {
                alpha = -INF;
                beta = INF;
            } else {
                delta += delta / 2; // widen ~1.5x per retry
            }
        }
    }

    fn negamax(
        &mut self,
        board: &mut Board,
        mut alpha: i32,
        mut beta: i32,
        mut depth: i32,
        ply: usize,
    ) -> i32 {
        if self.stopped {
            return 0;
        }
        self.nodes += 1;
        if self.nodes & 2047 == 0 {
            self.check_time();
        }
        if self.stopped {
            return 0;
        }

        self.pv_len[ply] = 0;

        if ply + 1 >= MAX_PLY {
            return self.quiescence(board, alpha, beta, ply);
        }

        let is_root = ply == 0;
        let is_pv = beta - alpha > 1;
        let in_check = board.in_check();

        // Check extension: search one ply deeper when in check.
        if in_check {
            depth += 1;
        }

        // Leaf → quiescence (only reachable when not in check, due to the
        // extension above).
        if depth <= 0 {
            return self.quiescence(board, alpha, beta, ply);
        }

        if !is_root {
            // Draw detection.
            if self.is_repetition(board) || Self::insufficient_material(board) {
                return DRAW;
            }
            if board.halfmove_clock() >= 100 {
                // Fifty-move draw, unless this position is checkmate.
                if !in_check {
                    return DRAW;
                }
                let mut ml = MoveList::new();
                generate_legal(board, &mut ml);
                return if ml.is_empty() {
                    -MATE + ply as i32
                } else {
                    DRAW
                };
            }

            // Mate-distance pruning: a faster mate elsewhere bounds this node.
            alpha = alpha.max(-MATE + ply as i32);
            beta = beta.min(MATE - ply as i32 - 1);
            if alpha >= beta {
                return alpha;
            }
        }

        self.seldepth = self.seldepth.max(ply);

        // Transposition table probe.
        let mut tt_move = Move::NULL;
        if let Some(entry) = self.tt.probe(board.zobrist_key()) {
            tt_move = entry.mv;
            if !is_pv && entry.depth >= depth {
                let s = score_from_tt(entry.score, ply);
                match entry.bound {
                    Bound::Exact => return s,
                    Bound::Lower if s >= beta => return s,
                    Bound::Upper if s <= alpha => return s,
                    _ => {}
                }
            }
        }

        let mut moves = MoveList::new();
        generate_legal(board, &mut moves);
        if moves.is_empty() {
            return if in_check { -MATE + ply as i32 } else { DRAW };
        }
        self.order_moves(board, &mut moves, tt_move, ply);

        let original_alpha = alpha;
        let mut best = -INF;
        let mut best_move = Move::NULL;
        let mut move_count = 0;

        for &mv in moves.as_slice() {
            let undo = board.make_move(mv);
            self.keys.push(board.zobrist_key());
            move_count += 1;

            // Principal-variation search: full window for the first move, a
            // null window probe for the rest (re-searched if it beats alpha).
            let score = if move_count == 1 {
                -self.negamax(board, -beta, -alpha, depth - 1, ply + 1)
            } else {
                let probe = -self.negamax(board, -alpha - 1, -alpha, depth - 1, ply + 1);
                if probe > alpha && probe < beta {
                    -self.negamax(board, -beta, -alpha, depth - 1, ply + 1)
                } else {
                    probe
                }
            };

            self.keys.pop();
            board.unmake_move(mv, undo);

            if self.stopped {
                return 0;
            }

            if score > best {
                best = score;
                best_move = mv;
                if score > alpha {
                    alpha = score;
                    self.update_pv(ply, mv);
                    if alpha >= beta {
                        // Beta cutoff: reward this quiet move for future ordering.
                        if !mv.is_capture() && !mv.is_promotion() {
                            self.store_killer(ply, mv);
                            self.update_history(mv, depth);
                        }
                        break;
                    }
                }
            }
        }

        let bound = if best >= beta {
            Bound::Lower
        } else if best > original_alpha {
            Bound::Exact
        } else {
            Bound::Upper
        };
        self.tt.store(
            board.zobrist_key(),
            best_move,
            score_to_tt(best, ply),
            depth,
            bound,
        );

        best
    }

    fn quiescence(&mut self, board: &mut Board, mut alpha: i32, beta: i32, ply: usize) -> i32 {
        if self.stopped {
            return 0;
        }
        self.nodes += 1;
        if self.nodes & 2047 == 0 {
            self.check_time();
        }
        if self.stopped {
            return 0;
        }
        if ply >= MAX_PLY {
            return self.eval.evaluate(board);
        }
        self.seldepth = self.seldepth.max(ply);

        // Draw detection bounds perpetual-check lines and scores repetitions.
        if self.is_repetition(board)
            || Self::insufficient_material(board)
            || board.halfmove_clock() >= 100
        {
            return DRAW;
        }

        let in_check = board.in_check();
        let mut moves = MoveList::new();
        let mut best;
        let stand;

        if in_check {
            // No stand-pat while in check: every legal evasion is considered.
            stand = -INF;
            best = -INF;
            generate_legal(board, &mut moves);
            if moves.is_empty() {
                return -MATE + ply as i32; // checkmate
            }
            self.order_moves(board, &mut moves, Move::NULL, ply);
        } else {
            stand = self.eval.evaluate(board);
            if stand >= beta {
                return stand;
            }
            if stand > alpha {
                alpha = stand;
            }
            best = stand;
            generate_noisy(board, &mut moves);
            self.order_moves(board, &mut moves, Move::NULL, ply);
        }

        for &mv in moves.as_slice() {
            // Delta pruning: skip captures that cannot plausibly reach alpha.
            if !in_check {
                let gain = Self::move_gain(board, mv);
                if stand + gain + DELTA_MARGIN < alpha {
                    continue;
                }
            }

            let undo = board.make_move(mv);
            self.keys.push(board.zobrist_key());
            let score = -self.quiescence(board, -beta, -alpha, ply + 1);
            self.keys.pop();
            board.unmake_move(mv, undo);

            if self.stopped {
                return 0;
            }
            if score > best {
                best = score;
                if score > alpha {
                    alpha = score;
                }
                if alpha >= beta {
                    break;
                }
            }
        }

        best
    }

    /// Optimistic material swing of a capturing/promoting move, for delta pruning.
    fn move_gain(board: &Board, mv: Move) -> i32 {
        let mut gain = if mv.is_en_passant() {
            PIECE_VALUE[PieceType::Pawn.index()]
        } else {
            board
                .piece_on(mv.to())
                .map_or(0, |p| PIECE_VALUE[p.piece_type().index()])
        };
        if mv.is_promotion() {
            gain += PIECE_VALUE[PieceType::Queen.index()] - PIECE_VALUE[PieceType::Pawn.index()];
        }
        gain
    }

    /// Order moves in place: TT move, then captures (MVV-LVA), then killers,
    /// then quiets by history.
    fn order_moves(&self, board: &Board, moves: &mut MoveList, tt_move: Move, ply: usize) {
        let killers = self.killers[ply];
        moves.as_mut_slice().sort_unstable_by(|&a, &b| {
            self.score_move(board, b, tt_move, killers)
                .cmp(&self.score_move(board, a, tt_move, killers))
        });
    }

    fn score_move(&self, board: &Board, mv: Move, tt_move: Move, killers: [Move; 2]) -> i32 {
        if mv == tt_move {
            return TT_SCORE;
        }
        if mv.is_capture() || mv.is_promotion() {
            let victim = if mv.is_en_passant() {
                PIECE_VALUE[PieceType::Pawn.index()]
            } else {
                board
                    .piece_on(mv.to())
                    .map_or(0, |p| PIECE_VALUE[p.piece_type().index()])
            };
            let attacker = board
                .piece_on(mv.from())
                .map_or(0, |p| p.piece_type().index() as i32);
            let promo = mv.promotion().map_or(0, |pt| PIECE_VALUE[pt.index()]);
            // Most-valuable-victim, least-valuable-attacker, plus promotion gain.
            return CAPTURE_BASE + victim * 16 - attacker + promo;
        }
        if mv == killers[0] {
            return KILLER0_SCORE;
        }
        if mv == killers[1] {
            return KILLER1_SCORE;
        }
        self.history[mv.from().index()][mv.to().index()].min(HISTORY_CAP)
    }

    fn store_killer(&mut self, ply: usize, mv: Move) {
        if self.killers[ply][0] != mv {
            self.killers[ply][1] = self.killers[ply][0];
            self.killers[ply][0] = mv;
        }
    }

    fn update_history(&mut self, mv: Move, depth: i32) {
        let entry = &mut self.history[mv.from().index()][mv.to().index()];
        *entry = (*entry + depth * depth).min(1 << 20);
    }

    fn update_pv(&mut self, ply: usize, mv: Move) {
        self.pv[ply][0] = mv;
        let child_len = self.pv_len[ply + 1];
        let mut i = 0;
        while i < child_len && i + 1 < MAX_PLY {
            self.pv[ply][i + 1] = self.pv[ply + 1][i];
            i += 1;
        }
        self.pv_len[ply] = i + 1;
    }

    /// Two-fold repetition within the halfmove window counts as a draw in search.
    fn is_repetition(&self, board: &Board) -> bool {
        let key = board.zobrist_key();
        let n = self.keys.len();
        let limit = board.halfmove_clock() as usize;
        let mut back = 4;
        while back <= limit && back < n {
            if self.keys[n - 1 - back] == key {
                return true;
            }
            back += 2;
        }
        false
    }

    /// Detect material that cannot force mate: K vs K, K+minor vs K, and
    /// same-colored K+B vs K+B.
    fn insufficient_material(board: &Board) -> bool {
        if board.pieces(PieceType::Pawn).any()
            || board.pieces(PieceType::Rook).any()
            || board.pieces(PieceType::Queen).any()
        {
            return false;
        }
        let wn = board
            .pieces_colored(Color::White, PieceType::Knight)
            .count();
        let wb = board
            .pieces_colored(Color::White, PieceType::Bishop)
            .count();
        let bn = board
            .pieces_colored(Color::Black, PieceType::Knight)
            .count();
        let bb = board
            .pieces_colored(Color::Black, PieceType::Bishop)
            .count();
        let white = wn + wb;
        let black = bn + bb;

        if white + black <= 1 {
            return true; // K vs K, or K + single minor vs K
        }
        if white == 1 && black == 1 && wb == 1 && bb == 1 {
            // King and bishop each: drawn if the bishops are the same color.
            let w = board.pieces_colored(Color::White, PieceType::Bishop).lsb();
            let b = board.pieces_colored(Color::Black, PieceType::Bishop).lsb();
            return (w.file() + w.rank()) % 2 == (b.file() + b.rank()) % 2;
        }
        false
    }

    // --- time management ---

    fn setup_time(&mut self, stm: Color) {
        self.start = Instant::now();

        if self.limits.infinite {
            self.soft_ms = u64::MAX;
            self.hard_ms = u64::MAX;
            return;
        }
        if let Some(mt) = self.limits.movetime {
            let m = mt.saturating_sub(MOVE_OVERHEAD).max(1);
            self.soft_ms = m;
            self.hard_ms = m;
            return;
        }

        let clock = if stm == Color::White {
            self.limits.wtime
        } else {
            self.limits.btime
        };
        let Some(time_left) = clock else {
            // Depth/nodes-limited or bare "go": no wall-clock limit.
            self.soft_ms = u64::MAX;
            self.hard_ms = u64::MAX;
            return;
        };

        let inc = if stm == Color::White {
            self.limits.winc
        } else {
            self.limits.binc
        }
        .unwrap_or(0);
        let mtg = self.limits.movestogo.map_or(40, |m| m.clamp(1, 40)) as u64;

        let t = time_left.saturating_sub(MOVE_OVERHEAD).max(1);
        let optimum = t / mtg + inc * 3 / 4;
        let cap = t * 4 / 5; // never spend more than 80% of the remaining clock
        self.soft_ms = optimum.min(cap).max(1);
        self.hard_ms = (optimum * 4).min(cap).max(self.soft_ms);
    }

    #[inline]
    fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// Set the stop flag if the external stop, node budget, or hard time limit
    /// has been reached.
    fn check_time(&mut self) {
        if self.stop.load(Ordering::Relaxed) {
            self.stopped = true;
            return;
        }
        if let Some(n) = self.limits.nodes {
            if self.nodes >= n {
                self.stopped = true;
                return;
            }
        }
        if self.hard_ms != u64::MAX && self.elapsed_ms() >= self.hard_ms {
            self.stopped = true;
        }
    }

    fn print_info(&self, depth: u32, score: i32) {
        let elapsed = self.elapsed_ms();
        let nps = self.nodes * 1000 / elapsed.max(1);

        let score_str = if score >= MATE_IN_MAX {
            format!("mate {}", (MATE - score + 1) / 2)
        } else if score <= -MATE_IN_MAX {
            format!("mate {}", -((MATE + score + 1) / 2))
        } else {
            format!("cp {score}")
        };

        let mut pv = String::new();
        for i in 0..self.pv_len[0] {
            if i > 0 {
                pv.push(' ');
            }
            pv.push_str(&self.pv[0][i].to_uci());
        }

        println!(
            "info depth {} seldepth {} score {} nodes {} nps {} hashfull {} time {} pv {}",
            depth,
            self.seldepth,
            score_str,
            self.nodes,
            nps,
            self.tt.hashfull(),
            elapsed,
            pv
        );
    }
}
