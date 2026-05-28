# Mindless — Development Log (plain-language)

A running, non-technical diary of what was built in each milestone and why, plus
the results. Newest milestone first.

---

## Milestone 1 — Foundation (2026-05-29)

### Goal

Lay a flawless foundation for the engine: set up the tools, teach the program
the full rules of chess, prove that knowledge is perfect, and let it talk to
ordinary chess programs. No "thinking" yet — that is for later milestones. Every
choice was made to support that future thinking layer without rework.

### What was done, and why

1. **Installed the build tools.** The machine had no Rust toolchain, so the
   standard Rust installer was used to set it up. This is what compiles the
   engine. Verified the compiler and build tool were working before writing any
   code.

2. **Created the project as both a program and a toolkit.** Mindless is set up
   so it can be run as a standalone chess engine *and* imported by other Rust
   projects as a reusable library. This was a project goal from the start: clean
   integration by other developers.

3. **Built the board and game state.** Implemented the fast "bitboard"
   representation of the 64 squares plus all the extra state the rules depend on
   (whose turn, castling availability, en passant, move counters). See
   [ARCHITECTURE.md](ARCHITECTURE.md) for what bitboards are in plain terms.

4. **Added position fingerprinting (hashing).** Each position gets a unique
   fingerprint that updates instantly as moves are played. The future thinking
   layer needs this to recognize repeated positions quickly, so it was built in
   now.

5. **Added FEN reading and writing.** FEN is the standard text shorthand for a
   chess position. Mindless reads and writes it, and a test confirms a position
   survives the round trip unchanged.

6. **Implemented move generation, including the hard parts.** Short-range pieces
   use precomputed tables; long-range pieces use the fast "magic bitboard"
   technique. The generator produces only fully legal moves and correctly
   handles every awkward rule: castling through/into check, pinned pieces, all
   four promotion choices, check escapes, and the notorious en passant capture
   that can secretly expose one's own king.

7. **Implemented fast make / unmake.** The engine can play a move and undo it
   exactly — the single most-used operation in a future search.

8. **Built the correctness harness (perft) and ran it.** Counted every possible
   sequence of moves out to several moves deep for the standard set of test
   positions and compared against the totals the chess-programming community has
   long established as correct.

9. **Implemented the UCI skeleton.** The engine now understands the standard
   commands a chess program sends, can be given any position, and replies with a
   legal move. (For now that move is chosen at random, because there is no
   thinking layer yet.)

10. **Wrote tests and documentation.** Automated tests cover FEN round-tripping,
    make/unmake exactness, hashing consistency, checkmate/stalemate detection,
    and perft. Documentation (this log, the architecture overview, and the
    README) was written for both developers and non-technical readers.

### Results

- **Correctness: perfect.** Every standard reference position matches the known
  exact move counts. The toughest positions (which specifically stress castling,
  promotions, and en passant) all pass:

  | Position   | Depth |        Total moves counted | Matches reference? |
  |------------|------:|---------------------------:|:------------------:|
  | startpos   |     6 |                119,060,324 |        yes         |
  | Kiwipete   |     5 |                193,690,690 |        yes         |
  | position 3 |     6 |                 11,030,083 |        yes         |
  | position 4 |     5 |                 15,833,292 |        yes         |
  | position 5 |     5 |                 89,941,194 |        yes         |
  | position 6 |     5 |                164,075,551 |        yes         |

- **Speed: strong.** On the development machine the engine counts roughly
  **260 million positions per second** overall in this test (release build;
  exact numbers vary by hardware). This is a healthy, top-tier-range starting
  point for the search work to come.

- **Quality checks: clean.** The optimized build compiles without warnings, the
  linter is clean, and the full test suite passes.

- **Plays in a GUI.** The engine loads in a standard chess program and plays
  complete, legal games without crashing.

### Key decisions, in brief

- **Bitboards + magic bitboards** for speed, because the future search will lean
  on fast move generation more than anything else.
- **Generate only legal moves directly** rather than filtering afterwards —
  faster, and it forced the tricky rules to be handled correctly and provably.
- **Incremental hashing and fast make/unmake** built now, so the search layer
  inherits them for free.
- **Zero external dependencies, all safe Rust** — self-contained, portable, and
  easy to trust and audit.
- **Verified against perft** to the published counts before moving on, on the
  principle that the foundation must be perfect before anything is built on it.

### What's next

- **Milestone 2 and beyond:** the thinking layer — searching ahead with modern
  pruning, judging positions (eventually with a neural-network evaluation),
  managing the clock, and tuning strength through automated self-play testing.
