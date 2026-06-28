use thiserror::Error;

use crate::SideToMove;

/// All piece types in Shogi (including promoted variants).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PieceType {
    Pawn,
    Lance,
    Knight,
    Silver,
    Gold,
    Bishop,
    Rook,
    King,
    ProPawn,
    ProLance,
    ProKnight,
    ProSilver,
    /// Promoted Bishop
    Horse,
    /// Promoted Rook
    Dragon,
}

impl PieceType {
    pub fn demote(self) -> PieceType {
        match self {
            PieceType::ProPawn => PieceType::Pawn,
            PieceType::ProLance => PieceType::Lance,
            PieceType::ProKnight => PieceType::Knight,
            PieceType::ProSilver => PieceType::Silver,
            PieceType::Horse => PieceType::Bishop,
            PieceType::Dragon => PieceType::Rook,
            p => p,
        }
    }

    pub fn promote(self) -> PieceType {
        match self {
            PieceType::Pawn => PieceType::ProPawn,
            PieceType::Lance => PieceType::ProLance,
            PieceType::Knight => PieceType::ProKnight,
            PieceType::Silver => PieceType::ProSilver,
            PieceType::Bishop => PieceType::Horse,
            PieceType::Rook => PieceType::Dragon,
            p => p,
        }
    }

    fn hand_idx(self) -> Option<usize> {
        match self {
            PieceType::Rook => Some(0),
            PieceType::Bishop => Some(1),
            PieceType::Gold => Some(2),
            PieceType::Silver => Some(3),
            PieceType::Knight => Some(4),
            PieceType::Lance => Some(5),
            PieceType::Pawn => Some(6),
            _ => None,
        }
    }
}

fn piece_sfen(color: SideToMove, pt: PieceType) -> &'static str {
    match (color, pt) {
        (SideToMove::Black, PieceType::Pawn) => "P",
        (SideToMove::Black, PieceType::Lance) => "L",
        (SideToMove::Black, PieceType::Knight) => "N",
        (SideToMove::Black, PieceType::Silver) => "S",
        (SideToMove::Black, PieceType::Gold) => "G",
        (SideToMove::Black, PieceType::Bishop) => "B",
        (SideToMove::Black, PieceType::Rook) => "R",
        (SideToMove::Black, PieceType::King) => "K",
        (SideToMove::Black, PieceType::ProPawn) => "+P",
        (SideToMove::Black, PieceType::ProLance) => "+L",
        (SideToMove::Black, PieceType::ProKnight) => "+N",
        (SideToMove::Black, PieceType::ProSilver) => "+S",
        (SideToMove::Black, PieceType::Horse) => "+B",
        (SideToMove::Black, PieceType::Dragon) => "+R",
        (SideToMove::White, PieceType::Pawn) => "p",
        (SideToMove::White, PieceType::Lance) => "l",
        (SideToMove::White, PieceType::Knight) => "n",
        (SideToMove::White, PieceType::Silver) => "s",
        (SideToMove::White, PieceType::Gold) => "g",
        (SideToMove::White, PieceType::Bishop) => "b",
        (SideToMove::White, PieceType::Rook) => "r",
        (SideToMove::White, PieceType::King) => "k",
        (SideToMove::White, PieceType::ProPawn) => "+p",
        (SideToMove::White, PieceType::ProLance) => "+l",
        (SideToMove::White, PieceType::ProKnight) => "+n",
        (SideToMove::White, PieceType::ProSilver) => "+s",
        (SideToMove::White, PieceType::Horse) => "+b",
        (SideToMove::White, PieceType::Dragon) => "+r",
    }
}

#[derive(Debug, Error)]
pub enum BoardError {
    #[error("no piece at file={file} rank={rank}")]
    NoPieceAtSquare { file: u8, rank: u8 },
    #[error("no piece of that type in hand")]
    NoPieceInHand,
    #[error("invalid piece type for hand")]
    InvalidPiece,
}

