/// CSA-specific conversions for shogiesa-core::Board.
use shogiesa_core::{Board, BoardError, PieceType, SideToMove};

pub fn from_csa_color(c: csa::Color) -> SideToMove {
    match c {
        csa::Color::Black => SideToMove::Black,
        csa::Color::White => SideToMove::White,
    }
}

pub fn from_csa_piece(p: csa::PieceType) -> Option<PieceType> {
    use csa::PieceType as C;
    Some(match p {
        C::Pawn => PieceType::Pawn,
        C::Lance => PieceType::Lance,
        C::Knight => PieceType::Knight,
        C::Silver => PieceType::Silver,
        C::Gold => PieceType::Gold,
        C::Bishop => PieceType::Bishop,
        C::Rook => PieceType::Rook,
        C::King => PieceType::King,
        C::ProPawn => PieceType::ProPawn,
        C::ProLance => PieceType::ProLance,
        C::ProKnight => PieceType::ProKnight,
        C::ProSilver => PieceType::ProSilver,
        C::Horse => PieceType::Horse,
        C::Dragon => PieceType::Dragon,
        C::All => return None,
    })
}

pub fn board_from_csa_position(pos: &csa::Position) -> Board {
    if let Some(bulk) = &pos.bulk {
        let mut grid = [[None; 9]; 9];
        for ri in 0..9 {
            for fi in 0..9 {
                if let Some((color, piece)) = bulk[ri][fi]
                    && let Some(piece) = from_csa_piece(piece)
                {
                    grid[ri][fi] = Some((from_csa_color(color), piece));
                }
            }
        }
        Board {
            grid,
            hand: [[0; 7]; 2],
            side: from_csa_color(pos.side_to_move),
            move_count: 1,
        }
    } else {
        let mut board = Board::initial(from_csa_color(pos.side_to_move));
        for (s, _) in &pos.drop_pieces {
            let ri = (s.rank - 1) as usize;
            let fi = (9 - s.file) as usize;
            board.grid[ri][fi] = None;
        }
        for &(color, s, piece) in &pos.add_pieces {
            if let Some(piece) = from_csa_piece(piece) {
                let ri = (s.rank - 1) as usize;
                let fi = (9 - s.file) as usize;
                board.grid[ri][fi] = Some((from_csa_color(color), piece));
            }
        }
        board
    }
}

pub fn apply_csa_action(board: &mut Board, action: csa::Action) -> Result<(), BoardError> {
    let csa::Action::Move(color, from, to, to_pt) = action else {
        return Ok(());
    };
    let color = from_csa_color(color);
    let to_piece = from_csa_piece(to_pt).ok_or(BoardError::InvalidPiece)?;
    if from.file == 0 {
        board.apply_drop(color, to.file, to.rank, to_piece)
    } else {
        board.apply_normal(color, from.file, from.rank, to.file, to.rank, to_piece)
    }
}
