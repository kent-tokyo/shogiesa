use std::collections::HashSet;
use std::fs;
use std::path::Path;

use shogiesa_core::{
    Board, ExtractConfig, PositionRecord, PositionTags, SideToMove, SourceInfo, board::PieceType,
    phase_from_ply,
};
use thiserror::Error;
use tracing::warn;

#[derive(Debug, Error)]
pub enum KifError {
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("KIF: {0}")]
    Parse(String),
}

impl From<shogiesa_core::BoardError> for KifError {
    fn from(e: shogiesa_core::BoardError) -> Self {
        KifError::Parse(e.to_string())
    }
}

// --- KIF piece name (kanji) → PieceType ---

fn piece_from_kif(s: &str) -> Option<(PieceType, &str)> {
    // 2-char names must be tried first
    let two_char: &[(&str, PieceType)] = &[
        ("成香", PieceType::ProLance),
        ("成桂", PieceType::ProKnight),
        ("成銀", PieceType::ProSilver),
    ];
    let one_char: &[(&str, PieceType)] = &[
        ("歩", PieceType::Pawn),
        ("香", PieceType::Lance),
        ("桂", PieceType::Knight),
        ("銀", PieceType::Silver),
        ("金", PieceType::Gold),
        ("角", PieceType::Bishop),
        ("飛", PieceType::Rook),
        ("王", PieceType::King),
        ("玉", PieceType::King),
        ("と", PieceType::ProPawn),
        ("馬", PieceType::Horse),
        ("竜", PieceType::Dragon),
        ("龍", PieceType::Dragon),
    ];
    for &(name, pt) in two_char.iter().chain(one_char) {
        if let Some(rest) = s.strip_prefix(name) {
            return Some((pt, rest));
        }
    }
    None
}

/// Convert a KIF full-width column digit (１..９) to file number (1..9).
fn fullwidth_col(c: char) -> Option<u8> {
    let code = c as u32;
    if (0xFF11..=0xFF19).contains(&code) {
        Some((code - 0xFF10) as u8)
    } else {
        None
    }
}

/// Convert a KIF kanji rank (一..九) to rank number (1..9).
fn kanji_rank(c: char) -> Option<u8> {
    match c {
        '一' => Some(1),
        '二' => Some(2),
        '三' => Some(3),
        '四' => Some(4),
        '五' => Some(5),
        '六' => Some(6),
        '七' => Some(7),
        '八' => Some(8),
        '九' => Some(9),
        _ => None,
    }
}

/// Parsed KIF move.
struct KifMove {
    dest_file: u8,
    dest_rank: u8,
    /// Piece type AFTER the move (already promoted if applicable).
    piece: PieceType,
    is_drop: bool,
    from_file: u8,
    from_rank: u8,
}

/// Parse a single KIF move token (the part after the ply number).
/// Returns `None` for non-move lines (resign, interrupt, etc.).
fn parse_kif_move(token: &str) -> Option<KifMove> {
    let token = token.trim();

    // "同" = same destination as previous move — not supported in initial impl
    if token.starts_with('同') {
        return None;
    }

    // Destination: full-width column + half-width rank digit
    let mut chars = token.chars();
    let dest_file_c = chars.next()?;
    let dest_file = fullwidth_col(dest_file_c)?;
    let dest_rank_c = chars.next()?;
    let dest_rank = kanji_rank(dest_rank_c)?;
    let rest = &token[dest_file_c.len_utf8() + dest_rank_c.len_utf8()..];

    // Piece name
    let (base_piece, rest) = piece_from_kif(rest)?;

    // Promotion or drop suffix
    let promotes = rest.starts_with('成');
    let is_drop = rest.starts_with('打');

    let rest = if promotes {
        &rest['成'.len_utf8()..]
    } else if is_drop {
        &rest['打'.len_utf8()..]
    } else {
        rest
    };

    let piece = if promotes {
        base_piece.promote()
    } else {
        base_piece
    };

    if is_drop {
        return Some(KifMove {
            dest_file,
            dest_rank,
            piece,
            is_drop: true,
            from_file: 0,
            from_rank: 0,
        });
    }

    // Origin: (file rank) — e.g., (77), (31)
    let inner = rest.trim().strip_prefix('(')?.split(')').next()?;
    let bytes = inner.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let from_file = bytes[0] - b'0';
    let from_rank = bytes[1] - b'0';

    Some(KifMove {
        dest_file,
        dest_rank,
        piece,
        is_drop,
        from_file,
        from_rank,
    })
}

