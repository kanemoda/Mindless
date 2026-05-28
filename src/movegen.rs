//! Fully legal move generation.
//!
//! Rather than generate pseudo-legal moves and filter them, this generator
//! computes the king's checkers, the squares that resolve a check, and the set
//! of pinned pieces up front, then emits only legal moves directly:
//!
//! * **King moves** never land on a square the enemy attacks (the enemy attack
//!   set is computed with our king removed, so the king cannot retreat along a
//!   slider's beam).
//! * **Double check** restricts movement to the king.
//! * **Single check** restricts every other piece to squares that block the
//!   checker or capture it.
//! * **Pinned pieces** are restricted to the line through the king and the
//!   pinning slider.
//! * **En passant** — the one case the above does not fully cover — is verified
//!   by testing king safety on the exact post-capture occupancy.

use crate::attacks::{BETWEEN, KING_ATTACKS, KNIGHT_ATTACKS, LINE, PAWN_ATTACKS};
use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::magic::{bishop_attacks, rook_attacks};
use crate::moves::{flag, Move, MoveList};
use crate::types::{CastlingRights, Color, PieceType, Square};

// Squares involved in castling (must be empty, and for the king's path, safe).
const WHITE_KS_PATH: Bitboard = Bitboard(0x60); // f1, g1
const WHITE_QS_EMPTY: Bitboard = Bitboard(0x0E); // b1, c1, d1
const WHITE_QS_PATH: Bitboard = Bitboard(0x0C); // c1, d1
const BLACK_KS_PATH: Bitboard = Bitboard(0x6000_0000_0000_0000); // f8, g8
const BLACK_QS_EMPTY: Bitboard = Bitboard(0x0E00_0000_0000_0000); // b8, c8, d8
const BLACK_QS_PATH: Bitboard = Bitboard(0x0C00_0000_0000_0000); // c8, d8

/// Generate all legal moves for the side to move into `list`.
pub fn generate_legal(board: &Board, list: &mut MoveList) {
    generate(board, list, true);
}

/// Generate only "noisy" legal moves — captures, en passant and promotions —
/// for quiescence search. Quiet moves (plain pushes, double pushes, castling
/// and quiet piece moves) are omitted.
pub fn generate_noisy(board: &Board, list: &mut MoveList) {
    generate(board, list, false);
}

/// Core legal-move generator. When `quiets` is true it produces every legal
/// move (the perft-verified behaviour); when false it produces only captures,
/// en passant and promotions.
fn generate(board: &Board, list: &mut MoveList, quiets: bool) {
    let us = board.side_to_move();
    let them = us.flip();
    let occ = board.occupied();
    let us_bb = board.color_bb(us);
    let them_bb = board.color_bb(them);
    let king_sq = board.king_square(us);

    // Enemy attacks with our king removed, so the king cannot step along the
    // continuation of a checking slider's ray.
    let enemy_attacks = attacks_by(board, them, occ ^ Bitboard::from_square(king_sq));

    // King moves: onto neither our own pieces nor attacked squares.
    let king_targets = KING_ATTACKS[king_sq.index()] & !us_bb & !enemy_attacks;
    add_piece_moves(king_sq, king_targets, them_bb, quiets, list);

    let checkers = board.attackers_to(king_sq, occ) & them_bb;
    let num_checkers = checkers.count();

    // In double check only the king may move.
    if num_checkers >= 2 {
        return;
    }

    // Squares that resolve a single check (block the ray or capture the
    // checker); unrestricted otherwise.
    let check_mask = if num_checkers == 1 {
        let checker = checkers.lsb();
        BETWEEN[king_sq.index()][checker.index()] | Bitboard::from_square(checker)
    } else {
        Bitboard::FULL
    };

    let pinned = compute_pinned(board, us, them, king_sq, occ);

    // Castling is a quiet move and only possible when not in check.
    if quiets && num_checkers == 0 {
        generate_castling(board, us, occ, enemy_attacks, list);
    }

    generate_pawn_moves(
        board, us, them, occ, them_bb, check_mask, pinned, king_sq, quiets, list,
    );

    // Knights: a pinned knight has no legal moves (it cannot stay on the ray).
    for from in board.pieces_colored(us, PieceType::Knight) & !pinned {
        let targets = KNIGHT_ATTACKS[from.index()] & !us_bb & check_mask;
        add_piece_moves(from, targets, them_bb, quiets, list);
    }

    // Bishops and queens — diagonal rays.
    let diagonal =
        board.pieces_colored(us, PieceType::Bishop) | board.pieces_colored(us, PieceType::Queen);
    for from in diagonal {
        let mut targets = bishop_attacks(from, occ) & !us_bb & check_mask;
        if pinned.contains(from) {
            targets &= LINE[king_sq.index()][from.index()];
        }
        add_piece_moves(from, targets, them_bb, quiets, list);
    }

    // Rooks and queens — orthogonal rays.
    let orthogonal =
        board.pieces_colored(us, PieceType::Rook) | board.pieces_colored(us, PieceType::Queen);
    for from in orthogonal {
        let mut targets = rook_attacks(from, occ) & !us_bb & check_mask;
        if pinned.contains(from) {
            targets &= LINE[king_sq.index()][from.index()];
        }
        add_piece_moves(from, targets, them_bb, quiets, list);
    }
}

