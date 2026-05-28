# Mindless

A UCI chess engine written in Rust, built for both strength and clean reuse.

Mindless is a single Rust crate that is **both** a runnable chess engine (the
`mindless` binary speaks the Universal Chess Interface, so any standard chess GUI
can run it) **and** a library (other Rust projects can depend on it to represent
positions, generate moves, and search).

> **Status — Milestone 2 (Baseline search + evaluation).** Mindless now *plays
> chess*: it searches with alpha-beta, evaluates positions, manages its clock,
> and reports its thinking over UCI. It is a clean, correct baseline — modern
> pruning/reductions (null-move, LMR, futility) and NNUE evaluation are the
> subject of later milestones. See [DEVLOG.md](DEVLOG.md) and
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

- **Start position**, 5 s: reaches depth 9, score ≈ +0.33, ~6.7 Mnps, PV
  `e2e4 e7e6 g1f3 d7d5 e4d5 e6d5 f1e2 g8f6 e1g1`.
- **Tactic** (`2rr3k/pp3pp1/1nnqbN1p/3pN3/2pP4/2P3Q1/PPB4P/R4RK1 w`): finds the
  winning `Qg6` and reports **mate in 2** essentially instantly.

Move generation remains perft-exact; `cargo run --release -- bench` reports
~260 Mnps aggregate over the reference suite.

## Testing

```sh
cargo test                              # FEN, make/unmake, hashing, perft, search/tactics
cargo test --release -- --ignored       # exhaustive perft against all reference depths
```

## Roadmap

- **Milestone 3:** modern pruning and reductions (null-move, LMR, futility,
  SEE-based pruning), tuned and validated with SPRT.
- **Beyond:** NNUE evaluation, opening/endgame refinements, multithreaded search.

## License

Licensed under either of MIT or Apache-2.0, at your option.
