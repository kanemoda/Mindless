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

**Milestone 1 builds the entire first layer** and a basic way to talk to chess
programs. The thinking layer comes in later milestones. So today Mindless knows
the rules of chess flawlessly and can play legal moves, but it does not yet try
to play *good* moves — when asked to move it picks a random legal one.

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

## How the project is organized

The code is split into focused, well-named parts, each responsible for one idea:
the board, the bitboards, the move encoding, the magic sliding-piece tables, the
hashing keys, the move generator, the correctness test, and the UCI conversation.
This separation keeps each piece simple to understand and test on its own, and it
is what lets other developers import just the parts they need.

Everything is written in safe Rust with no external dependencies, which keeps the
project self-contained, portable, and easy to trust.

---

## What is deliberately *not* here yet

- No searching ahead and no judgement of who is winning — the engine does not yet
  try to play well, only legally.
- No clock management.

These are the subject of upcoming milestones, and the foundation above was
designed specifically to support them.