/// Convenience wrapper returning a fresh [`MoveList`] of all legal moves.
pub fn legal_moves(board: &Board) -> MoveList {
    let mut list = MoveList::new();
    generate_legal(board, &mut list);
    list
}

/// Emit moves for a single non-pawn piece. Captures are always emitted; quiet
/// (non-capturing) moves only when `quiets` is true.
#[inline]
fn add_piece_moves(
    from: Square,
    targets: Bitboard,
    them_bb: Bitboard,
    quiets: bool,
    list: &mut MoveList,
) {
    for to in targets & them_bb {
        list.push(Move::new(from, to, flag::CAPTURE));
    }
    if quiets {
        for to in targets & !them_bb {
            list.push(Move::new(from, to, flag::QUIET));
        }
    }
}

/// All squares attacked by `color` given occupancy `occ`.
fn attacks_by(board: &Board, color: Color, occ: Bitboard) -> Bitboard {
    let mut attacks = pawn_attacks_set(board.pieces_colored(color, PieceType::Pawn), color);

    for sq in board.pieces_colored(color, PieceType::Knight) {
        attacks |= KNIGHT_ATTACKS[sq.index()];
    }

    let diagonal = board.pieces_colored(color, PieceType::Bishop)
        | board.pieces_colored(color, PieceType::Queen);
    for sq in diagonal {
        attacks |= bishop_attacks(sq, occ);
    }

    let orthogonal = board.pieces_colored(color, PieceType::Rook)
        | board.pieces_colored(color, PieceType::Queen);
    for sq in orthogonal {
        attacks |= rook_attacks(sq, occ);
    }

    attacks |= KING_ATTACKS[board.king_square(color).index()];
    attacks
}

/// The squares attacked by a set of pawns of the given color.
#[inline]
fn pawn_attacks_set(pawns: Bitboard, color: Color) -> Bitboard {
    match color {
        Color::White => pawns.north_west() | pawns.north_east(),
        Color::Black => pawns.south_west() | pawns.south_east(),
    }
}

/// Bitboard of our pieces that are pinned to the king by an enemy slider.
fn compute_pinned(
    board: &Board,
    us: Color,
    them: Color,
    king_sq: Square,
    occ: Bitboard,
) -> Bitboard {
    let us_bb = board.color_bb(us);
    let rook_like =
        board.pieces_colored(them, PieceType::Rook) | board.pieces_colored(them, PieceType::Queen);
    let bishop_like = board.pieces_colored(them, PieceType::Bishop)
        | board.pieces_colored(them, PieceType::Queen);

    // Enemy sliders that would hit the king on an empty board are the potential
    // pinners; a pin exists when exactly one of our pieces sits between them.
    let snipers = (rook_attacks(king_sq, Bitboard::EMPTY) & rook_like)
        | (bishop_attacks(king_sq, Bitboard::EMPTY) & bishop_like);

    let mut pinned = Bitboard::EMPTY;
    for sniper in snipers {
        let blockers = BETWEEN[king_sq.index()][sniper.index()] & occ;
        if blockers.count() == 1 && (blockers & us_bb).any() {
            pinned |= blockers;
        }
    }
    pinned
}