pub fn extract_from_str(
    content: &str,
    source_path: &str,
    config: &ExtractConfig,
    seen: &mut HashSet<String>,
) -> Result<Vec<PositionRecord>, KifError> {
    let mut board = Board::initial(SideToMove::Black);
    let mut out = Vec::new();
    let mut ply: u32 = 0;
    let mut in_moves = false;

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') || line.starts_with('*') {
            continue;
        }

        // Handicap check
        if line.starts_with("手合割") {
            let handicap = line
                .trim_start_matches("手合割")
                .trim_start_matches(['：', ':']);
            if !handicap.trim().starts_with("平手") {
                return Err(KifError::Parse(format!(
                    "unsupported handicap: {handicap:?} (only 平手 is supported)"
                )));
            }
            continue;
        }

        // Move section header
        if line.starts_with("手数") {
            in_moves = true;
            continue;
        }

        if !in_moves {
            continue;
        }

        // Terminal markers
        if line.starts_with("まで") || line == "中断" || line == "投了" {
            break;
        }

        // Move line: starts with a ply number
        let Some((ply_str, rest)) = line.split_once(|c: char| c.is_whitespace()) else {
            continue;
        };
        let Ok(_ply_num) = ply_str.trim().parse::<u32>() else {
            continue;
        };

        let move_token = rest.trim();

        // Terminal actions inline (投了 etc.)
        if move_token.starts_with("投了")
            || move_token.starts_with("中断")
            || move_token.starts_with("まで")
        {
            break;
        }

        let Some(kif_move) = parse_kif_move(move_token) else {
            warn!(
                path = source_path,
                ply, "unsupported move syntax {move_token:?}, stopping game"
            );
            break;
        };

        let color = board.side;
        let result = if kif_move.is_drop {
            board.apply_drop(
                color,
                kif_move.dest_file,
                kif_move.dest_rank,
                kif_move.piece,
            )
        } else {
            board.apply_normal(
                color,
                kif_move.from_file,
                kif_move.from_rank,
                kif_move.dest_file,
                kif_move.dest_rank,
                kif_move.piece,
            )
        };

        if let Err(e) = result {
            warn!(path = source_path, ply, "board error: {e}");
            break;
        }

        ply += 1;

        if config.max_ply.is_some_and(|max| ply > max) {
            break;
        }
        if ply < config.min_ply {
            continue;
        }
        if !(ply - config.min_ply).is_multiple_of(config.every_n) {
            continue;
        }

        let sfen = board.to_sfen();
        if config.dedup && !seen.insert(sfen.clone()) {
            continue;
        }

        let tags = PositionTags {
            phase: phase_from_ply(ply),
            side_to_move: board.side,
            in_check: false,
            has_capture: false,
        };
        let source = SourceInfo {
            kind: "kif".to_string(),
            path: source_path.to_string(),
            ply,
        };
        out.push(PositionRecord::new(sfen, source, tags));
    }

    Ok(out)
}

pub fn extract_from_path(
    path: &Path,
    config: &ExtractConfig,
    seen: &mut HashSet<String>,
) -> Result<Vec<PositionRecord>, KifError> {
    let content = fs::read_to_string(path)?;
    let source = path.to_string_lossy().into_owned();
    extract_from_str(&content, &source, config, seen)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piece_names_cover_all_types() {
        let cases = [
            ("歩", PieceType::Pawn),
            ("香", PieceType::Lance),
            ("桂", PieceType::Knight),
            ("銀", PieceType::Silver),
            ("金", PieceType::Gold),
            ("角", PieceType::Bishop),
            ("飛", PieceType::Rook),
            ("玉", PieceType::King),
            ("と", PieceType::ProPawn),
            ("成香", PieceType::ProLance),
            ("成桂", PieceType::ProKnight),
            ("成銀", PieceType::ProSilver),
            ("馬", PieceType::Horse),
            ("竜", PieceType::Dragon),
        ];
        for (name, expected) in cases {
            let (pt, _) = piece_from_kif(name).unwrap_or_else(|| panic!("missing: {name}"));
            assert_eq!(pt, expected, "piece: {name}");
        }
    }

    #[test]
    fn parse_normal_move() {
        let m = parse_kif_move("７六歩(77)").unwrap();
        assert_eq!(m.dest_file, 7);
        assert_eq!(m.dest_rank, 6);
        assert_eq!(m.piece, PieceType::Pawn);
        assert!(!m.is_drop);
        assert_eq!(m.from_file, 7);
        assert_eq!(m.from_rank, 7);
    }

    #[test]
    fn parse_promotion_move() {
        let m = parse_kif_move("２二角成(88)").unwrap();
        assert_eq!(m.dest_file, 2);
        assert_eq!(m.dest_rank, 2);
        assert_eq!(m.piece, PieceType::Horse); // 角 promoted → Horse
        assert!(!m.is_drop);
        assert_eq!(m.from_file, 8);
        assert_eq!(m.from_rank, 8);
    }

    #[test]
    fn parse_drop_move() {
        let m = parse_kif_move("４五角打").unwrap();
        assert_eq!(m.dest_file, 4);
        assert_eq!(m.dest_rank, 5);
        assert_eq!(m.piece, PieceType::Bishop);
        assert!(m.is_drop);
    }
}
