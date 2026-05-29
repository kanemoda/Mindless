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
use crate::see::see_ge;
use crate::tt::{Bound, Tt};
use crate::types::{Color, PieceType};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
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

// Reverse futility pruning (a.k.a. static null-move pruning).
/// Deepest remaining depth at which RFP is attempted.
const RFP_MAX_DEPTH: i32 = 6;
/// Per-ply safety margin (centipawns) the static eval must clear beta by.
const RFP_MARGIN: i32 = 80;

// Null-move pruning.
/// Minimum remaining depth at which a null move is tried.
const NMP_MIN_DEPTH: i32 = 3;
/// Null-move reduction is `NMP_BASE + depth / NMP_DIV`.
const NMP_BASE: i32 = 3;
const NMP_DIV: i32 = 3;

// SEE-based move-loop pruning: at low remaining depth in non-PV nodes, once a
// non-losing best score exists, skip moves that static exchange evaluation says
// lose too much material. Captures use a steeper, quadratic depth margin; quiet
// moves a gentler linear one.
/// Deepest remaining depth at which SEE move pruning is attempted.
const SEE_PRUNE_MAX_DEPTH: i32 = 8;
/// Capture SEE-pruning margin: a capture is pruned when its SEE is below
/// `-SEE_CAPTURE_MARGIN * depth * depth`.
const SEE_CAPTURE_MARGIN: i32 = 20;
/// Quiet SEE-pruning margin: a quiet move is pruned when its SEE is below
/// `-SEE_QUIET_MARGIN * depth`.
const SEE_QUIET_MARGIN: i32 = 65;

// Late move reductions.
/// Minimum remaining depth at which late moves may be reduced.
const LMR_MIN_DEPTH: i32 = 3;
/// Reduce only the moves tried after this many at a node (so moves 1..=3 are
/// searched at full depth).
const LMR_MIN_MOVE_COUNT: i32 = 3;
/// Reduction table parameters: `r = LMR_BASE + ln(depth) * ln(move) / LMR_DIVISOR`.
const LMR_BASE: f64 = 0.75;
const LMR_DIVISOR: f64 = 2.25;

// Move-ordering score tiers. Winning/equal captures (SEE >= 0) rank just below
// the TT move; killers and history-ranked quiets follow; losing captures
// (SEE < 0) are demoted below every quiet so they are tried last. Quiet moves
// are ranked by their combined history (main + continuation), which is bounded
// comfortably inside the killer tier.
const TT_SCORE: i32 = 2_000_000;
const CAPTURE_BASE: i32 = 1_000_000;
const KILLER0_SCORE: i32 = 900_000;
const KILLER1_SCORE: i32 = 800_000;
const BAD_CAPTURE_BASE: i32 = -1_000_000;

/// Upper bound on legal moves in any position; sizes the ordering scratch array.
const MAX_MOVES: usize = 256;

// History heuristics (main butterfly history plus continuation history).
/// Maximum magnitude of any history value; the gravity update keeps values here.
const HIST_CAP: i32 = 16_384;
/// Cap on a single history bonus/penalty (the raw value is `depth * depth`).
const HIST_MAX_BONUS: i32 = 1_200;
/// Combined history is divided by this and clamped to ±2 to nudge the LMR depth.
const HIST_LMR_DIV: i32 = 8_192;
/// Most quiet moves remembered per node for the history penalty ("malus").
const MAX_QUIETS: usize = 64;

// Late move pruning (move-count pruning) and history pruning of quiet moves: at
// low depth in non-PV nodes, once enough moves have been tried the remaining
// quiets are skipped, and quiets with poor history are skipped near the leaves.
/// Deepest remaining depth at which late-move pruning applies.
const LMP_MAX_DEPTH: i32 = 8;
/// Late-move-pruning count: after `LMP_BASE + depth * depth` moves, skip quiets.
const LMP_BASE: i32 = 3;
/// Deepest remaining depth at which quiet history pruning applies.
const HP_MAX_DEPTH: i32 = 4;
/// A quiet move is history-pruned when its combined history is below
/// `-HP_MARGIN * depth`.
const HP_MARGIN: i32 = 2_000;

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

