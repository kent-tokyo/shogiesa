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
}
