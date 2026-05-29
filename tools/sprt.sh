#!/usr/bin/env bash
#
# tools/sprt.sh — Run a self-play SPRT match between two builds of Mindless and
# print a clear PASS / FAIL / CONTINUE verdict.
#
# It builds a "new" and a "base" engine (from git refs, or the current working
# tree), plays them against each other with fastchess under an SPRT stopping
# rule, and reports the verdict, the log-likelihood ratio versus its bounds, the
# Elo estimate with error bars, the W-L-D, and the pentanomial breakdown.
#
# Typical use (test the change you're working on against the previous version):
#   tools/sprt.sh --new wd --base baseline-m2
#
# See TESTING.md for the full explanation.

set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# ---- defaults (all overridable via flags) ----
NEW_REF="wd"                                              # "wd" = current working tree
BASE_REF="baseline-m2"
TC="8+0.08"                                               # time control: seconds + increment
BOOK="tools/books/UHO_Lichess_4852_v1_sample.epd"
HASH="16"                                                 # per-engine hash in MB
ELO0="0"; ELO1="5"; ALPHA="0.05"; BETA="0.05"             # SPRT bounds
ROUNDS="40000"                                            # max opening pairs (SPRT usually stops first)
GAMES="2"                                                 # games per round (2 = play both colors)
NEW_LIMIT=""; BASE_LIMIT=""                               # extra per-engine fastchess tokens
CONCURRENCY=""                                            # default: cores - 2
FASTCHESS="${FASTCHESS:-$ROOT/tools/bin/fastchess}"

usage() {
    sed -n '3,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    cat <<'EOF'

Flags:
  --new REF         new engine ref ("wd" = working tree). Default: wd
  --base REF        base engine ref. Default: baseline-m2
  --tc TC           time control "secs+inc". Default: 8+0.08
  --book FILE       opening book (EPD). Default: the bundled UHO sample
  --hash MB         per-engine hash. Default: 16
  --elo0/--elo1     SPRT bounds. Default: 0 / 5
  --alpha/--beta    SPRT error rates. Default: 0.05 / 0.05
  --rounds N        max opening pairs. Default: 40000
  --concurrency N   parallel games. Default: cores-2
  --new-limit STR   extra per-engine tokens for new  (e.g. "nodes=20000")
  --base-limit STR  extra per-engine tokens for base (e.g. "nodes=20000")
  --fastchess PATH  fastchess binary. Default: tools/bin/fastchess
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --new) NEW_REF="$2"; shift 2;;
        --base) BASE_REF="$2"; shift 2;;
        --tc) TC="$2"; shift 2;;
        --book) BOOK="$2"; shift 2;;
        --hash) HASH="$2"; shift 2;;
        --elo0) ELO0="$2"; shift 2;;
        --elo1) ELO1="$2"; shift 2;;
        --alpha) ALPHA="$2"; shift 2;;
        --beta) BETA="$2"; shift 2;;
        --rounds) ROUNDS="$2"; shift 2;;
        --games) GAMES="$2"; shift 2;;
        --concurrency) CONCURRENCY="$2"; shift 2;;
        --new-limit) NEW_LIMIT="$2"; shift 2;;
        --base-limit) BASE_LIMIT="$2"; shift 2;;
        --fastchess) FASTCHESS="$2"; shift 2;;
        -h|--help) usage; exit 0;;
        *) echo "unknown argument: $1" >&2; usage; exit 1;;
    esac
done

if [[ -z "$CONCURRENCY" ]]; then
    CORES="$(nproc)"
    CONCURRENCY=$(( CORES > 3 ? CORES - 2 : 1 ))
fi

[[ -x "$FASTCHESS" ]] || { echo "ERROR: fastchess not found at '$FASTCHESS' (see TESTING.md, or set \$FASTCHESS)." >&2; exit 1; }
[[ -f "$BOOK" ]] || { echo "ERROR: opening book not found: '$BOOK'." >&2; exit 1; }

WORK="$(mktemp -d)"
# Clean up the temp build area, fastchess's tournament auto-save, and any
# leftover build worktrees when the script exits.
trap 'rm -rf "$WORK"; rm -f "$ROOT/config.json"; git worktree prune >/dev/null 2>&1 || true' EXIT

