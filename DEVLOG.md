# Mindless — Development Log (plain-language)

A running, non-technical diary of what was built in each milestone and why, plus
the results. Newest milestone first.

---

## Milestone 4 — First strength batch: smarter searching (2026-05-29)

### Goal

Make the engine genuinely *stronger* by teaching its look-ahead four well-known
"search shortcuts" that let it see much further in the same time — and prove each
one helps, by the scoreboard, before keeping it. This is the first milestone
whose changes had to earn their place: each technique was added on its own, then
played a few hundred fast games against the best previous version, and was kept
only if it won clearly more often (the SPRT test built in Milestone 3).

### What was done, and why

The four techniques, added and tested one at a time, in order:

1. **Aspiration windows.** Each time the engine searches one level deeper it
   already has a good guess of the score from the level before. Instead of
   re-examining the position with a completely open mind, it now starts from a
   narrow expectation around that guess and only widens if reality falls outside
   it. Most of the time reality matches, so the work is much cheaper.

2. **Reverse futility pruning.** Near the end of a line, if the engine is already
   doing *so* well that even a generous safety margin keeps it ahead of anything
   the opponent could hope for, it stops and accepts the position instead of
   examining every move. Clearly winning positions don't need to be picked apart.

3. **Null-move pruning.** A powerful "what if I do nothing?" test: the engine
   imagines passing its turn — handing the opponent two moves in a row — and
   takes a quick shallow look. If it is *still* winning even after that handicap,
   the real position must be very strong, so the line is pruned. This is switched
   off in the rare endgames where being forced to move is an advantage (zugzwang),
   and guarded so it can never invent a false checkmate.

4. **Late move reductions.** Because moves are already tried best-first, the
   *later*, quiet moves at each position are the ones least likely to be best. For
   those, the engine first takes a deliberately shallow look and only re-examines
   a move at full depth if that quick look is surprisingly promising. This is the
   single biggest contributor to the extra depth.

Throughout, the existing safeguards held: legal-move generation stayed perfect
(perft unchanged), forced mates are still found and reported correctly, and the
clock is still respected. Every change was a search-only change behind the same
interfaces.

### Results

Every one of the four techniques **passed** its self-play test, each measured
against the best version that came before it:

| Technique                | Verdict | Elo gain      | Games | Score (W–L–D) |
|--------------------------|:-------:|--------------:|------:|---------------|
| Aspiration windows       |  PASS   | +36.4 ± 13.1  | 1646  | 689–517–440   |
| Reverse futility pruning |  PASS   | +148.0 ± 27.3 |  500  | 293–92–115    |
| Null-move pruning        |  PASS   | +68.0 ± 18.3  |  906  | 413–238–255   |
| Late move reductions     |  PASS   | +161.6 ± 26.7 |  456  | 253–55–148    |

- **Total strength gain:** over a fixed 2,000-game match, the finished
  Milestone-4 engine measured **+348 ± 19 Elo** stronger than the Milestone-2
  baseline — it scored 88% (1618 wins, 93 losses, 289 draws), an enormous jump in
  playing strength.
- **Sees much deeper.** From the start position in five seconds it now reaches
  **depth 17**, up from **depth 9** at the baseline — nearly twice as far ahead
  for the same thinking time.
- **Quality checks clean:** optimized build, the full automated test suite
  (forced mates, tactics, time/node-limit obedience), the linter, and the
  formatter all pass; move generation remains perft-exact.

### Key decisions, in brief

- **One technique at a time, scoreboard-gated.** Each was implemented on its own
  branch and kept only after winning a self-play SPRT against the previous best —
  nothing was trusted on intuition alone.
- **Conservative, mainstream settings** for each shortcut, with the usual safety
  guards (never prune while in check, never reduce a checking move, protect
  against false mates and zugzwang). All four passed on the first attempt, with no
  tactical regressions.
- **Added a fixed-match mode** to the test script (`--fixed`) so total progress
  against a fixed reference can be measured over a set number of games, not just
  as a pass/fail verdict.
