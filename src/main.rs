//! The Mindless engine executable.
//!
//! With no arguments it speaks UCI on stdin/stdout (the normal mode for a chess
//! GUI). It also offers two command-line helpers:
//!
//! * `mindless perft <depth> [FEN]` — perft divide for a position.
//! * `mindless bench [depth]` — run the perft reference suite and report speed.

use mindless::board::Board;
use mindless::perft::{perft, perft_divide, SUITE};
use mindless::uci;
use std::env;
use std::time::Instant;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("bench") => {
            let depth_override = args.get(2).and_then(|s| s.parse::<u32>().ok());
            run_bench(depth_override);
        }
        Some("perft") => run_perft_cli(&args[2..]),
        Some("--version") | Some("-V") => println!("Mindless {VERSION}"),
        Some("--help") | Some("-h") => print_help(),
        _ => uci::run(),
    }
}

fn print_help() {
    println!("Mindless {VERSION} — a UCI chess engine");
    println!();
    println!("USAGE:");
    println!("  mindless                     Start the UCI engine (default)");
    println!("  mindless perft <depth> [FEN] Perft divide for a position");
    println!("  mindless bench [depth]       Run the perft reference suite");
    println!("  mindless --version           Print the version");
}

/// `mindless perft <depth> [FEN]`
fn run_perft_cli(args: &[String]) {
    let Some(depth) = args.first().and_then(|s| s.parse::<u32>().ok()) else {
        eprintln!("usage: mindless perft <depth> [FEN]");
        return;
    };

    let mut board = if args.len() > 1 {
        match Board::from_fen(&args[1..].join(" ")) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("invalid FEN: {e}");
                return;
            }
        }
    } else {
        Board::startpos()
    };

    // Build slider tables before timing so they are not counted.
    mindless::magic::init();

    let start = Instant::now();
    let (moves, total) = perft_divide(&mut board, depth);
    let elapsed = start.elapsed();

    for (mv, count) in &moves {
        println!("{}: {count}", mv.to_uci());
    }
    println!();
    println!("Nodes searched: {total}");

    let secs = elapsed.as_secs_f64();
    let mnps = if secs > 0.0 {
        total as f64 / secs / 1e6
    } else {
        0.0
    };
    println!("Time: {secs:.3}s   Speed: {mnps:.2} Mnps");
}

/// `mindless bench [depth]` — run every reference position and report results.
fn run_bench(depth_override: Option<u32>) {
    mindless::magic::init();

    println!("Mindless {VERSION} — perft benchmark\n");
    println!(
        "{:<11} {:>5} {:>15} {:>10} {:>10}  result",
        "position", "depth", "nodes", "time(s)", "Mnps"
    );
    println!("{}", "-".repeat(66));

    let mut grand_nodes = 0u64;
    let mut grand_secs = 0.0f64;
    let mut all_ok = true;

    for pos in SUITE {
        let depth =
            depth_override.unwrap_or_else(|| pos.counts.last().map(|&(d, _)| d).unwrap_or(1));
        let expected = pos
            .counts
            .iter()
            .find(|&&(d, _)| d == depth)
            .map(|&(_, c)| c);

        let mut board = Board::from_fen(pos.fen).expect("suite FEN is valid");
        let start = Instant::now();
        let nodes = perft(&mut board, depth);
        let secs = start.elapsed().as_secs_f64();

        let mnps = nodes as f64 / secs.max(1e-9) / 1e6;
        let result = match expected {
            Some(e) if e == nodes => "OK",
            Some(e) => {
                all_ok = false;
                eprintln!("  MISMATCH {}: got {nodes}, expected {e}", pos.name);
                "FAIL"
            }
            None => "n/a",
        };

        println!(
            "{:<11} {:>5} {:>15} {:>10.3} {:>10.2}  {result}",
            pos.name, depth, nodes, secs, mnps
        );

        grand_nodes += nodes;
        grand_secs += secs;
    }

    println!("{}", "-".repeat(66));
    let mnps = grand_nodes as f64 / grand_secs.max(1e-9) / 1e6;
    println!(
        "{:<11} {:>5} {:>15} {:>10.3} {:>10.2}",
        "TOTAL", "", grand_nodes, grand_secs, mnps
    );
    println!(
        "\nOverall: {}",
        if all_ok {
            "all positions correct"
        } else {
            "FAILURES PRESENT"
        }
    );
}
