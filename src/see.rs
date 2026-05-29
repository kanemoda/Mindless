//! Static Exchange Evaluation (SEE).
//!
//! SEE answers a single question about a move: *if this capture (or move to a
//! square) starts a sequence of captures and recaptures on the destination
//! square, and both sides always recapture with their least valuable attacker
//! and stop as soon as continuing would lose material, what is the net material
//! outcome for the side that moved first?*
//!
//! The result is in centipawns, from the mover's perspective: positive means the
//! exchange wins material, zero means it breaks even, negative means it loses
//! material. This is the standard tool for telling a "good" capture (a free or
//! even one) from a "bad" one (a sacrifice the opponent simply refutes), and it
//! is used both to order captures and to prune hopeless ones in the search.
//!
//! The implementation is the classic swap algorithm expressed recursively: at
//! each step the side to move either captures with its least valuable attacker
//! (and the opponent then gets the same choice on the now-occupied square) or
//! declines, whichever is better for it. Recapture attackers — including sliders
//! that only appear once a piece in front of them is removed ("x-ray"
//! attackers) — are found by recomputing the attacker set against the shrinking
//! occupancy, so doubled rooks, queen-behind-bishop batteries and similar are
//! handled correctly.

use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::eval::PIECE_VALUE;
use crate::moves::Move;
use crate::types::{Color, PieceType, Square};

/// Shorthand for a single-square bitboard.
#[inline]
fn bb(sq: Square) -> Bitboard {
    Bitboard::from_square(sq)
}

/// Piece values used by SEE, indexed by [`PieceType`]. The king is given a value
/// far larger than any other piece so it is only ever chosen as the very last
/// attacker; the "decline if losing" rule then guarantees a king is never
/// actually counted as captured.
const SEE_VALUE: [i32; 6] = PIECE_VALUE;

/// Static exchange evaluation of `mv`: the net material outcome (centipawns,
/// from the moving side's perspective) of the capture sequence it begins on its
/// destination square. Works for quiet moves too (the move "captures" nothing,
/// so the result is `<= 0`: zero if the destination is safe, negative if the
/// piece can be won there).
pub fn see(board: &Board, mv: Move) -> i32 {
    let to = mv.to();
    let from = mv.from();

    // Occupancy as it stands once the moving piece has left its origin (so any
    // slider behind it can join the fray). En passant also vacates the captured
    // pawn's square, which sits beside the destination.
    let mut occ = board.occupied() ^ bb(from);

    // Value of the piece standing on the destination (the first thing captured).
    let mut target_val = if mv.is_en_passant() {
        let cap_sq = Square::from_file_rank(to.file(), from.rank());
        occ ^= bb(cap_sq);
        SEE_VALUE[PieceType::Pawn.index()]
    } else {
        board
            .piece_on(to)
            .map_or(0, |p| SEE_VALUE[p.piece_type().index()])
    };

    // Value of the piece that ends up standing on the destination after our
    // move, i.e. the one the opponent would recapture. A promotion arrives as
    // the promoted piece and also banks the pawn-to-piece upgrade immediately.
    let moving = board
        .piece_on(from)
        .expect("see: from-square must hold the moving piece");
    let mut on_square_val = SEE_VALUE[moving.piece_type().index()];
    if let Some(promo) = mv.promotion() {
        let gain = SEE_VALUE[promo.index()] - SEE_VALUE[PieceType::Pawn.index()];
        on_square_val = SEE_VALUE[promo.index()];
        target_val += gain;
    }

    target_val - capture_value(board, to, board.side_to_move().flip(), occ, on_square_val)
}

/// True if the static exchange evaluation of `mv` is at least `threshold`
/// centipawns. Equivalent to `see(board, mv) >= threshold`; provided as the
/// natural form for pruning decisions ("is this capture at least equal?",
/// "is this move's material loss within the margin?").
#[inline]
pub fn see_ge(board: &Board, mv: Move, threshold: i32) -> bool {
    see(board, mv) >= threshold
}

/// Best material the `side` can extract by recapturing on `to`, given the
/// current `occ` and that the piece now standing on `to` is worth
/// `on_square_val`. The side may always decline (returning 0), so the result is
/// never negative — a player never makes a recapture that loses material.
fn capture_value(board: &Board, to: Square, side: Color, occ: Bitboard, on_square_val: i32) -> i32 {
    let Some((from_sq, attacker)) = least_valuable_attacker(board, to, side, occ) else {
        return 0; // no attacker: the sequence ends here.
    };

    // Capture the piece on the square (worth `on_square_val`); the opponent then
    // faces the same decision against our recapturing piece.
    let next_occ = occ ^ bb(from_sq);
    let gain = on_square_val
        - capture_value(
            board,
            to,
            side.flip(),
            next_occ,
            SEE_VALUE[attacker.index()],
        );

    // Decline the capture if it does not gain material.
    gain.max(0)
}

