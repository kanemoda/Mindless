/*
Independent reference inference for the eval-match check.

This loads a quantised `mindless` net and evaluates a FEN using *bullet's own*
`Chess768` feature mapping (via `SparseInputType::map_features` on a
`bulletformat::ChessBoard` parsed from the FEN) and the reference integer
inference from bullet's `examples/simple.rs`. It is deliberately independent of
the engine's `src/nnue.rs`: if `mindless eval` agrees with this on both
white- and black-to-move positions, the engine reproduces bullet's authoritative
convention (feature indices, perspective orientation, byte layout, arithmetic).

Usage:  refeval <net.bin> ["<FEN>"]   (FEN omitted => read FENs from stdin)
Output: "<FEN> | <cp>"  (stm-relative centipawns), matching `mindless eval`.
*/
use bullet_lib::game::formats::bulletformat::ChessBoard;
use bullet_lib::game::inputs::{Chess768, SparseInputType};

const HIDDEN_SIZE: usize = 128;
const SCALE: i32 = 400;
const QA: i16 = 255;
const QB: i16 = 64;

#[inline]
fn screlu(x: i16) -> i32 {
    let y = i32::from(x).clamp(0, i32::from(QA));
    y * y
}

/// Byte-for-byte the quantised format bullet outputs (from `examples/simple.rs`).
#[repr(C)]
pub struct Network {
    feature_weights: [Accumulator; 768],
    feature_bias: Accumulator,
    output_weights: [i16; 2 * HIDDEN_SIZE],
    output_bias: i16,
}

#[derive(Clone, Copy)]
#[repr(C, align(64))]
pub struct Accumulator {
    vals: [i16; HIDDEN_SIZE],
}

impl Network {
    fn evaluate(&self, us: &Accumulator, them: &Accumulator) -> i32 {
        let mut output = 0;
        for (&input, &weight) in us.vals.iter().zip(&self.output_weights[..HIDDEN_SIZE]) {
            output += screlu(input) * i32::from(weight);
        }
        for (&input, &weight) in them.vals.iter().zip(&self.output_weights[HIDDEN_SIZE..]) {
            output += screlu(input) * i32::from(weight);
        }
        output /= i32::from(QA);
        output += i32::from(self.output_bias);
        output *= SCALE;
        output /= i32::from(QA) * i32::from(QB);
        output
    }
}

fn load_net(path: &str) -> Box<Network> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read net {path}: {e}"));
    let want = std::mem::size_of::<Network>();
    assert!(bytes.len() >= want, "net {path} too small: {} < {want}", bytes.len());
    let mut net = Box::new(unsafe { std::mem::zeroed::<Network>() });
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), net.as_mut() as *mut Network as *mut u8, want);
    }
    net
}

fn eval_fen(net: &Network, fen: &str) {
    let fen = fen.trim();
    // ChessBoard::from_str expects "<FEN> | <score> | <result>"; the score and
    // result are irrelevant to the feature mapping, so supply placeholders.
    let record = format!("{fen} | 0 | 0.5");
    let board: ChessBoard = match record.parse() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("invalid FEN '{fen}': {e}");
            return;
        }
    };

    let mut us = net.feature_bias;
    let mut them = net.feature_bias;
    Chess768.map_features(&board, |stm, ntm| {
        let col = &net.feature_weights[stm];
        for (v, &w) in us.vals.iter_mut().zip(col.vals.iter()) {
            *v += w;
        }
        let col = &net.feature_weights[ntm];
        for (v, &w) in them.vals.iter_mut().zip(col.vals.iter()) {
            *v += w;
        }
    });

    println!("{fen} | {}", net.evaluate(&us, &them));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let net_path = args.get(1).expect("usage: refeval <net.bin> [\"<FEN>\"]");
    let net = load_net(net_path);

    if args.len() > 2 {
        eval_fen(&net, &args[2..].join(" "));
    } else {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let line = line.unwrap_or_default();
            if !line.trim().is_empty() {
                eval_fen(&net, &line);
            }
        }
    }
}
