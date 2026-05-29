# Mindless — Architecture (plain-language overview)

This document explains, in everyday language, **what** Mindless is made of and
**why** it is built that way. It is written for readers who do not read source
code. There is no programming knowledge required.

Each milestone adds a section here describing what it contributed and the
reasoning behind the choices.

---

## The big picture

A strong chess engine is built in two layers:

1. **The rules-and-bookkeeping layer** — knowing the board, knowing every legal
   move, and being able to play a move and take it back instantly. This layer
   has to be *perfect*: a single wrong or missing move would poison everything
   built on top of it.
2. **The thinking layer** — searching millions of possible continuations and
   judging which position is best. This is what makes an engine *strong*.

**Milestone 1 built the entire first layer** and a basic way to talk to chess
programs. **Milestone 2 builds the first version of the thinking layer**: the
engine now searches ahead, judges positions, manages its clock, and plays real
(if not yet expert) chess instead of random moves.

Two design goals shaped every decision:

- **Maximum eventual strength.** Choices were made so the thinking layer can be
  bolted on later without rebuilding the foundation.
- **Clean reuse by other developers.** Mindless is simultaneously a finished
  program *and* a toolkit other Rust projects can import.

---

## The building blocks of Milestone 1

### 1. How the board is stored — "bitboards"

A chess board has 64 squares. A modern computer naturally works with 64-bit
numbers — think of a row of 64 tiny on/off switches. Mindless represents the
board as a stack of these switch-rows, one per kind of piece: one row marks
where the white pawns are, another the black knights, and so on.

**Why:** asking questions like "where can this rook move?" or "is the king in
danger?" becomes a matter of flipping and combining these switch-rows, which is
something a processor can do billions of times per second. This is the standard
foundation of every top-tier engine, and it is what later makes a fast search
possible.

Alongside the switch-rows, Mindless also keeps a simple 64-square list of "what
is on this square," because some questions are quicker to answer that way. The
two views are always kept in agreement.

### 2. Tracking the full state of the game

Beyond piece positions, a chess position includes details that affect the rules:
whose turn it is, who may still castle, whether an *en passant* pawn capture is
momentarily available, and the move counters used for draw rules. Mindless
tracks all of these, because move legality genuinely depends on them.

### 3. A position's "fingerprint" — hashing

Every position gets a single large number that acts like a fingerprint. Two
identical positions get the same fingerprint; different positions almost
certainly get different ones.

**Why:** the future thinking layer will constantly ask "have I already analyzed
this exact position?" Comparing fingerprints is far faster than comparing whole
boards. Crucially, Mindless updates the fingerprint *incrementally* — when a
move is played it adjusts only the parts that changed, rather than recomputing
from scratch. We build this now so the thinking layer gets it for free later.

### 4. Reading and writing positions — "FEN"

FEN is the universal shorthand the chess world uses to write down a position as
a short line of text. Mindless can both read FEN (to set up any position) and
write it back out, and we verified that a position survives a round trip
unchanged.

**Why:** every chess program, test, and database speaks FEN. Supporting it makes
Mindless interoperable and easy to test.

### 5. Knowing where pieces can move

This is the heart of the milestone. Short-range pieces (knights, kings, pawns)
are easy: their moves are precomputed once into lookup tables.

The long-range pieces — bishops, rooks, queens — are harder, because how far
they slide depends on what is in the way. Mindless uses a well-known, very fast
technique called **"magic bitboards."** The intuition: for each square we found
a special multiplier number that instantly turns "what's blocking this rook"
into "here is exactly where it can go," via a single lookup. Finding those magic
numbers is a one-time setup the program does for itself when it starts.

**Why:** sliding-piece moves are the most frequently asked question in a chess
search, so making them nearly instant is essential for future strength.

### 6. Generating *only* legal moves

There are two ways to list a side's moves. The easy way lists plausible moves
and then throws out the illegal ones afterward. Mindless instead generates only
genuinely legal moves directly — it works out in advance which pieces are
"pinned" in front of their own king, whether the king is in check and how that
check can be answered, and which squares the king may safely step onto.

**Why:** it is faster (no wasted work) and it forces us to handle the genuinely
tricky rules — castling through or into danger, pinned pieces, and the rare
*en passant* capture that secretly exposes the king — correctly and explicitly.
The trickiest of these (en passant) is double-checked by briefly simulating the
capture to confirm the king is left safe.

### 7. Playing a move and taking it back instantly

The thinking layer will play a move, look ahead, and then *undo* it, millions of
times per second. Mindless supports playing a move and undoing it exactly,
restoring the position — fingerprint and all — to precisely what it was.

**Why:** this "make/unmake" cycle is the single most-used operation in a chess
search, so it is built to be fast and exact from day one.

### 8. The correctness test — "perft"

How do you *prove* the move rules are perfect? The chess-programming community
uses a standard test called **perft**: from a known starting position, count
every possible sequence of moves out to a given number of moves. Decades of
engines have established the exact correct totals for a set of deliberately
tricky positions.

Mindless reproduces **every one of these totals exactly**, including positions
specifically designed to trip up castling, promotions, and en passant. Matching
them is the gold-standard evidence that the foundation is bug-free. The same
machinery doubles as a speed benchmark.

### 9. Talking to chess programs — "UCI"

UCI is the common language chess programs (the graphical boards people actually
use) speak to engines. Mindless implements the essential UCI conversation:
introduce itself, accept a position, and answer with a move. This is why it can
be loaded into a normal chess program today and play a complete, legal game
without crashing.

---

## The building blocks of Milestone 2 — the thinking layer