/// The least valuable piece of `side` that attacks `to` under occupancy `occ`,
/// or `None` if there is none. Sliders seen only through a now-empty square are
/// included because [`Board::attackers_to`] is evaluated against `occ`.
#[inline]
fn least_valuable_attacker(
    board: &Board,
    to: Square,
    side: Color,
    occ: Bitboard,
) -> Option<(Square, PieceType)> {
    let attackers = board.attackers_to(to, occ) & occ & board.color_bb(side);
    if attackers.is_empty() {
        return None;
    }
    // Piece kinds are ordered pawn..king by ascending value, so the first kind
    // with an attacker present is the least valuable.
    for pt in PieceType::ALL {
        let bb = attackers & board.pieces(pt);
        if bb.any() {
            return Some((bb.lsb(), pt));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::see;
    use crate::board::Board;
    use crate::movegen::legal_moves;
    use crate::moves::Move;

    /// Find the legal move on `fen` whose UCI string is `uci`, then run SEE.
    fn see_uci(fen: &str, uci: &str) -> i32 {
        let board = Board::from_fen(fen).expect("valid FEN");
        let mv = legal_moves(&board)
            .as_slice()
            .iter()
            .copied()
            .find(|m: &Move| m.to_uci() == uci)
            .unwrap_or_else(|| panic!("no legal move {uci} in {fen}"));
        see(&board, mv)
    }

    #[test]
    fn wins_a_free_pawn() {
        // Rook captures an undefended pawn: +1 pawn.
        assert_eq!(see_uci("4k3/8/8/4p3/8/8/8/4R1K1 w - - 0 1", "e1e5"), 100);
    }

    #[test]
    fn wins_a_free_rook() {
        // Queen captures an undefended rook (the king is on g8, too far to help).
        assert_eq!(see_uci("3r2k1/8/8/8/8/8/8/3QK3 w - - 0 1", "d1d8"), 500);
    }

    #[test]
    fn rook_takes_pawn_defended_by_pawn() {
        // Rxe5 wins a pawn but loses the rook to ...dxe5: 100 - 500 = -400.
        assert_eq!(see_uci("4k3/8/3p4/4p3/8/8/8/4R1K1 w - - 0 1", "e1e5"), -400);
    }

    #[test]
    fn queen_takes_pawn_defended_by_pawn() {
        // A worse sacrifice: 100 - 900 = -800.
        assert_eq!(see_uci("4k3/8/3p4/4p3/8/8/8/4Q1K1 w - - 0 1", "e1e5"), -800);
    }

    #[test]
    fn equal_pawn_trade() {
        // Pawn takes pawn, recaptured by a pawn: dead even.
        assert_eq!(see_uci("4k3/8/3p4/4p3/3P4/8/8/4K3 w - - 0 1", "d4e5"), 0);
    }

    #[test]
    fn xray_battery_of_doubled_rooks() {
        // Doubled rooks behind each other on the e-file take a pawn defended by a
        // single pawn: Rxe5 dxe5 Rxe5. Net 100 - 500 + 100 = -300, and the rear
        // rook is only an attacker once the front one has moved (x-ray).
        assert_eq!(
            see_uci("4k3/8/3p4/4p3/8/8/4R3/4R1K1 w - - 0 1", "e2e5"),
            -300
        );
    }

    #[test]
    fn quiet_move_to_safe_square_is_zero() {
        // Moving a rook to an empty, unattacked square exchanges nothing.
        assert_eq!(see_uci("4k3/8/8/8/8/8/8/4R1K1 w - - 0 1", "e1e4"), 0);
    }

    #[test]
    fn quiet_move_into_attack_loses_the_piece() {
        // Rook steps onto a square guarded by a pawn and nothing defends it: the
        // rook is simply lost. 0 - 500 = -500.
        assert_eq!(see_uci("4k3/8/8/5p2/8/8/8/4R1K1 w - - 0 1", "e1e4"), -500);
    }

    #[test]
    fn pawn_captures_queen() {
        // Pawn takes a queen defended by the king (on c5): 900 - 100 = +800.
        assert_eq!(see_uci("8/8/8/2k5/3q4/4P3/8/4K3 w - - 0 1", "e3d4"), 800);
    }
}
