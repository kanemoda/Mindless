# Mindless

A UCI chess engine written in Rust, built for both strength and clean reuse.

Mindless is a single Rust crate that is **both** a runnable chess engine (the
`mindless` binary speaks the Universal Chess Interface, so any standard chess GUI
can run it) **and** a library (other Rust projects can depend on it to represent
positions and generate moves).

> **Status — Milestone 1 (Foundation).** This milestone delivers the engine's
> foundation: board representation, fully-legal move generation with magic
> bitboards, a verified perft correctness harness, and a working UCI skeleton.
> It does **not** yet search or evaluate — `go` currently returns a random legal
> move. Search, evaluation, time management and tuning arrive in later
> milestones. See [DEVLOG.md](DEVLOG.md) and [ARCHITECTURE.md](ARCHITECTURE.md).

## Features in this milestone

- **Bitboard board representation** with full game state (piece placement, side
  to move, castling rights, en passant, halfmove clock, fullmove number).
- **Incrementally-updated Zobrist hashing** (compile-time generated keys).
- **FEN parsing and serialization**, verified to round-trip exactly.
- **Magic bitboards** for sliding-piece (rook/bishop/queen) attacks.
- **Fully legal move generation** handling castling, en passant (including the
  discovered-check edge cases), all four promotions, pins and check evasions.
- **Compact 16-bit move encoding** and fast make/unmake.
- **Perft + perft divide** verified against the standard reference positions to
  the published node counts.
- **UCI skeleton**: `uci`, `isready`, `ucinewgame`, `position`, `go`, `stop`,
  `quit`, plus `d` and `perft` debug commands.
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

It then waits for UCI commands on standard input. A quick manual check:

```
uci
position startpos moves e2e4
go
```

To use it in a GUI (Cute Chess, Arena, BanksiaGUI, etc.), add a new engine and
point it at the `target/release/mindless` binary. It will load and play legal
(currently random) games without crashing.

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

Then drive the core directly:

```rust
use mindless::{Board, legal_moves};
use mindless::perft::perft;

let mut board = Board::startpos();

// Enumerate the legal moves in the position.
let moves = legal_moves(&board);
println!("{} legal moves", moves.len());

// Make and unmake a move (the engine's search hot path).
let undo = board.make_move(moves[0]);
// ... explore the resulting position ...
board.unmake_move(moves[0], undo);

// Count leaf nodes to a given depth.
assert_eq!(perft(&mut board, 4), 197_281);
```

The public surface is intentionally small and stable: `Board`, `Move`,
`MoveList`, `Square`, `Color`, `PieceType`, `Piece`, `CastlingRights`,
`Bitboard`, `legal_moves` / `generate_legal`, and the `perft` module.

## Perft correctness & performance

All standard reference positions match the published node counts exactly. Speeds
below are from one development machine (release build) and will vary by hardware:

| Position   | Depth |        Nodes | Speed (Mnps) |
|------------|------:|-------------:|-------------:|
| startpos   |     6 |  119,060,324 |        ~221  |
| Kiwipete   |     5 |  193,690,690 |        ~279  |
| position 3 |     6 |   11,030,083 |        ~178  |
| position 4 |     5 |   15,833,292 |        ~248  |
| position 5 |     5 |   89,941,194 |        ~276  |
| position 6 |     5 |  164,075,551 |        ~274  |

Reproduce with `cargo run --release -- bench`.

## Testing

```sh
cargo test                              # fast checks (FEN, make/unmake, hashing, shallow perft)
cargo test --release -- --ignored       # exhaustive perft against all reference depths
```

## Roadmap

- **Milestone 2+:** alpha-beta search with modern pruning, transposition table,
  NNUE evaluation, time management, and SPRT-based tuning.

## License

Licensed under either of MIT or Apache-2.0, at your option.