Milestone 2 turns the rule-follower into a player. It can now look ahead, form
an opinion about who is better, and choose a move on a clock.

### 1. Judging a position — "evaluation"

To choose between moves, the engine needs a number that says how good a position
is. Mindless adds up the material (a queen is worth more than a pawn) and adjusts
for *where* each piece sits — a knight in the centre is worth more than one in
the corner, a king should hide in the opening but march out in the endgame. These
positional bonuses come from a well-known, expert-tuned set of tables.

A key subtlety: the right values change as pieces come off the board. Mindless
therefore keeps two sets of numbers — one for the opening/middlegame, one for the
endgame — and blends between them based on how much material remains. This is
called a *tapered* evaluation.

**Why (and an important design choice):** the whole evaluation lives behind a
clean "swap-in" interface. A future milestone can replace these hand-made tables
with a neural network (NNUE) **without touching the search code at all**. That
separation was a deliberate requirement, planned from the start.

### 2. Looking ahead — "search"

The engine plays out lines of moves and replies in its head, many plies deep, and
keeps the move that leads to the best position it can force. The core technique
is *alpha-beta*: once a reply is found that's good enough to refute a candidate
move, the engine stops examining that candidate — there's no point proving it's
*even worse*. This safely skips huge numbers of pointless lines without ever
changing the conclusion.

It searches **iteratively**: first to a depth of one move, then two, then three,
and so on, until its time runs out. That sounds wasteful, but each shallow pass
makes the next deeper pass dramatically faster (it already knows which moves look
best and tries them first), and it guarantees the engine always has a complete,
usable answer ready the moment its clock forces it to move.

### 3. Not stopping in the middle of a fight — "quiescence search"

If the engine simply stopped looking after N moves, it might stop right after
grabbing a queen — without noticing its own queen gets recaptured next move. This
is the "horizon effect." To avoid it, when the main look-ahead ends, Mindless
keeps following just the captures (and pawn promotions) until the position is
calm, so its judgement is never based on the middle of an exchange.

### 4. Remembering what it already worked out — "transposition table"

Different move orders often reach the same position. Mindless keeps a large
memory of positions it has already analysed (keyed by the position fingerprint
from Milestone 1), so it can reuse that work instantly instead of redoing it.
The size of this memory is adjustable by the user, and it's emptied at the start
of each new game.

### 5. Trying the best moves first — "move ordering"

Alpha-beta is only fast if the strong moves are examined early. Mindless orders
each position's moves smartly: the move that worked here before, then captures of
valuable pieces, then a couple of "quiet" moves that recently proved good
elsewhere. Good ordering is the single biggest reason the search reaches useful
depths in the available time.

### 6. Knowing when it's a draw

The engine correctly recognises the three ways a game draws without checkmate:
the same position repeating, fifty moves passing with no pawn move or capture,
and too little material left to mate. It scores these as dead-even, so it neither
blunders into a draw when winning nor misses one when lost.

### 7. Managing the clock

Given how much time is left (and any increment), Mindless budgets a sensible
amount for each move, with a soft target and a hard ceiling, and stops the moment
either is reached. It deliberately leaves a safety margin so it never loses on
time.

### 8. Showing its thinking

While it searches, the engine continuously reports the depth it has reached, its
current best line of play, its evaluation (or "mate in N"), how many positions
it has examined and how fast, and how full its memory is. Chess programs display
this, and it's invaluable for debugging and for the automated testing that later
milestones will rely on.

> **Not yet (by design):** this milestone is a clean, correct *baseline*. The
> clever shortcuts that let top engines search much deeper for the same time
> (and the neural-network evaluation) are deferred to later milestones, where
> each can be added and then *measured* to confirm it genuinely makes the engine
> stronger.

---

## Milestone 3 — the measuring instrument

Milestone 3 added no playing ability. Instead it built the **scoreboard** that
every future improvement must satisfy: a way to play a candidate version against
the previous one over hundreds of fast games and decide, with controlled error
rates, whether it is genuinely stronger.

In plain terms: a small match-running program plays the two versions against
each other from a book of slightly off-balance openings (which produces decisive
games and sharp measurements), and a statistical stopping rule (SPRT) plays only
as many games as needed before declaring **pass**, **fail**, or **keep going**.
A one-line script wraps the whole thing, and the current engine is pinned with a
permanent tag so total progress can always be measured back to this point.

This instrument was itself validated — it shows no bias when a version plays an
identical copy of itself, and it quickly and correctly detects a deliberately
weakened version. From Milestone 4 on, no strength change is kept unless it
passes this test. The full how-to lives in `TESTING.md`.

---

## How the project is organized

The code is split into focused, well-named parts, each responsible for one idea:
the board, the bitboards, the move encoding, the magic sliding-piece tables, the
hashing keys, the move generator, the correctness test, the evaluation, the
position memory (transposition table), the search itself, and the UCI
conversation. This separation keeps each piece simple to understand and test on
its own, and it is what lets other developers import just the parts they need —
and it is what will let the evaluation be swapped for a neural network later
without disturbing the search.

Everything is written in safe Rust with no external dependencies, which keeps the
project self-contained, portable, and easy to trust.

---

## What is deliberately *not* here yet

- **No advanced search shortcuts.** The pruning and reduction tricks that let
  top engines search far deeper in the same time are intentionally left for
  Milestone 3, where each will be added *and measured* (via automated self-play
  testing) to prove it genuinely strengthens the engine.
- **No neural-network evaluation.** The current evaluation is a solid hand-made
  one; the much stronger NNUE network arrives in a later milestone and will slot
  into the interface already built for it.

These are the subject of upcoming milestones, and the architecture above was
designed specifically to support them.