/// grid[rank_idx][file_idx]
/// rank_idx = rank - 1   (0 = rank1/top,    8 = rank9/bottom)
/// file_idx = 9 - file   (0 = file9/leftmost in SFEN, 8 = file1/rightmost)
type Grid = [[Option<(SideToMove, PieceType)>; 9]; 9];

pub struct Board {
    pub grid: Grid,
    /// hand[color_idx][piece_idx]: R=0,B=1,G=2,S=3,N=4,L=5,P=6
    pub hand: [[u8; 7]; 2],
    pub side: SideToMove,
    pub move_count: u32,
}

fn color_idx(c: SideToMove) -> usize {
    match c {
        SideToMove::Black => 0,
        SideToMove::White => 1,
    }
}

fn sq(file: u8, rank: u8) -> (usize, usize) {
    ((rank - 1) as usize, (9 - file) as usize)
}

fn initial_grid() -> Grid {
    let mut g: Grid = [[None; 9]; 9];
    use PieceType::*;
    let back = [
        Lance, Knight, Silver, Gold, King, Gold, Silver, Knight, Lance,
    ];
    for (fi, &pt) in back.iter().enumerate() {
        g[0][fi] = Some((SideToMove::White, pt));
        g[8][fi] = Some((SideToMove::Black, pt));
    }
    g[1][1] = Some((SideToMove::White, Rook));
    g[1][7] = Some((SideToMove::White, Bishop));
    g[2].fill(Some((SideToMove::White, Pawn)));
    g[6].fill(Some((SideToMove::Black, Pawn)));
    g[7][1] = Some((SideToMove::Black, Bishop));
    g[7][7] = Some((SideToMove::Black, Rook));
    g
}

// ── Zobrist hashing ───────────────────────────────────────────────────────────

fn piece_idx(pt: PieceType) -> usize {
    match pt {
        PieceType::Pawn => 0,
        PieceType::Lance => 1,
        PieceType::Knight => 2,
        PieceType::Silver => 3,
        PieceType::Gold => 4,
        PieceType::Bishop => 5,
        PieceType::Rook => 6,
        PieceType::King => 7,
        PieceType::ProPawn => 8,
        PieceType::ProLance => 9,
        PieceType::ProKnight => 10,
        PieceType::ProSilver => 11,
        PieceType::Horse => 12,
        PieceType::Dragon => 13,
    }
}

const fn make_zobrist() -> [[[u64; 81]; 14]; 2] {
    let mut t = [[[0u64; 81]; 14]; 2];
    let mut seed: u64 = 0xdeadbeefcafebabe;
    let mut c = 0;
    while c < 2 {
        let mut p = 0;
        while p < 14 {
            let mut s = 0;
            while s < 81 {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                t[c][p][s] = seed;
                s += 1;
            }
            p += 1;
        }
        c += 1;
    }
    t
}
static ZOBRIST: [[[u64; 81]; 14]; 2] = make_zobrist();
const ZOBRIST_SIDE: u64 = 0x9e3779b97f4a7c15; // XOR'd in when White to move

fn sfen_piece(c: char, promoted: bool) -> Option<(usize, usize)> {
    let color = if c.is_uppercase() { 0 } else { 1 };
    let idx = match (c.to_ascii_uppercase(), promoted) {
        ('P', false) => 0,
        ('L', false) => 1,
        ('N', false) => 2,
        ('S', false) => 3,
        ('G', false) => 4,
        ('B', false) => 5,
        ('R', false) => 6,
        ('K', false) => 7,
        ('P', true) => 8,
        ('L', true) => 9,
        ('N', true) => 10,
        ('S', true) => 11,
        ('B', true) => 12,
        ('R', true) => 13,
        _ => return None,
    };
    Some((color, idx))
}

