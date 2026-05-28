//! Zobrist-keyed transposition table.
//!
//! The table caches the result of searching a position so that transpositions
//! (the same position reached by different move orders) and re-searches across
//! iterative-deepening iterations can reuse prior work. Each entry records the
//! best move, the score, the search depth it was obtained at, and a *bound*
//! describing whether the score is exact or only an alpha/beta bound.
//!
//! Storage is lock-free: each slot is two [`AtomicU64`]s, and the key is stored
//! XOR-ed with the data so a torn read (only possible under future multi-thread
//! search) is detected and treated as a miss. With the current single search
//! thread there are no races; this design simply keeps the table `Send + Sync`
//! and ready for Lazy SMP later.

use crate::moves::Move;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

/// The kind of score stored: exact, or a lower/upper bound from a cutoff.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Bound {
    /// Empty slot / no information.
    None = 0,
    /// Exact score (a PV node).
    Exact = 1,
    /// Lower bound (a fail-high / beta cutoff).
    Lower = 2,
    /// Upper bound (a fail-low / no move beat alpha).
    Upper = 3,
}

impl Bound {
    #[inline]
    fn from_bits(bits: u64) -> Bound {
        match bits & 0b11 {
            1 => Bound::Exact,
            2 => Bound::Lower,
            3 => Bound::Upper,
            _ => Bound::None,
        }
    }
}

/// A decoded transposition-table entry.
pub struct TtEntry {
    /// Best move found in this position (may be [`Move::NULL`]).
    pub mv: Move,
    /// Stored score (node-relative; the search adjusts mate scores by ply).
    pub score: i32,
    /// Depth the score was searched to.
    pub depth: i32,
    /// Bound type of the score.
    pub bound: Bound,
}

/// One table slot: `key ^ data` and `data`, both atomic.
struct Slot {
    key: AtomicU64,
    data: AtomicU64,
}

/// The transposition table.
pub struct Tt {
    slots: Box<[Slot]>,
    mask: usize,
    generation: AtomicU8,
}

#[inline]
fn pack(mv: u16, score: i16, depth: u8, bound: Bound, generation: u8) -> u64 {
    (mv as u64)
        | ((score as u16 as u64) << 16)
        | ((depth as u64) << 32)
        | ((bound as u64) << 40)
        | ((generation as u64) << 42)
}

#[inline]
fn unpack_move(data: u64) -> Move {
    Move::from_bits((data & 0xFFFF) as u16)
}
#[inline]
fn unpack_score(data: u64) -> i16 {
    ((data >> 16) & 0xFFFF) as u16 as i16
}
#[inline]
fn unpack_depth(data: u64) -> u8 {
    ((data >> 32) & 0xFF) as u8
}
#[inline]
fn unpack_gen(data: u64) -> u8 {
    ((data >> 42) & 0xFF) as u8
}

/// Largest power of two not exceeding `n` (for `n >= 1`).
#[inline]
fn prev_power_of_two(n: usize) -> usize {
    1usize << (usize::BITS - 1 - n.max(1).leading_zeros())
}

impl Tt {
    /// Create a table sized to about `mb` mebibytes (rounded down to a power of
    /// two number of slots).
    pub fn new(mb: usize) -> Tt {
        let bytes = mb.max(1) * 1024 * 1024;
        let count = prev_power_of_two(bytes / std::mem::size_of::<Slot>());
        let slots = (0..count)
            .map(|_| Slot {
                key: AtomicU64::new(0),
                data: AtomicU64::new(0),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Tt {
            slots,
            mask: count - 1,
            generation: AtomicU8::new(0),
        }
    }

    /// Number of slots.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Always false (the table always has at least one slot); present so clippy
    /// is satisfied alongside [`len`](Self::len).
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Empty every slot and reset the generation counter.
    pub fn clear(&self) {
        for slot in self.slots.iter() {
            slot.key.store(0, Ordering::Relaxed);
            slot.data.store(0, Ordering::Relaxed);
        }
        self.generation.store(0, Ordering::Relaxed);
    }

    /// Advance the generation counter; called once at the start of each search
    /// so entries from previous searches can be preferentially overwritten.
    pub fn new_search(&self) {
        let next = self.generation.load(Ordering::Relaxed).wrapping_add(1);
        self.generation.store(next, Ordering::Relaxed);
    }

    /// Look up `key`. Returns `None` on a miss.
    pub fn probe(&self, key: u64) -> Option<TtEntry> {
        let slot = &self.slots[(key as usize) & self.mask];
        let data = slot.data.load(Ordering::Relaxed);
        let stored = slot.key.load(Ordering::Relaxed);
        if stored ^ data != key {
            return None;
        }
        let bound = Bound::from_bits(data >> 40);
        if bound == Bound::None {
            return None;
        }
        Some(TtEntry {
            mv: unpack_move(data),
            score: unpack_score(data) as i32,
            depth: unpack_depth(data) as i32,
            bound,
        })
    }

    /// Store an entry, using a depth-preferred replacement scheme with aging.
    pub fn store(&self, key: u64, mv: Move, score: i32, depth: i32, bound: Bound) {
        let slot = &self.slots[(key as usize) & self.mask];
        let old_data = slot.data.load(Ordering::Relaxed);
        let old_key = slot.key.load(Ordering::Relaxed);
        let cur_gen = self.generation.load(Ordering::Relaxed);

        let same_position = (old_key ^ old_data) == key;
        let old_bound = Bound::from_bits(old_data >> 40);
        let old_depth = unpack_depth(old_data) as i32;
        let old_gen = unpack_gen(old_data);

        // Replace empty slots, the same position, aged entries, or anything not
        // deeper than this result (exact scores get a small bonus).
        let prefer_new = depth + if bound == Bound::Exact { 2 } else { 0 } >= old_depth;
        let replace = old_bound == Bound::None || same_position || old_gen != cur_gen || prefer_new;
        if !replace {
            return;
        }

        // Preserve a useful move if this store has none for the same position.
        let mv_bits = if mv.is_null() && same_position {
            unpack_move(old_data).to_bits()
        } else {
            mv.to_bits()
        };

        let depth_byte = depth.clamp(0, 255) as u8;
        let data = pack(mv_bits, score as i16, depth_byte, bound, cur_gen);
        slot.data.store(data, Ordering::Relaxed);
        slot.key.store(key ^ data, Ordering::Relaxed);
    }

    /// Approximate fill level in per-mille (0–1000), sampling the first slots.
    pub fn hashfull(&self) -> usize {
        let cur = self.generation.load(Ordering::Relaxed);
        let sample = self.slots.len().min(1000);
        let mut used = 0;
        for slot in self.slots.iter().take(sample) {
            let data = slot.data.load(Ordering::Relaxed);
            if Bound::from_bits(data >> 40) != Bound::None && unpack_gen(data) == cur {
                used += 1;
            }
        }
        used * 1000 / sample.max(1)
    }
}