/// Depth reduction for a late, quiet move, read from a precomputed
/// `ln(depth) * ln(move_count)` table: deeper searches and later moves reduce
/// more. The table is built once on first use.
fn lmr_reduction(depth: i32, move_count: i32) -> i32 {
    static TABLE: OnceLock<[[i32; 64]; 64]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [[0i32; 64]; 64];
        // Skip index 0 in both dimensions to avoid ln(0); those entries are
        // never read (reductions only apply at depth >= 3, move >= 4).
        for (d, row) in t.iter_mut().enumerate().skip(1) {
            for (m, slot) in row.iter_mut().enumerate().skip(1) {
                *slot = (LMR_BASE + (d as f64).ln() * (m as f64).ln() / LMR_DIVISOR) as i32;
            }
        }
        t
    });
    let d = depth.clamp(0, 63) as usize;
    let m = move_count.clamp(0, 63) as usize;
    table[d][m]
}

/// A reference to a recently played move for continuation history: the moving
/// piece's index (`0..12`) and its destination square index (`0..64`). `None`
/// stands for "no real move" (the root, or a null move).
type ContRef = Option<(usize, usize)>;

/// Number of continuation plies tracked: the move one ply back and two plies back.
const CONT_PLIES: usize = 2;

/// Move a history value toward a bonus with "gravity": the closer it already is
/// to ±[`HIST_CAP`], the less a further bonus moves it, so values stay bounded
/// in `[-HIST_CAP, HIST_CAP]` without an explicit clamp.
#[inline]
fn apply_gravity(entry: &mut i32, bonus: i32) {
    *entry += bonus - *entry * bonus.abs() / HIST_CAP;
}

/// Continuation-history tables: how good a move (by piece + destination) tends to
/// be when it follows a particular recent move. Indexed by `[offset][prev piece]
/// [prev to][cur piece][cur to]`, where `offset` 0 is the move one ply back and 1
/// the move two plies back. Stored as one flat heap array to keep the [`Searcher`]
/// small and avoid a multi-megabyte stack temporary.
struct ContHist {
    table: Box<[i32]>,
}

impl ContHist {
    fn new() -> ContHist {
        ContHist {
            table: vec![0i32; CONT_PLIES * 12 * 64 * 12 * 64].into_boxed_slice(),
        }
    }

    #[inline]
    fn index(offset: usize, prev: (usize, usize), cur: (usize, usize)) -> usize {
        (((offset * 12 + prev.0) * 64 + prev.1) * 12 + cur.0) * 64 + cur.1
    }

    #[inline]
    fn get(&self, offset: usize, prev: (usize, usize), cur: (usize, usize)) -> i32 {
        self.table[Self::index(offset, prev, cur)]
    }

    #[inline]
    fn update(&mut self, offset: usize, prev: (usize, usize), cur: (usize, usize), bonus: i32) {
        apply_gravity(&mut self.table[Self::index(offset, prev, cur)], bonus);
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
    cont: ContHist,
    /// Per-ply record of the move played to descend from that ply, used to index
    /// continuation history one and two plies deeper.
    conth_stack: [ContRef; MAX_PLY],
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
            cont: ContHist::new(),
            conth_stack: [None; MAX_PLY],
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
            return self.negamax(board, -INF, INF, depth, 0, true);
        }

        let mut delta = ASPIRATION_DELTA;
        let mut alpha = (prev - delta).max(-INF);
        let mut beta = (prev + delta).min(INF);

