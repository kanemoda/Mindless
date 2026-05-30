# Mindless — Testing Guide (plain-language)

This document explains, in everyday terms, how we measure whether a change to
the engine actually makes it play **better**. From here on, every change that is
meant to improve playing strength must earn its place by passing this test.

You do not need to read any source code to follow this.

---

## Why this exists

Chess-engine changes are deceptive. A tweak that "should obviously help" often
does nothing, or quietly makes the engine *weaker*. The only trustworthy judge
is the scoreboard: let the new version play a few hundred or few thousand very
fast games against the old version and see if it really wins more.

This milestone built the machinery to do exactly that, reliably and
reproducibly. It adds **no** new playing ability itself — it is the measuring
instrument that all future strength work depends on.

---

## The three pieces

1. **A match runner — `fastchess`.** A small, fast program that sits between two
   engines, feeds them positions, enforces the clock, and tallies the results.
   It is built from source (see "Rebuilding the tools" below) and kept at
   `tools/bin/fastchess`.

2. **An opening book — `UHO_Lichess_4852_v1` (sampled).** Every test game starts
   from a position taken from this book rather than the standard start position.
   We use the *UHO* ("Unbalanced Human Openings") book: real opening positions
   where one side has a slight edge. Starting from slightly sharp positions means
   fewer boring draws and more decisive games, which lets us measure small
   strength differences with far fewer games. It is the de-facto standard for
   this kind of testing. The full book is huge (2.6 million positions), so the
   repository keeps a diverse 10,000-position sample at
   `tools/books/UHO_Lichess_4852_v1_sample.epd`.

3. **The test script — `tools/sprt.sh`.** One command that builds the two engine
   versions, plays them against each other under the rules below, and prints a
   clear verdict.

---

## What "SPRT" means

SPRT (Sequential Probability Ratio Test) is a clever way to answer "is the new
version stronger?" using as few games as possible.

Instead of fixing a number of games in advance, it watches the running score and
keeps playing **only until it is confident** of the answer — then stops
immediately. It is set up with two guard-rails:

- It will only declare the new version a winner if the evidence says it is at
  least a few Elo stronger (we use **elo1 = 5**, i.e. "is it ≥ 5 Elo better?").
- It will declare failure if the evidence says the gain is essentially zero
  (**elo0 = 0**).
- The chance of a wrong "pass" or wrong "fail" is each capped at 5%
  (**alpha = beta = 0.05**).

A strong improvement is confirmed in a few hundred games; a useless change is
rejected just as quickly; only genuinely borderline changes take many games.

---

## How to run a test

The standard question is "does the change I just made beat the previous
version?" From the project directory:

```sh
tools/sprt.sh --new wd --base baseline-m2
```

- `--new wd` means "the engine as it is right now in my working folder."
- `--base baseline-m2` means "the fixed reference version" (a permanent tag, see
  below). For a rolling per-change comparison you can instead point `--base` at
  the previous commit or any saved version.

Useful knobs (all optional, with sensible defaults):

| Flag            | Meaning                                   | Default              |
|-----------------|-------------------------------------------|----------------------|
| `--new` / `--base` | which versions to compare (a tag, commit, or `wd`) | `wd` / `baseline-m2` |
| `--tc`          | time per game ("seconds+increment")       | `8+0.08`             |
| `--concurrency` | how many games at once                    | cores − 2            |
| `--elo0`/`--elo1` | the SPRT guard-rails                     | `0` / `5`            |
| `--rounds`      | safety cap on number of games             | very large           |
| `--fixed`       | play all `--rounds` games, no early stop  | off                  |

The defaults run a fast game (8 seconds + 0.08s/move), use most of the CPU while
leaving a couple of cores free, and apply the bounds above.

`--fixed` turns off the SPRT early-stopping rule so the script plays a set number
of games and reports the Elo difference over all of them. The SPRT mode is best
for a quick pass/fail decision on a change; `--fixed` is best for *measuring* how
much stronger one version is than another (for example, the total gain of a
finished milestone against a fixed baseline) over a chosen, larger sample. For
example, to gauge total progress since the baseline:

```sh
tools/sprt.sh --new wd --base baseline-m2 --fixed --rounds 1000
```

---

## How to read the verdict

At the end the script prints a block like this:

