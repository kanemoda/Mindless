# Mindless

A UCI chess engine written in Rust, built for both strength and clean reuse.

Mindless is a single Rust crate that is **both** a runnable chess engine (the
`mindless` binary speaks the Universal Chess Interface, so any standard chess GUI
can run it) **and** a library (other Rust projects can depend on it to represent
positions, generate moves, and search).

> **Status — Milestone 6 (First NNUE).** Mindless now evaluates positions with a
> trained **NNUE neural network**, embedded in the binary and on by default. It
> measures **+46 Elo** over the Milestone-5 hand-crafted evaluation (SPRT) and
> **+405 Elo** over the first playing baseline. From the start position it searches
> about **17 plies deep in five seconds** — the network costs some depth versus the
> old evaluation but judges positions far better. See [DEVLOG.md](DEVLOG.md) and
> [ARCHITECTURE.md](ARCHITECTURE.md) for plain-language explanations.

## Features

**Foundation (Milestone 1)**

- Bitboard board representation with full game state.
- Magic bitboards for sliding pieces; fully-legal move generation (castling, en
  passant with discovered-check handling, promotions, pins, check evasions).
- Incremental Zobrist hashing, fast make/unmake, FEN parse/serialize.
- Perft harness verified exactly against the standard reference positions.

**Search & evaluation (Milestone 2)**

- **Tapered hand-crafted evaluation** (PeSTO material + piece-square tables),
  behind an `Evaluator` trait so it can be swapped for NNUE later.
- **Negamax + alpha-beta** with **principal-variation search** and **iterative
  deepening**, with full principal-variation reporting.
- **Quiescence search** (captures + promotions, stand-pat, delta pruning,
  MVV-LVA ordering) to avoid horizon-effect blunders.
- **Transposition table** (lock-free, configurable size, depth-preferred
  replacement with aging) with ply-correct mate-score handling.
- **Move ordering**: TT move → MVV-LVA captures → killers → history heuristic.
- **Correct draws**: threefold repetition, fifty-move rule, insufficient
  material.
- **Time management** honoring all standard `go` parameters, with soft/hard
  limits that never overstep the clock.
- **UCI `info`** output: depth, seldepth, score (cp/mate), nodes, nps, hashfull,
  time, and pv.
- **Zero runtime dependencies** — pure safe Rust.

**Search refinements (Milestones 4–5)**

- **Aspiration windows**, **reverse-futility pruning**, **null-move pruning**,
  and **late move reductions** (Milestone 4).
- **Static exchange evaluation (SEE)**: winning/equal captures are ordered first
  and losing captures are pruned in quiescence and (depth-scaled) in the main
  search.
- **Continuation history** (one and two plies back) alongside the main history,
  with a penalty for quiet moves that fail to cut, feeding both move ordering and
  late-move reductions.
- **Late move pruning** and **history pruning** of unpromising quiet moves near
  the leaves.
- Every strength change is kept only after passing a self-play SPRT (a tested but
  unhelpful idea — singular extensions — was measured and deliberately left out).

**Neural-network evaluation (Milestone 6)**