        loop {
            let score = self.negamax(board, alpha, beta, depth, 0, true);
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
        do_null: bool,
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

        // Static-eval-based pruning (RFP and null-move pruning). Both apply only
        // at non-PV nodes that are not in check and where beta is a normal
        // (non-mate) score, so the static eval is computed once and shared.
        if !is_pv && !in_check && beta.abs() < MATE_IN_MAX {
            let eval = self.eval.evaluate(board);

            // Reverse futility pruning: near the leaves, if the eval beats beta
            // by a depth-scaled margin, assume a fail-high and return at once.
            if depth <= RFP_MAX_DEPTH && eval - RFP_MARGIN * depth >= beta {
                return eval;
            }

            // Null-move pruning: hand the opponent a free move and search at a
            // reduced depth; if the result is still at or above beta the
            // position is so strong that we prune. Guarded by sufficient
            // non-pawn material (zugzwang safeguard) and no consecutive null
            // moves (`do_null`).
            if do_null
                && depth >= NMP_MIN_DEPTH
                && eval >= beta
                && Self::has_non_pawn_material(board)
            {
                let r = NMP_BASE + depth / NMP_DIV;
                let null_depth = (depth - 1 - r).max(0);
                let undo = board.make_null();
                self.conth_stack[ply] = None;
                self.keys.push(board.zobrist_key());
                let score = -self.negamax(board, -beta, -beta + 1, null_depth, ply + 1, false);
                self.keys.pop();
                board.unmake_null(undo);
                if self.stopped {
                    return 0;
                }
                if score >= beta {
                    // Never propagate an unproven mate score from a null search.
                    return if score >= MATE_IN_MAX { beta } else { score };
                }
            }
        }

        // Continuation-history context: the moves played one and two plies back.
        let cont0 = if ply >= 1 {
            self.conth_stack[ply - 1]
        } else {
            None
        };
        let cont1 = if ply >= 2 {
            self.conth_stack[ply - 2]
        } else {
            None
        };

        let mut moves = MoveList::new();
        generate_legal(board, &mut moves);
        if moves.is_empty() {
            return if in_check { -MATE + ply as i32 } else { DRAW };
        }
        self.order_moves(board, &mut moves, tt_move, ply, cont0, cont1, true);

        let original_alpha = alpha;
        let mut best = -INF;
        let mut best_move = Move::NULL;
        let mut move_count = 0;
        let mut quiets_tried = [Move::NULL; MAX_QUIETS];
        let mut n_quiets = 0usize;

        for &mv in moves.as_slice() {
            let is_quiet = !mv.is_capture() && !mv.is_promotion();

            // The moving piece (read before the move, while it is still on
            // `from`) keys this move in the continuation-history tables; the
            // combined history is computed once and reused for pruning and LMR.
            let piece_idx = board.piece_on(mv.from()).map_or(0, |p| p.index());
            let hist = if is_quiet {
                self.quiet_history(mv.from().index(), mv.to().index(), piece_idx, cont0, cont1)
            } else {
                0
            };

            // Move-loop pruning, at non-PV nodes that are not in check and once a
            // non-losing best score exists — so the first (best-ordered) move is
            // always searched and we never prune while being mated. Pruned moves
            // are not made, saving the whole subtree.
            if !is_pv && !in_check && best > -MATE_IN_MAX {
                // Late move pruning: after a depth-dependent move count, skip the
                // remaining quiet moves entirely.
                if is_quiet && depth <= LMP_MAX_DEPTH && move_count >= LMP_BASE + depth * depth {
                    continue;
                }
                // History pruning: near the leaves, skip quiet moves whose
                // combined history is poor.
                if is_quiet && depth <= HP_MAX_DEPTH && hist < -HP_MARGIN * depth {
                    continue;
                }
                // SEE pruning: skip moves that lose too much material. Captures
                // use a steeper quadratic margin, quiets a linear one.
                if depth <= SEE_PRUNE_MAX_DEPTH {
                    let threshold = if is_quiet {
                        -SEE_QUIET_MARGIN * depth
                    } else {
                        -SEE_CAPTURE_MARGIN * depth * depth
                    };
                    if !see_ge(board, mv, threshold) {
                        continue;
                    }
                }
            }

            if is_quiet && n_quiets < quiets_tried.len() {
                quiets_tried[n_quiets] = mv;
                n_quiets += 1;
            }

            let undo = board.make_move(mv);
            self.conth_stack[ply] = Some((piece_idx, mv.to().index()));
            self.keys.push(board.zobrist_key());
            move_count += 1;

            // Principal-variation search with late-move reductions. The first
            // move gets a full-window search; the rest get a null-window probe.
            // Late, quiet, non-checking, non-killer moves are probed at a reduced
            // depth and only re-searched at full depth if that probe beats alpha.
            let score = if move_count == 1 {
                -self.negamax(board, -beta, -alpha, depth - 1, ply + 1, true)
            } else {
                let mut reduction = 0;
                if is_quiet
                    && !in_check
                    && depth >= LMR_MIN_DEPTH
                    && move_count > LMR_MIN_MOVE_COUNT
                    && mv != self.killers[ply][0]
                    && mv != self.killers[ply][1]
                    && !board.in_check()
                {
                    reduction = lmr_reduction(depth, move_count);
                    if is_pv {
                        reduction -= 1;
                    }
                    // Reduce less for moves with good combined history, more for
                    // moves with bad history (reusing the value computed above).
                    reduction -= (hist / HIST_LMR_DIV).clamp(-2, 2);
                    reduction = reduction.clamp(0, depth - 2);
                }

                // Reduced null-window probe (reduction 0 ⇒ ordinary PVS probe).
                let mut s = -self.negamax(
                    board,
                    -alpha - 1,
                    -alpha,
                    depth - 1 - reduction,
                    ply + 1,
                    true,
                );
                // A reduced probe that beats alpha is re-tried at full depth.
                if reduction > 0 && s > alpha {
                    s = -self.negamax(board, -alpha - 1, -alpha, depth - 1, ply + 1, true);
                }
                // A move that lands inside the window is a new PV: re-search wide.
                if s > alpha && s < beta {
                    s = -self.negamax(board, -beta, -alpha, depth - 1, ply + 1, true);
                }
                s
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
                        // Beta cutoff. Reward the cutoff move (if quiet) and
                        // penalise the earlier quiets that failed to cut, across
                        // both the main and continuation history tables.
                        if is_quiet {
                            self.store_killer(ply, mv);
                        }
                        self.update_histories(
                            board,
                            mv,
                            is_quiet,
                            depth,
                            cont0,
                            cont1,
                            &quiets_tried[..n_quiets],
                        );
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
            self.order_moves(board, &mut moves, Move::NULL, ply, None, None, false);
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
            self.order_moves(board, &mut moves, Move::NULL, ply, None, None, false);
        }

        for &mv in moves.as_slice() {
            if !in_check {
                // Delta pruning: skip captures that cannot plausibly reach alpha.
                let gain = Self::move_gain(board, mv);
                if stand + gain + DELTA_MARGIN < alpha {
                    continue;
                }
                // SEE pruning: skip plain captures that lose material outright.
                // Promotions are always tried (rare and forcing).
                if mv.is_capture() && !mv.is_promotion() && !see_ge(board, mv, 0) {
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

    /// Order moves in place, best first: TT move, then winning/equal captures
    /// (MVV-LVA), then killers, then quiets by history, then losing captures.
    ///
    /// Each move is scored exactly once into a scratch array before sorting, so
    /// the (relatively expensive) SEE split is computed once per move rather than
    /// once per comparison. When `use_see` is false the capture split is skipped
    /// and every capture is ranked by plain MVV-LVA — used by quiescence, which
    /// prunes losing captures directly and so never needs them demoted.
    #[allow(clippy::too_many_arguments)]
    fn order_moves(
        &self,
        board: &Board,
        moves: &mut MoveList,
        tt_move: Move,
        ply: usize,
        cont0: ContRef,
        cont1: ContRef,
        use_see: bool,
    ) {
        let killers = self.killers[ply];
        let n = moves.len();
        let mut scored = [(0i32, Move::NULL); MAX_MOVES];
        for (slot, &mv) in scored[..n].iter_mut().zip(moves.as_slice()) {
            *slot = (
                self.score_move(board, mv, tt_move, killers, cont0, cont1, use_see),
                mv,
            );
        }
        scored[..n].sort_unstable_by_key(|&(score, _)| std::cmp::Reverse(score));
        for (dst, src) in moves.as_mut_slice().iter_mut().zip(&scored[..n]) {
            *dst = src.1;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn score_move(
        &self,
        board: &Board,
        mv: Move,
        tt_move: Move,
        killers: [Move; 2],
        cont0: ContRef,
        cont1: ContRef,
        use_see: bool,
    ) -> i32 {
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
            let mvv_lva = victim * 16 - attacker + promo;
            // Demote captures that lose material (negative SEE) below all quiets.
            let base = if use_see && !see_ge(board, mv, 0) {
                BAD_CAPTURE_BASE
            } else {
                CAPTURE_BASE
            };
            return base + mvv_lva;
        }
        if mv == killers[0] {
            return KILLER0_SCORE;
        }
        if mv == killers[1] {
            return KILLER1_SCORE;
        }
        let pc = board.piece_on(mv.from()).map_or(0, |p| p.index());
        self.quiet_history(mv.from().index(), mv.to().index(), pc, cont0, cont1)
    }

    /// Combined quiet-move history: the butterfly main history plus the
    /// continuation history for one and two plies back. `pc` is the moving
    /// piece's index, passed explicitly because the board may already be in the
    /// child position (after the move was made) when this is called from LMR.
    #[inline]
    fn quiet_history(
        &self,
        from: usize,
        to: usize,
        pc: usize,
        cont0: ContRef,
        cont1: ContRef,
    ) -> i32 {
        let mut h = self.history[from][to];
        let cur = (pc, to);
        if let Some(prev) = cont0 {
            h += self.cont.get(0, prev, cur);
        }
        if let Some(prev) = cont1 {
            h += self.cont.get(1, prev, cur);
        }
        h
    }

    fn store_killer(&mut self, ply: usize, mv: Move) {
        if self.killers[ply][0] != mv {
            self.killers[ply][1] = self.killers[ply][0];
            self.killers[ply][0] = mv;
        }
    }

    /// Update history after a beta cutoff: reward `cutoff` (when it is a quiet
    /// move) and penalise the earlier quiet moves that were tried but did not
    /// cut, in both the main and continuation tables.
    #[allow(clippy::too_many_arguments)]
    fn update_histories(
        &mut self,
        board: &Board,
        cutoff: Move,
        cutoff_is_quiet: bool,
        depth: i32,
        cont0: ContRef,
        cont1: ContRef,
        tried: &[Move],
    ) {
        let bonus = (depth * depth).min(HIST_MAX_BONUS);
        if cutoff_is_quiet {
            self.bump_history(board, cutoff, cont0, cont1, bonus);
        }
        for &q in tried {
            if q != cutoff {
                self.bump_history(board, q, cont0, cont1, -bonus);
            }
        }
    }

    /// Apply one history delta to a quiet move across the main and continuation
    /// tables. The board is at the node's own position, so `mv.from()` still
    /// holds the moving piece.
    fn bump_history(
        &mut self,
        board: &Board,
        mv: Move,
        cont0: ContRef,
        cont1: ContRef,
        delta: i32,
    ) {
        let from = mv.from().index();
        let to = mv.to().index();
        apply_gravity(&mut self.history[from][to], delta);
        let pc = board.piece_on(mv.from()).map_or(0, |p| p.index());
        let cur = (pc, to);
        if let Some(prev) = cont0 {
            self.cont.update(0, prev, cur, delta);
        }
        if let Some(prev) = cont1 {
            self.cont.update(1, prev, cur, delta);
        }
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

    /// True if the side to move has at least one knight, bishop, rook, or queen.
    /// Used as a zugzwang safeguard before null-move pruning.
    fn has_non_pawn_material(board: &Board) -> bool {
        let us = board.side_to_move();
        (board.pieces_colored(us, PieceType::Knight)
            | board.pieces_colored(us, PieceType::Bishop)
            | board.pieces_colored(us, PieceType::Rook)
            | board.pieces_colored(us, PieceType::Queen))
        .any()
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