/// Compute a Zobrist hash directly from a SFEN string (board + side to move only).
/// Returns `None` if the SFEN is unparseable.
pub fn zobrist_from_sfen(sfen: &str) -> Option<u64> {
    let mut parts = sfen.split_whitespace();
    let board_str = parts.next()?;
    let side_str = parts.next()?;

    let mut h: u64 = if side_str == "w" { ZOBRIST_SIDE } else { 0 };
    let mut sq = 0usize; // 0..81 (ri*9 + fi)
    let chars: Vec<char> = board_str.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '/' {
            i += 1;
            continue;
        }
        if ch == '+' {
            i += 1;
            if i >= chars.len() {
                return None;
            }
            let (color, pidx) = sfen_piece(chars[i], true)?;
            h ^= ZOBRIST[color][pidx][sq];
            sq += 1;
        } else if ch.is_ascii_digit() {
            sq += (ch as u8 - b'0') as usize;
        } else {
            let (color, pidx) = sfen_piece(ch, false)?;
            h ^= ZOBRIST[color][pidx][sq];
            sq += 1;
        }
        i += 1;
    }
    Some(h)
}

/// Can an `opp` piece of type `pt` (whose forward ri-direction is `opp_fwd`) attack along
/// direction `(pdr, pdf)` for `dist` steps? Called once per ray after the blocking scan.
fn attacks_along(
    pt: PieceType,
    opp_fwd: i32,
    pdr: i32,
    _pdf: i32,
    dist: u32,
    is_diag: bool,
) -> bool {
    match pt {
        PieceType::Rook => !is_diag,
        PieceType::Bishop => is_diag,
        PieceType::Dragon => !is_diag || dist == 1, // Rook + king-range
        PieceType::Horse => is_diag || dist == 1,   // Bishop + king-range
        PieceType::Lance => !is_diag && pdr == opp_fwd, // forward column, any dist
        PieceType::Pawn => !is_diag && pdr == opp_fwd && dist == 1,
        PieceType::King => dist == 1,
        // Silver: forward + all diagonals
        PieceType::Silver => dist == 1 && (is_diag || pdr == opp_fwd),
        // Gold-like: all orthogonals + forward diagonals
        PieceType::Gold
        | PieceType::ProPawn
        | PieceType::ProLance
        | PieceType::ProKnight
        | PieceType::ProSilver => dist == 1 && (!is_diag || pdr == opp_fwd),
        PieceType::Knight => false, // handled outside ray loops
    }
}

impl Board {
    pub fn initial(side: SideToMove) -> Self {
        Board {
            grid: initial_grid(),
            hand: [[0; 7]; 2],
            side,
            move_count: 1,
        }
    }

    /// Apply a normal (non-drop) move. `to_piece` is the piece type AFTER the move
    /// (already promoted if applicable — caller is responsible for this).
    pub fn apply_normal(
        &mut self,
        color: SideToMove,
        from_file: u8,
        from_rank: u8,
        to_file: u8,
        to_rank: u8,
        to_piece: PieceType,
    ) -> Result<(), BoardError> {
        let (from_ri, from_fi) = sq(from_file, from_rank);
        let (to_ri, to_fi) = sq(to_file, to_rank);
        self.grid[from_ri][from_fi]
            .take()
            .ok_or(BoardError::NoPieceAtSquare {
                file: from_file,
                rank: from_rank,
            })?;
        if let Some((_, cap)) = self.grid[to_ri][to_fi].take()
            && let Some(hi) = cap.demote().hand_idx()
        {
            self.hand[color_idx(color)][hi] += 1;
        }
        self.grid[to_ri][to_fi] = Some((color, to_piece));
        self.advance_turn();
        Ok(())
    }