- **NNUE evaluation** — a `(768 -> 128)x2 -> 1` perspective network with
  squared-clipped-ReLU activation and integer quantization, trained on ~42M
  self-play positions with [bullet](https://github.com/jw1912/bullet). The 193 KB
  net is **embedded in the binary and used by default**; the engine's integer
  inference is verified to match the trainer's reference output exactly
  ("eval-match"). SPRT-validated at **+46 Elo** over the hand-crafted evaluation.
  The classical PeSTO evaluation stays available at runtime and remains the
  library's default. See `tools/nnue/trainer/` to reproduce the net.

## Requirements

Rust (stable). If you don't have it, install via [rustup](https://rustup.rs):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

This crate builds on stable Rust 1.74+.

## Build

```sh
cargo build --release        # optimized binary at target/release/mindless
cargo test                   # run the test suite
```

## Run as a chess engine (UCI)

```sh
./target/release/mindless
```

It then speaks UCI on standard input/output. A quick manual game snippet:

```
uci
isready
ucinewgame
position startpos moves e2e4 e7e5
go movetime 1000
```

To use it in a GUI (Cute Chess, Arena, BanksiaGUI, en-croissant, ...), add a new
engine and point it at the `target/release/mindless` binary.

### UCI options

| Option       | Type   | Default | Meaning                                  |
|--------------|--------|---------|------------------------------------------|
| `Hash`       | spin   | 64      | Transposition table size in MB (1–4096). |
| `Clear Hash` | button | —       | Empty the transposition table.           |
| `EvalFile`   | string | built-in | NNUE network file to use instead of the embedded one. `<default>` (or empty) restores the built-in net; `<none>` selects the hand-crafted PeSTO evaluation. |

### Supported `go` parameters

`wtime` `btime` `winc` `binc` `movestogo` `movetime` `depth` `nodes` `infinite`.
The search stops promptly on `stop`, when a limit is reached, or when time runs
low — and never oversteps the clock.

## Command-line helpers

```sh
# Perft divide for a position (defaults to the start position).
mindless perft 6
mindless perft 5 "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1"

# Run the perft reference suite and report nodes/second.
mindless bench
```

## Use as a library

Add it as a git dependency:

```toml
[dependencies]
mindless = { git = "https://github.com/kanemoda/Mindless" }
```

Generate moves and search:

```rust
use mindless::{Board, legal_moves};
use mindless::search::{search_sync, SearchLimits};

let board = Board::startpos();

// All legal moves in the position.
let moves = legal_moves(&board);
assert_eq!(moves.len(), 20);

// Search to a fixed depth and read the best move and score.
let limits = SearchLimits { depth: Some(8), ..Default::default() };
let result = search_sync(&board, &[board.zobrist_key()], limits);
println!("best move {} (score {})", result.best_move.to_uci(), result.score);
```

The public surface is intentionally small and stable: `Board`, `Move`,
`MoveList`, `Square`, `Color`, `PieceType`, `Piece`, `CastlingRights`,
`Bitboard`, `Evaluator`/`HandCrafted`, `Tt`, the `search` module
(`think`, `search_sync`, `SearchLimits`), and the `perft` module.

## Strength & performance

Example search output (release build, one development machine; figures vary by
hardware):

- **Start position**, 5 s: reaches **depth 17** with the NNUE evaluation (it was
  depth 20 with the old hand-crafted evaluation — the network trades some search
  depth for much sharper judgement); figures vary by hardware.
- **Tactic** (`2rr3k/pp3pp1/1nnqbN1p/3pN3/2pP4/2P3Q1/PPB4P/R4RK1 w`): finds the
  winning `Qg6` and reports **mate in 2** essentially instantly.
- **Playing strength:** the NNUE evaluation is **+46 ± 16 Elo** over the
  Milestone-5 hand-crafted evaluation (SPRT, 1,512 games) and **+405 ± 45 Elo**
  over `baseline-m2`, the first playing baseline (fixed 600-game match).

Move generation remains perft-exact; `cargo run --release -- bench` reports
~254 Mnps aggregate over the reference suite.

## Testing

Unit / correctness tests:

```sh
cargo test                              # FEN, make/unmake, hashing, perft, search/tactics
cargo test --release -- --ignored       # exhaustive perft against all reference depths
```

Playing-strength testing (self-play SPRT via fastchess) — see
[TESTING.md](TESTING.md) for the full guide:

```sh
tools/sprt.sh --new wd --base baseline-m2     # is the working tree stronger than the baseline?
```

This builds both versions, plays them under a fast time control from a balanced
opening book, and prints a PASS / FAIL / CONTINUE verdict with the Elo estimate
and scoreboard. From Milestone 4 on, every strength change must pass an SPRT test
to be kept.

## Roadmap

- **Done (Milestones 4–6):** modern pruning and reductions (null-move, LMR,
  reverse futility); SEE-based ordering and pruning, continuation history, and
  late-move / history pruning; and a first SPRT-validated **NNUE evaluation**
  (embedded, on by default) — all SPRT-validated.
- **Next:** incremental NNUE accumulator updates inside the search (to recover the
  depth the network costs), larger and network-labelled training data for a
  stronger net, then opening/endgame refinements and multithreaded search.

## License

Licensed under either of MIT or Apache-2.0, at your option.