- **Tagged the result** `baseline-m4`, a new and much stronger reference point for
  measuring future progress.

### What's next

- **More search refinements** (further pruning and extension ideas), each held to
  the same scoreboard standard.
- Further out: replacing the hand-made position judgement with a **neural-network
  evaluation (NNUE)**, which slots into the interface built for it in Milestone 2.

---

## Milestone 3 — Testing infrastructure (2026-05-29)

### Goal

Build a reliable, reproducible way to **measure** whether a change makes the
engine play better, before we start making such changes. This milestone adds no
new playing ability on purpose — it is the measuring instrument that every future
strength improvement will have to satisfy. From the next milestone on, each
change is kept only if it provably wins more games.

### What was done, and why

1. **Installed a match runner (`fastchess`).** A small, fast program that plays
   two engine versions against each other and tallies results. Built from source
   (it needs only a C++ compiler — no heavy dependencies). See
   [TESTING.md](TESTING.md).

2. **Added an opening book.** Every test game starts from a slightly sharp
   position drawn from the widely-used "UHO" (Unbalanced Human Openings) book.
   Starting from off-balance positions produces more decisive games and fewer
   dull draws, which lets us detect small strength differences with far fewer
   games. The full book is enormous, so a diverse 10,000-position sample is kept
   in the repository.

3. **Wrote the SPRT test script (`tools/sprt.sh`).** One command builds the two
   versions, plays them under a fast time control, and prints a clear verdict —
   PASS (keep the change), FAIL (discard it), or CONTINUE (needs more games) —
   along with the estimated Elo gain, the scoreboard, and the confidence measure.
   "SPRT" is a statistical method that plays only as many games as needed to be
   confident, stopping early when the answer is clear.

4. **Pinned a permanent baseline.** The current engine is tagged `baseline-m2`
   so that, no matter how much the engine changes later, we can always measure
   total progress back to this fixed point.

5. **A couple of small engine touch-ups for clean testing** (no strength
   change): the engine now advertises a standard `Threads` option, and it always
   plays exactly the move it reported as best (previously, on a clock-forced cut
   it could occasionally play a slightly different move than the last line it
   displayed — harmless, but it cluttered the test logs).

6. **Validated the instrument before trusting it** (see Results).

### Results

- **The match runner works** and drives the engine correctly over UCI.
- **No bias.** Playing the engine against an identical copy of itself produced an
  Elo difference of essentially zero (−0.9 ± 1.7) with wins and losses dead even
  (163–164 with 73 draws over 400 deterministic games). This proves the harness
  does not favour either side.
- **Correctly detects real differences.** Against a deliberately crippled version
  (its thinking capped to a tiny fraction), the harness detected a huge gap
  (> +1000 Elo) and the test resolved to PASS in under 300 games — about a minute.
  It reacts quickly and in the right direction.
- **One-line workflow** is in place: `tools/sprt.sh --new wd --base baseline-m2`.
- An *absolute* strength rating against an outside reference engine was **deferred**
  (it needs careful calibration and was optional this milestone).

### Key decisions, in brief

- **fastchess over cutechess** — lighter, no GUI toolkit needed, builds from a
  single `make`.
- **UHO opening book** — the standard choice for sharp, low-draw, high-resolution
  testing; sampled down to keep the repository small.
- **SPRT with bounds (0, 5) and 5% error rates** — the conventional, efficient
  setup that confirms or rejects a change in as few games as possible.
- **Validate before trusting** — the no-bias and handicap checks prove the
  measuring instrument is sound, so future verdicts can be believed.

### What's next

- **Milestone 4:** the first real strength features (search shortcuts such as
  null-move pruning and late-move reductions), each one proposed on a branch and
  kept only if it passes an SPRT test against the previous version.

---

## Milestone 2 — Baseline search + evaluation (2026-05-29)

### Goal