    /// Apply a drop move.
    pub fn apply_drop(
        &mut self,
        color: SideToMove,
        to_file: u8,
        to_rank: u8,
        piece: PieceType,
    ) -> Result<(), BoardError> {
        let hi = piece.demote().hand_idx().ok_or(BoardError::InvalidPiece)?;
        if self.hand[color_idx(color)][hi] == 0 {
            return Err(BoardError::NoPieceInHand);
        }
        self.hand[color_idx(color)][hi] -= 1;
        let (to_ri, to_fi) = sq(to_file, to_rank);
        self.grid[to_ri][to_fi] = Some((color, piece));
        self.advance_turn();
        Ok(())
    }

    fn advance_turn(&mut self) {
        self.side = match self.side {
            SideToMove::Black => SideToMove::White,
            SideToMove::White => SideToMove::Black,
        };
        self.move_count += 1;
    }

    pub fn to_sfen(&self) -> String {
        let mut board = String::new();
        for ri in 0..9 {
            let mut empty = 0u8;
            for fi in 0..9 {
                match self.grid[ri][fi] {
                    None => empty += 1,
                    Some((c, p)) => {
                        if empty > 0 {
                            board.push_str(&empty.to_string());
                            empty = 0;
                        }
                        board.push_str(piece_sfen(c, p));
                    }
                }
            }
            if empty > 0 {
                board.push_str(&empty.to_string());
            }
            if ri < 8 {
                board.push('/');
            }
        }
        let side = if self.side == SideToMove::Black {
            'b'
        } else {
            'w'
        };
        format!(
            "{} {} {} {}",
            board,
            side,
            self.hand_sfen(),
            self.move_count
        )
    }

    /// Returns true if a normal move to (to_file, to_rank) by mover_color captures an opponent piece.
    /// Compute Zobrist hash of the current position (board pieces + side to move).
    pub fn zobrist_hash(&self) -> u64 {
        let mut h: u64 = if self.side == SideToMove::White {
            ZOBRIST_SIDE
        } else {
            0
        };
        for ri in 0..9usize {
            for fi in 0..9usize {
                if let Some((color, pt)) = self.grid[ri][fi] {
                    h ^= ZOBRIST[color_idx(color)][piece_idx(pt)][ri * 9 + fi];
                }
            }
        }
        h
    }

    pub fn is_capture(&self, to_file: u8, to_rank: u8, mover_color: SideToMove) -> bool {
        let (ri, fi) = sq(to_file, to_rank);
        matches!(self.grid[ri][fi], Some((c, _)) if c != mover_color)
    }

    /// Returns true if the side to move is in check (no move generation needed).
    pub fn is_in_check(&self) -> bool {
        let us = self.side;
        let opp = match us {
            SideToMove::Black => SideToMove::White,
            SideToMove::White => SideToMove::Black,
        };
        // opp's "forward" ri-direction (the direction opp pieces advance)
        let opp_fwd: i32 = match opp {
            SideToMove::Black => -1,
            SideToMove::White => 1,
        };

        // Locate our king
        let mut king_pos = None;
        'find: for r in 0..9usize {
            for f in 0..9usize {
                if self.grid[r][f] == Some((us, PieceType::King)) {
                    king_pos = Some((r, f));
                    break 'find;
                }
            }
        }
        let (kr, kf) = match king_pos {
            Some(p) => p,
            None => return false,
        };

        // Ray scan in all 8 directions; first piece in each ray blocks further pieces.
        for &(dr, df) in &[
            (-1i32, 0i32),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (-1, 1),
            (1, -1),
            (1, 1),
        ] {
            let is_diag = dr != 0 && df != 0;
            let (mut r, mut f) = (kr as i32 + dr, kf as i32 + df);
            let mut dist = 1u32;
            while (0..9i32).contains(&r) && (0..9i32).contains(&f) {
                if let Some((c, pt)) = self.grid[r as usize][f as usize] {
                    if c == opp && attacks_along(pt, opp_fwd, -dr, -df, dist, is_diag) {
                        return true;
                    }
                    break;
                }
                r += dr;
                f += df;
                dist += 1;
            }
        }

