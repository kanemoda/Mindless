//! Zobrist hashing keys for incremental position hashing.
//!
//! Every component of the position (piece-on-square, side to move, castling
//! rights, en-passant file) has a random 64-bit key. The position hash is the
//! XOR of all active keys, which lets [`Board`](crate::board::Board) update the
//! hash incrementally in `make`/`unmake` by XOR-ing only what changed.
//!
//! The keys are generated at compile time by a small `const` SplitMix64
//! generator with a fixed seed, so they are deterministic across builds and
//! cost nothing to initialize at runtime.

/// Fixed seed for the key generator. Any nonzero constant works; this one is
/// arbitrary but frozen so hashes are stable across builds.
const SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// One step of the SplitMix64 generator. Returns `(value, next_state)`.
const fn splitmix64(state: u64) -> (u64, u64) {
    let next_state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = next_state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31), next_state)
}

/// All Zobrist keys for a position.
pub struct Zobrist {
    /// `piece[piece_index][square]` — see [`Piece::index`](crate::types::Piece::index).
    pub piece: [[u64; 64]; 12],
    /// `castling[rights_bits]` — indexed by the 4-bit castling mask (`0..16`).
    pub castling: [u64; 16],
    /// `ep_file[file]` — keyed by the file (`0..8`) of the en-passant square.
    pub ep_file: [u64; 8],
    /// XOR-ed into the hash when it is Black to move.
    pub side: u64,
}

impl Zobrist {
    const fn new() -> Zobrist {
        let mut state = SEED;

        let mut piece = [[0u64; 64]; 12];
        let mut p = 0;
        while p < 12 {
            let mut s = 0;
            while s < 64 {
                let (value, next) = splitmix64(state);
                state = next;
                piece[p][s] = value;
                s += 1;
            }
            p += 1;
        }

        let mut castling = [0u64; 16];
        let mut c = 0;
        while c < 16 {
            let (value, next) = splitmix64(state);
            state = next;
            castling[c] = value;
            c += 1;
        }

        let mut ep_file = [0u64; 8];
        let mut e = 0;
        while e < 8 {
            let (value, next) = splitmix64(state);
            state = next;
            ep_file[e] = value;
            e += 1;
        }

        let (side, _) = splitmix64(state);

        Zobrist {
            piece,
            castling,
            ep_file,
            side,
        }
    }
}

/// The global, compile-time-generated set of Zobrist keys.
pub static ZOBRIST: Zobrist = Zobrist::new();
