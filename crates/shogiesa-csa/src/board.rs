use csa::{Action, Color, PieceType, Square};
use thiserror::Error;

// grid[rank_idx][file_idx]
// rank_idx = csa_rank - 1  (0 = rank1/top, 8 = rank9/bottom)
// file_idx = 9 - csa_file  (0 = file9/leftmost in SFEN, 8 = file1/rightmost)
type Grid = [[Option<(Color, PieceType)>; 9]; 9];

#[derive(Debug, Error)]
pub enum BoardError {
    #[error("no piece at ({file},{rank})")]
    NoPieceAtSquare { file: u8, rank: u8 },
    #[error("no piece of that type in hand")]
    NoPieceInHand,
    #[error("invalid piece type for hand")]
    InvalidPiece,
}

pub struct Board {
    grid: Grid,
    // hand[color_idx][piece_idx]: R=0,B=1,G=2,S=3,N=4,L=5,P=6
    hand: [[u8; 7]; 2],
    pub side: Color,
    pub move_count: u32,
}

fn sq(s: &Square) -> (usize, usize) {
    ((s.rank - 1) as usize, (9 - s.file) as usize)
}

fn ci(c: Color) -> usize {
    match c {
        Color::Black => 0,
        Color::White => 1,
    }
}

fn demote(pt: PieceType) -> PieceType {
    match pt {
        PieceType::ProPawn => PieceType::Pawn,
        PieceType::ProLance => PieceType::Lance,
        PieceType::ProKnight => PieceType::Knight,
        PieceType::ProSilver => PieceType::Silver,
        PieceType::Horse => PieceType::Bishop,
        PieceType::Dragon => PieceType::Rook,
        p => p,
    }
}

fn hand_idx(pt: PieceType) -> Option<usize> {
    match pt {
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

fn piece_sfen(color: Color, pt: PieceType) -> &'static str {
    match (color, pt) {
        (Color::Black, PieceType::Pawn) => "P",
        (Color::Black, PieceType::Lance) => "L",
        (Color::Black, PieceType::Knight) => "N",
        (Color::Black, PieceType::Silver) => "S",
        (Color::Black, PieceType::Gold) => "G",
        (Color::Black, PieceType::Bishop) => "B",
        (Color::Black, PieceType::Rook) => "R",
        (Color::Black, PieceType::King) => "K",
        (Color::Black, PieceType::ProPawn) => "+P",
        (Color::Black, PieceType::ProLance) => "+L",
        (Color::Black, PieceType::ProKnight) => "+N",
        (Color::Black, PieceType::ProSilver) => "+S",
        (Color::Black, PieceType::Horse) => "+B",
        (Color::Black, PieceType::Dragon) => "+R",
        (Color::White, PieceType::Pawn) => "p",
        (Color::White, PieceType::Lance) => "l",
        (Color::White, PieceType::Knight) => "n",
        (Color::White, PieceType::Silver) => "s",
        (Color::White, PieceType::Gold) => "g",
        (Color::White, PieceType::Bishop) => "b",
        (Color::White, PieceType::Rook) => "r",
        (Color::White, PieceType::King) => "k",
        (Color::White, PieceType::ProPawn) => "+p",
        (Color::White, PieceType::ProLance) => "+l",
        (Color::White, PieceType::ProKnight) => "+n",
        (Color::White, PieceType::ProSilver) => "+s",
        (Color::White, PieceType::Horse) => "+b",
        (Color::White, PieceType::Dragon) => "+r",
        _ => "?",
    }
}

fn initial_grid() -> Grid {
    let mut g: Grid = [[None; 9]; 9];
    use PieceType::*;
    // Back ranks: file9(fi=0) → file1(fi=8): L N S G K G S N L
    let back = [
        Lance, Knight, Silver, Gold, King, Gold, Silver, Knight, Lance,
    ];
    for (fi, &pt) in back.iter().enumerate() {
        g[0][fi] = Some((Color::White, pt)); // rank1
        g[8][fi] = Some((Color::Black, pt)); // rank9
    }
    // Rank2: White Rook at file8(fi=1), White Bishop at file2(fi=7)
    g[1][1] = Some((Color::White, Rook));
    g[1][7] = Some((Color::White, Bishop));
    // Rank3: White pawns
    g[2].fill(Some((Color::White, Pawn)));
    // Rank7: Black pawns
    g[6].fill(Some((Color::Black, Pawn)));
    // Rank8: Black Bishop at file8(fi=1), Black Rook at file2(fi=7)
    g[7][1] = Some((Color::Black, Bishop));
    g[7][7] = Some((Color::Black, Rook));
    g
}

impl Board {
    pub fn from_csa_position(pos: &csa::Position) -> Self {
        let mut grid = match &pos.bulk {
            Some(b) => *b,
            None => {
                let mut g = initial_grid();
                // PI handicap: drop_pieces clears squares from standard position
                for (s, _) in &pos.drop_pieces {
                    let (ri, fi) = sq(s);
                    g[ri][fi] = None;
                }
                for &(color, s, pt) in &pos.add_pieces {
                    let (ri, fi) = sq(&s);
                    g[ri][fi] = Some((color, pt));
                }
                g
            }
        };
        // Ensure no Out-of-bounds from add_pieces with file=0 (shouldn't happen)
        let _ = &mut grid;
        Board {
            grid,
            hand: [[0; 7]; 2],
            side: pos.side_to_move,
            move_count: 1,
        }
    }

    pub fn apply(&mut self, action: Action) -> Result<(), BoardError> {
        let Action::Move(color, from, to, to_pt) = action else {
            return Ok(());
        };
        let (to_ri, to_fi) = sq(&to);
        let color_i = ci(color);

        if from.file == 0 {
            // Drop move
            let base = demote(to_pt);
            let hi = hand_idx(base).ok_or(BoardError::InvalidPiece)?;
            if self.hand[color_i][hi] == 0 {
                return Err(BoardError::NoPieceInHand);
            }
            self.hand[color_i][hi] -= 1;
            self.grid[to_ri][to_fi] = Some((color, to_pt));
        } else {
            let (from_ri, from_fi) = sq(&from);
            self.grid[from_ri][from_fi]
                .take()
                .ok_or(BoardError::NoPieceAtSquare {
                    file: from.file,
                    rank: from.rank,
                })?;
            // Capture
            if let Some((_, cap)) = self.grid[to_ri][to_fi].take()
                && let Some(hi) = hand_idx(demote(cap))
            {
                self.hand[color_i][hi] += 1;
            }
            self.grid[to_ri][to_fi] = Some((color, to_pt));
        }

        self.side = match self.side {
            Color::Black => Color::White,
            Color::White => Color::Black,
        };
        self.move_count += 1;
        Ok(())
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
        let side = if self.side == Color::Black { 'b' } else { 'w' };
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
        for color in [Color::Black, Color::White] {
            for (pt, idx) in pieces {
                let n = self.hand[ci(color)][idx];
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
        let pos = csa::Position::default();
        let board = Board::from_csa_position(&pos);
        assert_eq!(
            board.to_sfen(),
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"
        );
    }
}