        // Knight jumps (not blocked by intervening pieces)
        // opp knight at (kr - 2*opp_fwd, kf±1) attacks king
        let knight_r = kr as i32 - 2 * opp_fwd;
        for &kdf in &[-1i32, 1] {
            let knight_f = kf as i32 + kdf;
            if (0..9i32).contains(&knight_r)
                && (0..9i32).contains(&knight_f)
                && self.grid[knight_r as usize][knight_f as usize] == Some((opp, PieceType::Knight))
            {
                return true;
            }
        }

        false
    }

    fn hand_sfen(&self) -> String {
        let pieces = [
            (PieceType::Rook, 0usize),
            (PieceType::Bishop, 1),
            (PieceType::Gold, 2),
            (PieceType::Silver, 3),
            (PieceType::Knight, 4),
            (PieceType::Lance, 5),
            (PieceType::Pawn, 6),
        ];
        let mut s = String::new();
        for color in [SideToMove::Black, SideToMove::White] {
            for (pt, idx) in pieces {
                let n = self.hand[color_idx(color)][idx];
                if n > 0 {
                    if n > 1 {
                        s.push_str(&n.to_string());
                    }
                    s.push_str(piece_sfen(color, pt));
                }
            }
        }
        if s.is_empty() { "-".to_string() } else { s }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_sfen() {
        let b = Board::initial(SideToMove::Black);
        assert_eq!(
            b.to_sfen(),
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"
        );
    }

    fn empty_board(side: SideToMove) -> Board {
        Board {
            grid: [[None; 9]; 9],
            hand: [[0; 7]; 2],
            side,
            move_count: 1,
        }
    }

    #[test]
    fn not_in_check_initially() {
        assert!(!Board::initial(SideToMove::Black).is_in_check());
    }

    #[test]
    fn rook_check() {
        // White rook on same file above Black king → Black in check
        let mut b = empty_board(SideToMove::Black);
        let (kr, kf) = (8usize, 4usize); // Black king at ri=8,fi=4 (file5,rank9)
        b.grid[kr][kf] = Some((SideToMove::Black, PieceType::King));
        b.grid[5][kf] = Some((SideToMove::White, PieceType::Rook)); // same column, above
        assert!(b.is_in_check());
    }

    #[test]
    fn rook_blocked() {
        // Interposing piece blocks the rook → not in check
        let mut b = empty_board(SideToMove::Black);
        let (kr, kf) = (8usize, 4usize);
        b.grid[kr][kf] = Some((SideToMove::Black, PieceType::King));
        b.grid[5][kf] = Some((SideToMove::White, PieceType::Rook));
        b.grid[7][kf] = Some((SideToMove::Black, PieceType::Pawn)); // blocks
        assert!(!b.is_in_check());
    }

    #[test]
    fn pawn_check() {
        // White pawn one square "above" (ri-1) Black king → Black in check
        let mut b = empty_board(SideToMove::Black);
        let (kr, kf) = (5usize, 4usize);
        b.grid[kr][kf] = Some((SideToMove::Black, PieceType::King));
        b.grid[kr - 1][kf] = Some((SideToMove::White, PieceType::Pawn));
        assert!(b.is_in_check());
    }

    #[test]
    fn knight_check() {
        // White knight at (kr-2, kf+1) attacks Black king — jumps over pieces
        let mut b = empty_board(SideToMove::Black);
        let (kr, kf) = (6usize, 4usize);
        b.grid[kr][kf] = Some((SideToMove::Black, PieceType::King));
        // White knight: opp_fwd=+1, knight_r = kr - 2*1 = kr-2
        b.grid[kr - 2][kf + 1] = Some((SideToMove::White, PieceType::Knight));
        b.grid[kr - 1][kf] = Some((SideToMove::Black, PieceType::Gold)); // blocker on the path
        assert!(b.is_in_check(), "knight jumps over blocker");
    }
}
