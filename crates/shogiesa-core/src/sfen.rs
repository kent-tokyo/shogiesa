use thiserror::Error;

use crate::SideToMove;

/// A syntactically validated SFEN string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sfen(String);

#[derive(Debug, Error)]
pub enum SfenError {
    #[error("expected 4 fields, got {got}")]
    FieldCount { got: usize },
    #[error("board has {got} ranks, expected 9")]
    RankCount { got: usize },
    #[error("rank {rank} expands to {got} squares, expected 9")]
    RankWidth { rank: usize, got: u8 },
    #[error("unknown piece character {ch:?} in rank {rank}")]
    UnknownPiece { rank: usize, ch: char },
    #[error("invalid side to move: {got:?}")]
    InvalidSide { got: String },
    #[error("invalid hand: {got:?}")]
    InvalidHand { got: String },
    #[error("invalid move count: {got:?}")]
    InvalidMoveCount { got: String },
}

impl Sfen {
    pub fn parse(s: &str) -> Result<Self, SfenError> {
        let fields: Vec<&str> = s.split_ascii_whitespace().collect();
        if fields.len() != 4 {
            return Err(SfenError::FieldCount { got: fields.len() });
        }
        validate_board(fields[0])?;
        validate_side(fields[1])?;
        validate_hand(fields[2])?;
        validate_move_count(fields[3])?;
        Ok(Sfen(s.to_string()))
    }

    pub fn side_to_move(&self) -> SideToMove {
        // safe: validated in parse()
        match self.0.split_ascii_whitespace().nth(1) {
            Some("b") => SideToMove::Black,
            _ => SideToMove::White,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_board(board: &str) -> Result<(), SfenError> {
    let ranks: Vec<&str> = board.split('/').collect();
    if ranks.len() != 9 {
        return Err(SfenError::RankCount { got: ranks.len() });
    }
    for (ri, rank) in ranks.iter().enumerate() {
        let width = rank_width(rank, ri)?;
        if width != 9 {
            return Err(SfenError::RankWidth {
                rank: ri + 1,
                got: width,
            });
        }
    }
    Ok(())
}

fn rank_width(rank: &str, ri: usize) -> Result<u8, SfenError> {
    let mut width: u8 = 0;
    let mut chars = rank.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() {
            width += ch as u8 - b'0';
        } else if ch == '+' {
            // promoted piece — next char is the piece letter
            match chars.next() {
                Some(p) if is_piece(p) => width += 1,
                Some(p) => {
                    return Err(SfenError::UnknownPiece {
                        rank: ri + 1,
                        ch: p,
                    });
                }
                None => {
                    return Err(SfenError::UnknownPiece {
                        rank: ri + 1,
                        ch: '+',
                    });
                }
            }
        } else if is_piece(ch) {
            width += 1;
        } else {
            return Err(SfenError::UnknownPiece { rank: ri + 1, ch });
        }
    }
    Ok(width)
}

fn is_piece(ch: char) -> bool {
    matches!(
        ch,
        'P' | 'L'
            | 'N'
            | 'S'
            | 'G'
            | 'B'
            | 'R'
            | 'K'
            | 'p'
            | 'l'
            | 'n'
            | 's'
            | 'g'
            | 'b'
            | 'r'
            | 'k'
    )
}

fn validate_side(s: &str) -> Result<(), SfenError> {
    if s == "b" || s == "w" {
        Ok(())
    } else {
        Err(SfenError::InvalidSide { got: s.to_string() })
    }
}

fn validate_hand(s: &str) -> Result<(), SfenError> {
    if s == "-" {
        return Ok(());
    }
    let mut chars = s.chars().peekable();
    let mut saw_piece = false;
    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() {
            // optional count before a piece
            if !chars
                .peek()
                .is_some_and(|p| is_piece(*p) && !p.eq_ignore_ascii_case(&'k'))
            {
                return Err(SfenError::InvalidHand { got: s.to_string() });
            }
        } else if ch.eq_ignore_ascii_case(&'k') {
            // A king can never legitimately be captured/held in hand -- reject here so
            // `Board::from_sfen` can assume a well-formed hand string (its piece-index lookup
            // has no entry for King and would otherwise panic on this).
            return Err(SfenError::InvalidHand { got: s.to_string() });
        } else if is_piece(ch) {
            saw_piece = true;
        } else {
            return Err(SfenError::InvalidHand { got: s.to_string() });
        }
    }
    if !saw_piece {
        return Err(SfenError::InvalidHand { got: s.to_string() });
    }
    Ok(())
}

fn validate_move_count(s: &str) -> Result<(), SfenError> {
    match s.parse::<u32>() {
        Ok(n) if n >= 1 => Ok(()),
        _ => Err(SfenError::InvalidMoveCount { got: s.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STARTPOS: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";

    #[test]
    fn parse_startpos() {
        let sfen = Sfen::parse(STARTPOS).unwrap();
        assert_eq!(sfen.side_to_move(), SideToMove::Black);
    }

    #[test]
    fn parse_after_first_move() {
        let s = "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2";
        let sfen = Sfen::parse(s).unwrap();
        assert_eq!(sfen.side_to_move(), SideToMove::White);
    }

    #[test]
    fn reject_wrong_field_count() {
        // 3 fields instead of 4
        assert!(matches!(
            Sfen::parse("lnsgkgsnl b -"),
            Err(SfenError::FieldCount { .. })
        ));
        // 5 fields
        assert!(matches!(
            Sfen::parse("lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1 extra"),
            Err(SfenError::FieldCount { .. })
        ));
    }

    #[test]
    fn reject_bad_rank_count() {
        let bad = "lnsgkgsnl/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
        assert!(matches!(Sfen::parse(bad), Err(SfenError::RankCount { .. })));
    }

    #[test]
    fn reject_bad_rank_width() {
        let bad = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNLX b - 1";
        assert!(matches!(
            Sfen::parse(bad),
            Err(SfenError::RankWidth { .. }) | Err(SfenError::UnknownPiece { .. })
        ));
    }

    #[test]
    fn reject_bad_side() {
        let bad = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL x - 1";
        assert!(matches!(
            Sfen::parse(bad),
            Err(SfenError::InvalidSide { .. })
        ));
    }

    #[test]
    fn accept_hand_pieces() {
        let s = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b RBrp 10";
        assert!(Sfen::parse(s).is_ok());
    }

    #[test]
    fn reject_king_in_hand() {
        // A king can never legitimately be captured/held in hand -- `PieceType::hand_idx()` has
        // no entry for it, so accepting this here would let `Board::from_sfen` panic downstream
        // instead of surfacing a clean parse error.
        let bare = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b K 10";
        assert!(matches!(
            Sfen::parse(bare),
            Err(SfenError::InvalidHand { .. })
        ));
        let counted = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b 2k 10";
        assert!(matches!(
            Sfen::parse(counted),
            Err(SfenError::InvalidHand { .. })
        ));
    }

    #[test]
    fn reject_zero_move_count() {
        let bad = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 0";
        assert!(matches!(
            Sfen::parse(bad),
            Err(SfenError::InvalidMoveCount { .. })
        ));
    }
}