```
Elo: 1107.46 +/- nan, nElo: ...
Games: 294, Wins: 293, Losses: 0, Draws: 1, Points: 293.5 (99.83 %)
Ptnml(0-2): [0, 0, 0, 1, 146]
LLR: 2.96 (...) (-2.94, 2.94) [0.00, 5.00]
Verdict: PASS  — H1 accepted (new is >= elo1 Elo stronger)
```

- **Verdict** is the bottom line:
  - **PASS** — the new version is confirmed stronger; keep the change.
  - **FAIL** — the change did not help (or hurt); discard it.
  - **CONTINUE** — not enough evidence yet; let it run longer.
- **Elo** is the estimated strength gain (in the universal chess rating unit),
  with its error margin. Positive = stronger.
- **Wins/Losses/Draws** is the raw scoreboard from the new version's side.
- **LLR** is the running confidence measure; the test stops when it reaches the
  upper bound (PASS) or lower bound (FAIL) shown in parentheses.
- **Ptnml** ("pentanomial") groups games into pairs played from the same opening
  with reversed colours; it is the most accurate way to summarise the result.

---

## The workflow from Milestone 4 onward

1. Make a strength change on a branch.
2. Run `tools/sprt.sh --new wd --base <previous version>`.
3. **Keep the change only if it PASSes.** Otherwise discard it.
4. Periodically, also compare against the permanent `baseline-m2` tag to see the
   total progress accumulated since this point.

This discipline — every change measured, only proven gains kept — is how the
engine will climb in strength without guesswork or regressions.

---

## Testing a new evaluation — the NNUE "eval-match"

Milestone 6 added a neural-network evaluation (NNUE). A *learned* evaluation needs
one extra check **before** any game is played, on top of the usual SPRT.

**The eval-match.** The network is trained by a separate program (bullet) and then
run by the engine's own, independently written inference code. Those two must
agree on a position's score to the last centipawn — otherwise the engine is not
playing the network it was trained on, and no game result means anything. To check:

```sh
mindless eval <net.bin> "<FEN>"        # the engine's score for one position
mindless eval <net.bin> < fens.txt     # or a list of FENs, one per line
```

The same FENs are scored by the trainer's reference inference
(`tools/nnue/trainer/refeval.rs`); the two centipawn columns must be **identical**.
This is run on a spread of positions — both sides to move, openings through
endgames — and only once it matches exactly is the net trusted enough to test for
strength. The Milestone-6 net matched exactly on every position tried.

**Then the usual SPRT.** A trusted net is gated like any other strength change: it
must beat the previous version on the scoreboard. The net is embedded in the binary
and on by default, so the test is the normal comparison:

```sh
tools/sprt.sh --new wd --base baseline-m5     # network engine vs the hand-crafted one
```

To test a *specific* net file without rebuilding, or to switch a side's evaluation
within one build, pass the engine option through the runner:

```sh
# load a particular net into the new engine
tools/sprt.sh --new wd --base baseline-m5 --new-limit "option.EvalFile=/abs/path/net.bin"
# turn the network off (hand-crafted PeSTO) on one side
tools/sprt.sh --new wd --base wd --base-limit "option.EvalFile=<none>"
```

The Milestone-6 net passed at **+46 ± 16 Elo** over `baseline-m5` (hand-crafted).

---

## How we know the instrument itself is trustworthy

Before relying on it, the harness was validated two ways:

- **Identical-versions test (no-bias check).** The engine was played against an
  exact copy of itself. Result: **Elo ≈ 0** (−0.9 ± 1.7) with wins and losses
  essentially equal (163–164–73 over 400 deterministic games). This confirms the
  harness has no built-in bias toward either side.
- **Handicap test (sensitivity check).** A deliberately crippled version (its
  thinking capped to a tiny fraction) was played against the normal engine. The
  harness correctly detected a huge gap (over +1000 Elo) and the test resolved to
  **PASS in under 300 games / about a minute** — confirming it reacts quickly and
  in the right direction when there is a real difference.

A rough *absolute* strength rating against an external reference engine was
deferred to a later milestone (it needs careful calibration).

---

## Rebuilding the tools

The `fastchess` binary is not stored in the repository (only the script and the
book sample are). To rebuild it on a fresh machine:

```sh
git clone https://github.com/Disservin/fastchess.git
cd fastchess && make -j
# then copy the resulting ./fastchess into the project's tools/bin/ folder
```

It needs only a C++17 compiler and `make` — no other dependencies. The full
opening book can be obtained from `https://github.com/official-stockfish/books`
if you want more openings than the bundled sample.