# Build a git ref (or the working tree) and echo the resulting binary path.
build_ref() {
    local ref="$1"
    if [[ -z "$ref" || "$ref" == "wd" || "$ref" == "." ]]; then
        cargo build --release >/dev/null 2>&1
        echo "$ROOT/target/release/mindless"
        return
    fi
    local tag; tag="$(echo "$ref" | tr -c 'A-Za-z0-9_.-' '_')"
    local wt="$WORK/wt-$tag" bin="$WORK/mindless-$tag"
    git worktree add --detach --force "$wt" "$ref" >/dev/null 2>&1
    ( cd "$wt" && cargo build --release >/dev/null 2>&1 )
    cp "$wt/target/release/mindless" "$bin"
    git worktree remove --force "$wt" >/dev/null 2>&1
    echo "$bin"
}

echo ">> Building NEW  ($NEW_REF) ..."
NEW_BIN="$(build_ref "$NEW_REF")"
if [[ "$NEW_REF" == "$BASE_REF" ]]; then
    echo ">> BASE shares NEW's source ($BASE_REF) — A-vs-A."
    BASE_BIN="$NEW_BIN"
else
    echo ">> Building BASE ($BASE_REF) ..."
    BASE_BIN="$(build_ref "$BASE_REF")"
fi

NEW_E="tc=$TC $NEW_LIMIT"
BASE_E="tc=$TC $BASE_LIMIT"
LOG="$WORK/sprt.log"

echo
echo ">> SPRT  new[$NEW_REF] vs base[$BASE_REF]"
echo "   TC=$TC  hash=${HASH}MB  concurrency=$CONCURRENCY  book=$(basename "$BOOK")"
echo "   bounds: elo0=$ELO0 elo1=$ELO1 alpha=$ALPHA beta=$BETA"
echo

set +e
# shellcheck disable=SC2086
"$FASTCHESS" \
    -engine cmd="$NEW_BIN" name=new $NEW_E option.Hash="$HASH" option.Threads=1 \
    -engine cmd="$BASE_BIN" name=base $BASE_E option.Hash="$HASH" option.Threads=1 \
    -each proto=uci \
    -openings file="$BOOK" format=epd order=random \
    -rounds "$ROUNDS" -games "$GAMES" -repeat \
    -concurrency "$CONCURRENCY" \
    -sprt elo0="$ELO0" elo1="$ELO1" alpha="$ALPHA" beta="$BETA" \
    -draw movenumber=40 movecount=8 score=10 \
    -resign movecount=3 score=400 \
    -ratinginterval 20 \
    2>&1 | tee "$LOG"
set -e

echo
echo "================= SPRT VERDICT ================="
grep -E "^Elo:"   "$LOG" | tail -1 || true
grep -E "^Games:" "$LOG" | tail -1 || true
grep -E "Ptnml"   "$LOG" | tail -1 || true
LLR_LINE="$(grep -E "LLR:" "$LOG" | tail -1 || true)"
echo "${LLR_LINE:-LLR: (not reported)}"

if [[ -n "$LLR_LINE" ]]; then
    LLR="$(echo "$LLR_LINE" | sed -E 's/.*LLR: *(-?[0-9.]+).*/\1/')"
    LO="$(echo  "$LLR_LINE" | sed -E 's/.*\((-?[0-9.]+), *(-?[0-9.]+)\).*/\1/')"
    HI="$(echo  "$LLR_LINE" | sed -E 's/.*\((-?[0-9.]+), *(-?[0-9.]+)\).*/\2/')"
    awk -v l="$LLR" -v lo="$LO" -v hi="$HI" 'BEGIN {
        if (l >= hi)      print "Verdict: PASS  — H1 accepted (new is >= elo1 Elo stronger)";
        else if (l <= lo) print "Verdict: FAIL  — H0 accepted (new is not elo1 Elo stronger)";
        else              print "Verdict: CONTINUE — inconclusive within the rounds budget";
    }'
else
    echo "Verdict: UNKNOWN (no LLR line found in output)"
fi
echo "================================================"
