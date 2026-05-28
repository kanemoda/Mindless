//! Magic bitboards for sliding pieces (rook, bishop, queen).
//!
//! A "magic" is a multiplier that hashes the relevant occupancy bits of a
//! square into a dense index, turning sliding attack lookups into a single
//! multiply, shift and array read. The magics and attack tables are built once,
//! lazily, on first use:
//!
//! * relevant-occupancy masks are derived geometrically per square;
//! * magics are found by trying sparse random candidates (a fixed seed keeps
//!   this deterministic) until one maps every occupancy subset without an
//!   index collision;
//! * a shared attack table is filled using a slow reference ray-walk.
//!
//! Attack lookups (`rook_attacks`, `bishop_attacks`, `queen_attacks`) are the
//! public entry points and are safe to call from any thread.

use crate::bitboard::Bitboard;
use crate::types::Square;
use std::sync::OnceLock;

/// Fixed seed for the magic search, so builds are reproducible.
const SEED: u64 = 0x0123_4567_89AB_CDEF;

/// A tiny xorshift64 generator used only while building the tables.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A candidate magic with few set bits, which tend to work well.
    fn sparse(&mut self) -> u64 {
        self.next_u64() & self.next_u64() & self.next_u64()
    }
}

/// Per-square magic data indexing into a shared attack table.
#[derive(Clone, Copy)]
struct Magic {
    mask: u64,
    magic: u64,
    shift: u32,
    offset: usize,
}

impl Magic {
    const EMPTY: Magic = Magic {
        mask: 0,
        magic: 0,
        shift: 0,
        offset: 0,
    };

    #[inline]
    fn index(&self, occ: u64) -> usize {
        self.offset + (((occ & self.mask).wrapping_mul(self.magic)) >> self.shift) as usize
    }
}

/// Magics plus the attack table for one slider kind.
struct Slider {
    magics: [Magic; 64],
    table: Vec<u64>,
}

/// Both slider kinds, built together.
struct Sliders {
    rook: Slider,
    bishop: Slider,
}

static SLIDERS: OnceLock<Sliders> = OnceLock::new();

#[inline]
fn sliders() -> &'static Sliders {
    SLIDERS.get_or_init(Sliders::build)
}

/// Rook directions: N, S, E, W.
const ROOK_DIRS: [(i32, i32); 4] = [(0, 1), (0, -1), (1, 0), (-1, 0)];
/// Bishop directions: the four diagonals.
const BISHOP_DIRS: [(i32, i32); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

#[inline]
fn dirs(rook: bool) -> [(i32, i32); 4] {
    if rook {
        ROOK_DIRS
    } else {
        BISHOP_DIRS
    }
}

/// Relevant-occupancy mask for `sq`: the squares whose contents can block the
/// slider, excluding the board-edge squares (a blocker on the edge never
/// changes the reachable set).
fn relevant_mask(sq: usize, rook: bool) -> u64 {
    let f = (sq % 8) as i32;
    let r = (sq / 8) as i32;
    let mut mask = 0u64;
    for (df, dr) in dirs(rook) {
        let mut nf = f + df;
        let mut nr = r + dr;
        // Include a square only while there is a further square on the board.
        while (nf + df) >= 0 && (nf + df) < 8 && (nr + dr) >= 0 && (nr + dr) < 8 {
            mask |= 1u64 << (nr * 8 + nf);
            nf += df;
            nr += dr;
        }
    }
    mask
}

/// Reference slider attacks from `sq` for a given occupancy, by walking rays
/// until a blocker (included) or the edge.
fn ray_attacks(sq: usize, occ: u64, rook: bool) -> u64 {
    let f = (sq % 8) as i32;
    let r = (sq / 8) as i32;
    let mut attacks = 0u64;
    for (df, dr) in dirs(rook) {
        let mut nf = f + df;
        let mut nr = r + dr;
        while (0..8).contains(&nf) && (0..8).contains(&nr) {
            let s = (nr * 8 + nf) as u64;
            attacks |= 1u64 << s;
            if occ & (1u64 << s) != 0 {
                break;
            }
            nf += df;
            nr += dr;
        }
    }
    attacks
}

/// Enumerate every subset of `mask` via the carry-rippler trick.
fn occupancy_subsets(mask: u64) -> Vec<u64> {
    let size = 1usize << mask.count_ones();
    let mut subsets = Vec::with_capacity(size);
    let mut subset = 0u64;
    loop {
        subsets.push(subset);
        subset = subset.wrapping_sub(mask) & mask;
        if subset == 0 {
            break;
        }
    }
    debug_assert_eq!(subsets.len(), size);
    subsets
}

/// Search for a magic that maps each occupancy subset to its attack set with no
/// conflicting collisions.
fn find_magic(mask: u64, occs: &[u64], refs: &[u64], shift: u32, rng: &mut Rng) -> u64 {
    let size = occs.len();
    let mut used = vec![0u64; size];
    let mut epoch = vec![0u32; size];
    let mut current = 0u32;

    loop {
        let magic = rng.sparse();
        // Heuristic: good magics scatter the mask's high byte well.
        if (mask.wrapping_mul(magic) & 0xFF00_0000_0000_0000).count_ones() < 6 {
            continue;
        }

        current += 1;
        let mut ok = true;
        for i in 0..size {
            let idx = ((occs[i].wrapping_mul(magic)) >> shift) as usize;
            if epoch[idx] != current {
                epoch[idx] = current;
                used[idx] = refs[i];
            } else if used[idx] != refs[i] {
                ok = false;
                break;
            }
        }
        if ok {
            return magic;
        }
    }
}

impl Slider {
    fn build(rook: bool) -> Slider {
        let mut rng = Rng::new(SEED);
        let mut magics = [Magic::EMPTY; 64];
        let mut table: Vec<u64> = Vec::new();

        for (sq, slot) in magics.iter_mut().enumerate() {
            let mask = relevant_mask(sq, rook);
            let bits = mask.count_ones();
            let shift = 64 - bits;

            let occs = occupancy_subsets(mask);
            let refs: Vec<u64> = occs.iter().map(|&o| ray_attacks(sq, o, rook)).collect();

            let magic = find_magic(mask, &occs, &refs, shift, &mut rng);

            let offset = table.len();
            table.resize(offset + occs.len(), 0);
            for (i, &occ) in occs.iter().enumerate() {
                let idx = ((occ.wrapping_mul(magic)) >> shift) as usize;
                table[offset + idx] = refs[i];
            }

            *slot = Magic {
                mask,
                magic,
                shift,
                offset,
            };
        }

        Slider { magics, table }
    }
}

impl Sliders {
    fn build() -> Sliders {
        Sliders {
            rook: Slider::build(true),
            bishop: Slider::build(false),
        }
    }
}

/// Rook attacks from `sq` given the full board occupancy.
#[inline]
pub fn rook_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    let s = sliders();
    let m = &s.rook.magics[sq.index()];
    Bitboard(s.rook.table[m.index(occupied.0)])
}

/// Bishop attacks from `sq` given the full board occupancy.
#[inline]
pub fn bishop_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    let s = sliders();
    let m = &s.bishop.magics[sq.index()];
    Bitboard(s.bishop.table[m.index(occupied.0)])
}

/// Queen attacks from `sq` given the full board occupancy.
#[inline]
pub fn queen_attacks(sq: Square, occupied: Bitboard) -> Bitboard {
    rook_attacks(sq, occupied) | bishop_attacks(sq, occupied)
}

/// Force the slider tables to build now. Optional; lookups build on demand.
/// Useful to move the one-time cost out of the timed search.
pub fn init() {
    let _ = sliders();
}