/// Generate the (up to two) castling moves available to `us`.
fn generate_castling(
    board: &Board,
    us: Color,
    occ: Bitboard,
    enemy_attacks: Bitboard,
    list: &mut MoveList,
) {
    let rights = board.castling_rights();
    match us {
        Color::White => {
            if rights.has(CastlingRights::WHITE_KING)
                && (occ & WHITE_KS_PATH).is_empty()
                && (enemy_attacks & WHITE_KS_PATH).is_empty()
            {
                list.push(Move::new(Square::E1, Square::G1, flag::KING_CASTLE));
            }
            if rights.has(CastlingRights::WHITE_QUEEN)
                && (occ & WHITE_QS_EMPTY).is_empty()
                && (enemy_attacks & WHITE_QS_PATH).is_empty()
            {
                list.push(Move::new(Square::E1, Square::C1, flag::QUEEN_CASTLE));
            }
        }
        Color::Black => {
            if rights.has(CastlingRights::BLACK_KING)
                && (occ & BLACK_KS_PATH).is_empty()
                && (enemy_attacks & BLACK_KS_PATH).is_empty()
            {
                list.push(Move::new(Square::E8, Square::G8, flag::KING_CASTLE));
            }
            if rights.has(CastlingRights::BLACK_QUEEN)
                && (occ & BLACK_QS_EMPTY).is_empty()
                && (enemy_attacks & BLACK_QS_PATH).is_empty()
            {
                list.push(Move::new(Square::E8, Square::C8, flag::QUEEN_CASTLE));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn generate_pawn_moves(
    board: &Board,
    us: Color,
    them: Color,
    occ: Bitboard,
    them_bb: Bitboard,
    check_mask: Bitboard,
    pinned: Bitboard,
    king_sq: Square,
    quiets: bool,
    list: &mut MoveList,
) {
    let empty = !occ;
    let all_pawns = board.pieces_colored(us, PieceType::Pawn);

    // Unpinned pawns move freely (ray = the whole board).
    add_pawn_moves(
        all_pawns & !pinned,
        Bitboard::FULL,
        us,
        empty,
        them_bb,
        check_mask,
        quiets,
        list,
    );

    // Pinned pawns are restricted to the line through the king.
    for from in all_pawns & pinned {
        let ray = LINE[king_sq.index()][from.index()];
        add_pawn_moves(
            Bitboard::from_square(from),
            ray,
            us,
            empty,
            them_bb,
            check_mask,
            quiets,
            list,
        );
    }

    if let Some(ep) = board.en_passant() {
        generate_en_passant(board, us, them, king_sq, occ, ep, list);
    }
}

/// Generate pushes and diagonal captures (with promotions) for a set of pawns,
/// restricting destinations to `ray` (used for pin masking) and `check_mask`.
/// Captures and promotions are always emitted; quiet pushes only when `quiets`.
#[allow(clippy::too_many_arguments)]
fn add_pawn_moves(
    pawns: Bitboard,
    ray: Bitboard,
    us: Color,
    empty: Bitboard,
    them_bb: Bitboard,
    check_mask: Bitboard,
    quiets: bool,
    list: &mut MoveList,
) {
    match us {
        Color::White => {
            let single = pawns.north() & empty;
            let single_t = single & check_mask & ray;
            emit_pushes(single_t, Bitboard::RANK_8, 8, quiets, list);
            if quiets {
                let double = (single & Bitboard::RANK_3).north() & empty & check_mask & ray;
                emit_doubles(double, 16, list);
            }

            let left = pawns.north_west() & them_bb & check_mask & ray;
            let right = pawns.north_east() & them_bb & check_mask & ray;
            emit_captures(left, Bitboard::RANK_8, 7, list);
            emit_captures(right, Bitboard::RANK_8, 9, list);
        }
        Color::Black => {
            let single = pawns.south() & empty;
            let single_t = single & check_mask & ray;
            emit_pushes(single_t, Bitboard::RANK_1, -8, quiets, list);
            if quiets {
                let double = (single & Bitboard::RANK_6).south() & empty & check_mask & ray;
                emit_doubles(double, -16, list);
            }

            let left = pawns.south_west() & them_bb & check_mask & ray;
            let right = pawns.south_east() & them_bb & check_mask & ray;
            emit_captures(left, Bitboard::RANK_1, -9, list);
            emit_captures(right, Bitboard::RANK_1, -7, list);
        }
    }
}

#[inline]
fn origin(to: Square, delta: i32) -> Square {
    Square::new((to.index() as i32 - delta) as u8)
}

/// Emit single pushes. Promotions (on the last rank) are always emitted; plain
/// quiet pushes only when `quiets` is true.
fn emit_pushes(
    targets: Bitboard,
    promo_rank: Bitboard,
    delta: i32,
    quiets: bool,
    list: &mut MoveList,
) {
    if quiets {
        for to in targets & !promo_rank {
            list.push(Move::new(origin(to, delta), to, flag::QUIET));
        }
    }
    for to in targets & promo_rank {
        emit_promotions(origin(to, delta), to, false, list);
    }
}

/// Emit diagonal captures; targets on the promotion rank fan out to four
/// promotion-captures.
fn emit_captures(targets: Bitboard, promo_rank: Bitboard, delta: i32, list: &mut MoveList) {
    for to in targets & !promo_rank {
        list.push(Move::new(origin(to, delta), to, flag::CAPTURE));
    }
    for to in targets & promo_rank {
        emit_promotions(origin(to, delta), to, true, list);
    }
}

/// Emit double pawn pushes.
fn emit_doubles(targets: Bitboard, delta: i32, list: &mut MoveList) {
    for to in targets {
        list.push(Move::new(origin(to, delta), to, flag::DOUBLE_PAWN));
    }
}

/// Emit the four promotion moves (knight, bishop, rook, queen) for one pawn.
#[inline]
fn emit_promotions(from: Square, to: Square, capture: bool, list: &mut MoveList) {
    let base = if capture {
        flag::PROMO_KNIGHT_CAPTURE
    } else {
        flag::PROMO_KNIGHT
    };
    list.push(Move::new(from, to, base)); // knight
    list.push(Move::new(from, to, base + 1)); // bishop
    list.push(Move::new(from, to, base + 2)); // rook
    list.push(Move::new(from, to, base + 3)); // queen
}

/// Generate legal en-passant captures (rare, so each is fully verified).
fn generate_en_passant(
    board: &Board,
    us: Color,
    them: Color,
    king_sq: Square,
    occ: Bitboard,
    ep: Square,
    list: &mut MoveList,
) {
    // Our pawns positioned to capture onto the ep square.
    let candidates =
        board.pieces_colored(us, PieceType::Pawn) & PAWN_ATTACKS[them.index()][ep.index()];
    for from in candidates {
        let captured = Square::from_file_rank(ep.file(), from.rank());
        if en_passant_is_legal(board, us, them, king_sq, occ, from, ep, captured) {
            list.push(Move::new(from, ep, flag::EN_PASSANT));
        }
    }
}

/// True if making the en-passant capture leaves our king safe. This single test
/// subsumes pins, check evasion, and the horizontal discovered-check edge case,
/// because both the moving pawn and the captured pawn are removed at once.
#[allow(clippy::too_many_arguments)]
fn en_passant_is_legal(
    board: &Board,
    us: Color,
    them: Color,
    king_sq: Square,
    occ: Bitboard,
    from: Square,
    ep: Square,
    captured: Square,
) -> bool {
    let occ_after = (occ ^ Bitboard::from_square(from) ^ Bitboard::from_square(captured))
        | Bitboard::from_square(ep);
    let enemy_pawns = board.pieces_colored(them, PieceType::Pawn) ^ Bitboard::from_square(captured);
    let rook_like =
        board.pieces_colored(them, PieceType::Rook) | board.pieces_colored(them, PieceType::Queen);
    let bishop_like = board.pieces_colored(them, PieceType::Bishop)
        | board.pieces_colored(them, PieceType::Queen);
    let k = king_sq.index();

    let attacked = (PAWN_ATTACKS[us.index()][k] & enemy_pawns)
        | (KNIGHT_ATTACKS[k] & board.pieces_colored(them, PieceType::Knight))
        | (KING_ATTACKS[k] & board.pieces_colored(them, PieceType::King))
        | (rook_attacks(king_sq, occ_after) & rook_like)
        | (bishop_attacks(king_sq, occ_after) & bishop_like);

    attacked.is_empty()
}