Turn the rule-following random-move player from Milestone 1 into one that
actually *plays* chess: looks ahead, judges who is better, manages its clock, and
explains its thinking — all correctly, and at a strength we can measure. The
deliberate scope limit: build a **clean, correct baseline**, and leave the
clever speed-up tricks (and the neural-network evaluation) for later milestones,
so each can be added and proven to help rather than piled on untested.

### What was done, and why

1. **Added a position evaluator.** The engine now scores a position by material
   plus where the pieces stand, blending between opening and endgame values as
   the board empties (a "tapered" evaluation), using a well-known expert-tuned
   set of tables. Crucially, the evaluation sits behind a swap-in interface so a
   future neural network can replace it without touching the search — a planned
   requirement.

2. **Built the search.** Negamax with alpha-beta: the engine looks ahead and
   prunes lines it has already proven are not worth examining. It deepens
   iteratively (one ply, then two, ...) so it always has a ready answer when the
   clock demands one, and it reports the full best line it foresees.

3. **Added quiescence search.** At the end of the look-ahead it keeps following
   captures until the position is calm, so it never judges a position in the
   middle of an exchange (the "horizon effect").

4. **Added a transposition table.** A large, size-configurable memory of already
   analysed positions, so repeated positions (reached by different move orders)
   are not re-analysed. Mate scores are stored carefully so "mate in N" stays
   correct no matter where in the tree it is found.

5. **Ordered moves well.** The engine tries the most promising moves first (the
   move that worked before, valuable captures, recently-good quiet moves). This
   is the single biggest factor in how deep it can see in the available time.

6. **Handled draws correctly.** Threefold repetition, the fifty-move rule, and
   insufficient material are all recognised and scored as draws.

7. **Added time management.** It budgets time from the clock with a soft target
   and a hard ceiling, always keeping a safety margin so it never loses on time,
   and it honours every standard time/depth/node instruction a chess program can
   send.

8. **Wired it into the UCI conversation** with live "thinking" output and the
   standard `Hash` (memory size) and `Clear Hash` options, on a background thread
   so the engine stays responsive (it can be told to stop mid-think).

9. **Tested and sanity-checked.** New automated tests confirm it finds forced
   mates, wins free material, recognises stalemate, and obeys time/node limits;
   the existing rule-correctness tests still pass. Two full engine-vs-itself
   games were played to confirm coherent, legal, crash-free play.

### Results

- **Plays real, coherent chess.** In self-play it opens sensibly (develops
  pieces, castles), conducts middlegames, and reaches natural endings. Two
  sanity games: one ended decisively (checkmate/stalemate) after 173 half-moves,
  the other was drawn by repetition — no crashes, responsive throughout.
- **Solves tactics.** On a standard tactical test position it instantly finds
  the winning move and announces a forced mate in two.
- **Searches to a useful depth.** From the start position it reaches about depth
  9 in five seconds on the development machine (this baseline omits the
  deeper-searching shortcuts coming in Milestone 3).
- **Respects the clock** and never oversteps its time budget.
- **Quality checks clean:** optimized build, linter (`clippy`), and formatter all
  pass with no warnings; the full automated test suite is green; move generation
  remains perft-exact.

### Key decisions, in brief

- **Evaluation behind a swap-in interface** so NNUE can replace it later with no
  search changes.
- **A clean, correct baseline on purpose** — no aggressive pruning yet, so that
  Milestone 3 can add those one at a time and *measure* each with automated
  self-play (SPRT) testing.
- **Search on its own thread** so the engine stays responsive to "stop" and can
  think indefinitely when asked.
- **Reused Milestone 1's foundation directly** — the fast move-maker, the
  position fingerprints, and the perft-verified move generator all carried over
  unchanged, which is exactly what they were designed for.

### What's next

- **Milestone 3:** the deeper-searching shortcuts (null-move, late-move
  reductions, futility and capture pruning), each added and validated by
  automated self-play testing (SPRT) so we only keep changes that measurably win
  more games. Further out: the NNUE neural-network evaluation.

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
