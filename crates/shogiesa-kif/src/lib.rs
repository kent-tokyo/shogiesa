use std::collections::HashSet;
use std::fs;
use std::path::Path;

use shogiesa_core::{
    Board, ExtractConfig, GameOutcome, GameResultInfo, PositionRecord, PositionTags, RawMove,
    SideToMove, SourceInfo, UsiMove, board::PieceType, phase_from_ply,
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

/// Parse the `N` in a `変化：N手` (variation/branch) marker line.
/// Returns `None` if the line isn't a well-formed variation marker.
fn parse_henka_ply(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("変化")?;
    let rest = rest.trim_start_matches(['：', ':']);
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
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
///
/// `prev_dest` is the destination square of the previous move, needed to
/// resolve "同" (same-square) notation.
fn parse_kif_move(token: &str, prev_dest: Option<(u8, u8)>) -> Option<KifMove> {
    let token = token.trim();

    // "同" = same destination as previous move
    let (dest_file, dest_rank, rest) = if let Some(rest) = token.strip_prefix('同') {
        let (dest_file, dest_rank) = prev_dest?;
        (dest_file, dest_rank, rest.trim_start_matches(['　', ' ']))
    } else {
        // Destination: full-width column + half-width rank digit
        let mut chars = token.chars();
        let dest_file_c = chars.next()?;
        let dest_file = fullwidth_col(dest_file_c)?;
        let dest_rank_c = chars.next()?;
        let dest_rank = kanji_rank(dest_rank_c)?;
        let rest = &token[dest_file_c.len_utf8() + dest_rank_c.len_utf8()..];
        (dest_file, dest_rank, rest)
    };

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

/// Build the initial Board for a KIF handicap type.
///
/// Handicaps remove specific White pieces from the standard starting position.
/// White always moves first in handicap games.
///
/// grid[rank_idx][file_idx]: rank_idx = rank-1, file_idx = 9-file
/// White pieces on rank 1 (ri=0): fi=0=L fi=1=N fi=2=S fi=3=G fi=4=K fi=5=G fi=6=S fi=7=N fi=8=L
/// White flying pieces on rank 2 (ri=1): fi=1=R fi=7=B
fn handicap_board(name: &str) -> Option<Board> {
    // squares to clear (ri, fi) — always White's pieces
    let removals: &[(usize, usize)] = match name.trim() {
        "平手" => return Some(Board::initial(SideToMove::Black)),
        "香落ち" => &[(0, 8)],                           // file1 lance
        "右香落ち" => &[(0, 0)],                         // file9 lance
        "角落ち" => &[(1, 7)],                           // bishop
        "飛車落ち" | "飛落ち" => &[(1, 1)],              // rook
        "二枚落ち" => &[(1, 1), (1, 7)],                 // rook + bishop
        "四枚落ち" => &[(1, 1), (1, 7), (0, 0), (0, 8)], // + both lances
        "六枚落ち" => &[(1, 1), (1, 7), (0, 0), (0, 8), (0, 1), (0, 7)], // + both knights
        "八枚落ち" => &[
            (1, 1),
            (1, 7),
            (0, 0),
            (0, 8),
            (0, 1),
            (0, 7),
            (0, 2),
            (0, 6),
        ], // + both silvers
        "十枚落ち" => &[
            (1, 1),
            (1, 7),
            (0, 0),
            (0, 8),
            (0, 1),
            (0, 7),
            (0, 2),
            (0, 6),
            (0, 3),
            (0, 5),
        ], // + both golds
        _ => return None,
    };
    let mut board = Board::initial(SideToMove::White); // handicap: White moves first
    for &(ri, fi) in removals {
        board.grid[ri][fi] = None;
    }
    Some(board)
}

pub fn extract_from_str(
    content: &str,
    source_path: &str,
    config: &ExtractConfig,
    seen: &mut HashSet<String>,
) -> Result<Vec<PositionRecord>, KifError> {
    // Resolved via a full-game walk (see `extract_moves_from_str`) rather than re-derived here,
    // so game-result resolution can't drift out of sync with -- or, worse, invert the winner of --
    // the one already-tested implementation. Independent of `config.max_ply`: the terminal marker
    // this needs to see can be well past whatever ply the position-emitting loop below truncates
    // the mainline at.
    //
    // Degrades to `Unknown` provenance rather than propagating a failed walk (mirrors the CSA
    // extractor's identical guard) -- this function's own loop below is deliberately lenient
    // about mid-game errors (warns and stops just that block, keeping positions already
    // collected), and an unsupported-handicap error is the one case both functions already fail
    // on identically, so nothing is lost.
    let (outcome, mainline_result_source) = match extract_moves_from_str(content, source_path) {
        Ok((_, o, reason)) => (o, reason),
        Err(_) => (GameOutcome::Unknown, "kif_walk_error"),
    };

    let mut board = Board::initial(SideToMove::Black);
    let mut out = Vec::new();
    let mut ply: u32 = 0;
    let mut in_moves = false;
    let mut prev_dest: Option<(u8, u8)> = None;

    // Mainline board/prev_dest snapshot after `k` mainline moves, indexed by `k`. A `変化：N手`
    // marker branches from `checkpoints[N-1]`; branches never extend this (they only ever
    // resolve against the mainline, not against each other — nested variations are out of scope).
    let mut checkpoints: Vec<(Board, Option<(u8, u8)>)> = Vec::new();
    let mut current_path = source_path.to_string();
    let mut variation_count: u32 = 0;
    // Set together with current_path at each 変化 marker; None while still on the mainline.
    // Carried in SourceInfo so a variation and its mainline can be grouped by root_id without
    // parsing the '#varN@ply' suffix back out of `path`.
    let mut current_variation_id: Option<String> = None;
    let mut current_branch_from_ply: Option<u32> = None;
    // Whether the current block (mainline or a variation) is still accepting move lines. Only
    // mainline parsing may `break` the outer loop; every stop condition inside a variation
    // (terminal marker, parse error, max-ply) just ends that block so scanning can find the
    // next `変化` marker instead of abandoning the rest of the file.
    let mut accepting = false;

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') || line.starts_with('*') {
            continue;
        }

        // Handicap: set initial board and side-to-move
        if line.starts_with("手合割") {
            let handicap = line
                .trim_start_matches("手合割")
                .trim_start_matches(['：', ':']);
            match handicap_board(handicap.trim()) {
                Some(b) => board = b,
                None => {
                    return Err(KifError::Parse(format!(
                        "unsupported handicap: {handicap:?}"
                    )));
                }
            }
            continue;
        }

        // Move section header
        if line.starts_with("手数") {
            in_moves = true;
            accepting = true;
            checkpoints.push((board.clone(), prev_dest));
            continue;
        }

        if !in_moves {
            continue;
        }

        // Variation/branch marker: jump back to the mainline checkpoint at ply N-1 and start
        // extracting the branch's moves under a distinct source path.
        if line.starts_with("変化") {
            let branch = parse_henka_ply(line).and_then(|n| {
                if n == 0 {
                    return None;
                }
                checkpoints.get((n - 1) as usize).map(|cp| (n, cp.clone()))
            });
            match branch {
                Some((n, (cp_board, cp_prev_dest))) => {
                    variation_count += 1;
                    board = cp_board;
                    prev_dest = cp_prev_dest;
                    ply = n - 1;
                    current_path = format!("{source_path}#var{variation_count}@{n}");
                    current_variation_id = Some(format!("var{variation_count}"));
                    current_branch_from_ply = Some(n);
                    accepting = true;
                }
                None => {
                    warn!(
                        path = source_path,
                        line, "malformed or out-of-range 変化 marker, skipping"
                    );
                    accepting = false;
                }
            }
            continue;
        }

        // Terminal markers end the current block only; scanning continues for more 変化 blocks.
        if line.starts_with("まで") || line == "中断" || line == "投了" {
            accepting = false;
            continue;
        }

        if !accepting {
            continue;
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
            accepting = false;
            continue;
        }

        let is_mainline = current_path == source_path;

        let Some(kif_move) = parse_kif_move(move_token, prev_dest) else {
            warn!(
                path = source_path,
                ply, "unsupported move syntax {move_token:?}, stopping game"
            );
            if is_mainline {
                break;
            }
            accepting = false;
            continue;
        };

        let color = board.side;
        let has_capture =
            !kif_move.is_drop && board.is_capture(kif_move.dest_file, kif_move.dest_rank, color);

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
            if is_mainline {
                break;
            }
            accepting = false;
            continue;
        }

        // Updated for every played move (even ones later filtered out below),
        // so "同" always resolves against the true previous move.
        prev_dest = Some((kif_move.dest_file, kif_move.dest_rank));

        ply += 1;

        if is_mainline {
            checkpoints.push((board.clone(), prev_dest));
        }

        if config.max_ply.is_some_and(|max| ply > max) {
            if is_mainline {
                break;
            }
            accepting = false;
            continue;
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

        let in_check = board.is_in_check();
        let tags = PositionTags {
            phase: phase_from_ply(ply),
            side_to_move: board.side,
            in_check,
            has_capture,
        };
        let source = SourceInfo {
            kind: "kif".to_string(),
            path: current_path.clone(),
            ply,
            // Shared by the mainline and every variation branching from it, so `split` can
            // group them without depending on `path`'s '#varN@ply' suffix convention.
            root_id: Some(source_path.to_string()),
            variation_id: current_variation_id.clone(),
            branch_from_ply: current_branch_from_ply,
        };
        let mut rec = PositionRecord::new(sfen, source, tags);
        rec.game_result = if is_mainline {
            Some(GameResultInfo {
                outcome,
                result_source: mainline_result_source.to_string(),
            })
        } else {
            // A variation branch's own terminal (if any) isn't the actual game's result --
            // matches the convention `RawMove.outcome` already uses for variation moves.
            Some(GameResultInfo {
                outcome: GameOutcome::Unknown,
                result_source: "kif_variation".to_string(),
            })
        };
        out.push(rec);
    }

    Ok(out)
}

/// Decode raw KIF file bytes to a `String`, tolerating both UTF-8 and Shift_JIS (CP932) input.
///
/// Why: `.kif` is a legacy Windows-native Japanese format; a large fraction of real-world KIF
/// files in the wild are Shift_JIS, not UTF-8. This strips a UTF-8 BOM first -- its bytes decode
/// as valid UTF-8 `U+FEFF`, which is NOT whitespace by Unicode's definition, so leaving it in
/// would make `extract_from_str`'s `line.trim()` + prefix checks silently miss the first content
/// line (e.g. `手合割`) -- then attempts strict UTF-8 validation, falling back to a lossy
/// Shift_JIS decode only if that fails. Shift_JIS byte sequences essentially never coincidentally
/// validate as UTF-8 in real KIF text, so this two-step sniff is reliable in practice.
fn decode_kif_bytes(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    encoding_rs::SHIFT_JIS
        .decode_without_bom_handling(bytes)
        .0
        .into_owned()
}

pub fn extract_from_path(
    path: &Path,
    config: &ExtractConfig,
    seen: &mut HashSet<String>,
) -> Result<Vec<PositionRecord>, KifError> {
    let bytes = fs::read(path)?;
    let content = decode_kif_bytes(&bytes);
    let source = path.to_string_lossy().into_owned();
    extract_from_str(&content, &source, config, seen)
}

fn opponent(color: SideToMove) -> SideToMove {
    match color {
        SideToMove::Black => SideToMove::White,
        SideToMove::White => SideToMove::Black,
    }
}

/// Classifies a recognized terminal `token` (one `resolve_kif_terminal` already returned
/// `Some(outcome)` for) into a `result_source` reason code, for the mainline only -- distinguishes
/// an explicit abort (`中断`) from a genuinely ambiguous marker (an unrecognized `まで`-suffix,
/// `outcome == Unknown`) from an ordinary resolved marker.
fn kif_reason_for(token: &str, outcome: GameOutcome) -> &'static str {
    if token.starts_with("中断") {
        "kif_interrupted"
    } else if outcome == GameOutcome::Unknown {
        "kif_terminal_undetermined"
    } else {
        "kif_marker"
    }
}

/// Parses a KIF terminal marker -- either a standalone summary line (`まで77手で先手の勝ち`,
/// `持将棋`, `千日手`, `中断`) or an inline move-line token (`投了`, etc.) -- into a
/// `GameOutcome`, given whose turn it was when the marker appeared and which color moved first
/// in this game (先手/後手 name move ORDER, not a fixed color -- in a handicap game the
/// handicapped side, not Black, conventionally moves first, so "先手の勝ち" means "whoever
/// opened the game wins", not "Black wins"). `None` if `token` isn't a recognized terminal
/// marker at all.
fn resolve_kif_terminal(
    token: &str,
    side_to_move: SideToMove,
    first_mover: SideToMove,
) -> Option<GameOutcome> {
    if token.starts_with("まで") {
        let winner = if token.contains("先手の勝ち") {
            Some(first_mover)
        } else if token.contains("後手の勝ち") {
            Some(opponent(first_mover))
        } else {
            None
        };
        if let Some(w) = winner {
            return Some(match w {
                SideToMove::Black => GameOutcome::BlackWins,
                SideToMove::White => GameOutcome::WhiteWins,
            });
        }
        // Must fall through here rather than return early -- real KIF summary lines like "まで
        // 64手で千日手"/"まで120手で持将棋" carry their marker as a substring of the summary
        // line, not as its own standalone line.
        if token.contains("千日手") || token.contains("持将棋") {
            return Some(GameOutcome::Draw);
        }
        if token.contains("詰み") {
            return Some(match opponent(side_to_move) {
                SideToMove::Black => GameOutcome::BlackWins,
                SideToMove::White => GameOutcome::WhiteWins,
            });
        }
        return Some(GameOutcome::Unknown);
    }
    if token.starts_with("投了") || token.starts_with("詰み") {
        return Some(match opponent(side_to_move) {
            SideToMove::Black => GameOutcome::BlackWins,
            SideToMove::White => GameOutcome::WhiteWins,
        });
    }
    if token.starts_with("持将棋") || token.starts_with("千日手") {
        return Some(GameOutcome::Draw);
    }
    if token.starts_with("中断") {
        return Some(GameOutcome::Unknown);
    }
    None
}

/// Unfiltered full-game walk producing one `RawMove` per ply (mainline and every variation
/// branch), for lineprior-style export. Parallels `extract_from_str`'s mainline/変化 walk but:
/// captures the SFEN BEFORE each move (not after) and the USI move token itself; applies no
/// `min_ply`/`every_n`/`dedup` filtering; and resolves the mainline's `GameOutcome` from its
/// terminal marker instead of just stopping. Variation-branch moves always carry
/// `GameOutcome::Unknown` -- a branch's own terminal text (if any) isn't the actual game's
/// result -- but are still emitted and still share the mainline's `root_id`.
/// Also returns the resolved mainline `GameOutcome` directly (not just backfilled onto each
/// `RawMove`), since `extract_from_str` needs it even when the mainline has zero moves before its
/// terminal marker -- a case where `out`'s mainline prefix would otherwise be empty and the
/// outcome would be lost.
pub fn extract_moves_from_str(
    content: &str,
    source_path: &str,
) -> Result<(Vec<RawMove>, GameOutcome, &'static str), KifError> {
    let mut board = Board::initial(SideToMove::Black);
    let mut out: Vec<RawMove> = Vec::new();
    let mut ply: u32 = 0;
    let mut in_moves = false;
    let mut prev_dest: Option<(u8, u8)> = None;

    let mut checkpoints: Vec<(Board, Option<(u8, u8)>)> = Vec::new();
    let mut current_path = source_path.to_string();
    let mut variation_count: u32 = 0;
    let mut current_variation_id: Option<String> = None;
    let mut current_branch_from_ply: Option<u32> = None;
    let mut accepting = false;
    // Mainline `RawMove`s are always pushed before any variation's (the format never resumes
    // extending the mainline after a `変化` marker), so this stays a valid prefix bound.
    let mut mainline_move_count: usize = 0;
    let mut outcome = GameOutcome::Unknown;
    let mut reason: &'static str = "kif_no_terminal";
    // Captured once the move list starts (after any 手合割 handicap reassignment), since 先手/
    // 後手 in the summary line name move order, not a fixed color.
    let mut first_mover = SideToMove::Black;

    for line in content.lines() {
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') || line.starts_with('*') {
            continue;
        }

        if line.starts_with("手合割") {
            let handicap = line
                .trim_start_matches("手合割")
                .trim_start_matches(['：', ':']);
            match handicap_board(handicap.trim()) {
                Some(b) => board = b,
                None => {
                    return Err(KifError::Parse(format!(
                        "unsupported handicap: {handicap:?}"
                    )));
                }
            }
            continue;
        }

        if line.starts_with("手数") {
            in_moves = true;
            accepting = true;
            first_mover = board.side;
            checkpoints.push((board.clone(), prev_dest));
            continue;
        }

        if !in_moves {
            continue;
        }

        let is_mainline = current_path == source_path;

        if line.starts_with("変化") {
            let branch = parse_henka_ply(line).and_then(|n| {
                if n == 0 {
                    return None;
                }
                checkpoints.get((n - 1) as usize).map(|cp| (n, cp.clone()))
            });
            match branch {
                Some((n, (cp_board, cp_prev_dest))) => {
                    variation_count += 1;
                    board = cp_board;
                    prev_dest = cp_prev_dest;
                    ply = n - 1;
                    current_path = format!("{source_path}#var{variation_count}@{n}");
                    current_variation_id = Some(format!("var{variation_count}"));
                    current_branch_from_ply = Some(n);
                    accepting = true;
                }
                None => {
                    warn!(
                        path = source_path,
                        line, "malformed or out-of-range 変化 marker, skipping"
                    );
                    accepting = false;
                }
            }
            continue;
        }

        if let Some(o) = resolve_kif_terminal(line, board.side, first_mover) {
            if is_mainline {
                outcome = o;
                reason = kif_reason_for(line, o);
            }
            accepting = false;
            continue;
        }

        if !accepting {
            continue;
        }

        let Some((ply_str, rest)) = line.split_once(|c: char| c.is_whitespace()) else {
            continue;
        };
        let Ok(_ply_num) = ply_str.trim().parse::<u32>() else {
            continue;
        };

        let move_token = rest.trim();

        if let Some(o) = resolve_kif_terminal(move_token, board.side, first_mover) {
            if is_mainline {
                outcome = o;
                reason = kif_reason_for(move_token, o);
            }
            accepting = false;
            continue;
        }

        let Some(kif_move) = parse_kif_move(move_token, prev_dest) else {
            warn!(
                path = source_path,
                ply, "unsupported move syntax {move_token:?}, stopping game"
            );
            if is_mainline {
                break;
            }
            accepting = false;
            continue;
        };

        let color = board.side;
        let sfen_before = board.to_sfen();
        let promote = !kif_move.is_drop
            && board
                .piece_at(kif_move.from_file, kif_move.from_rank)
                .map(|(_, p)| p)
                != Some(kif_move.piece);
        let usi_move = if kif_move.is_drop {
            UsiMove::Drop {
                piece: kif_move.piece,
                to_file: kif_move.dest_file,
                to_rank: kif_move.dest_rank,
            }
        } else {
            UsiMove::Normal {
                from_file: kif_move.from_file,
                from_rank: kif_move.from_rank,
                to_file: kif_move.dest_file,
                to_rank: kif_move.dest_rank,
                promote,
            }
        }
        .to_usi_string();

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
            if is_mainline {
                break;
            }
            accepting = false;
            continue;
        }

        prev_dest = Some((kif_move.dest_file, kif_move.dest_rank));
        ply += 1;

        if is_mainline {
            checkpoints.push((board.clone(), prev_dest));
        }

        out.push(RawMove {
            ply,
            mover: color,
            sfen_before,
            usi_move,
            source: SourceInfo {
                kind: "kif".to_string(),
                path: current_path.clone(),
                ply,
                root_id: Some(source_path.to_string()),
                variation_id: current_variation_id.clone(),
                branch_from_ply: current_branch_from_ply,
            },
            // Backfilled below for the mainline prefix once its terminal marker is seen.
            outcome: GameOutcome::Unknown,
        });
        if is_mainline {
            mainline_move_count += 1;
        }
    }

    for mv in out.iter_mut().take(mainline_move_count) {
        mv.outcome = outcome;
    }
    Ok((out, outcome, reason))
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
        let m = parse_kif_move("７六歩(77)", None).unwrap();
        assert_eq!(m.dest_file, 7);
        assert_eq!(m.dest_rank, 6);
        assert_eq!(m.piece, PieceType::Pawn);
        assert!(!m.is_drop);
        assert_eq!(m.from_file, 7);
        assert_eq!(m.from_rank, 7);
    }

    #[test]
    fn parse_promotion_move() {
        let m = parse_kif_move("２二角成(88)", None).unwrap();
        assert_eq!(m.dest_file, 2);
        assert_eq!(m.dest_rank, 2);
        assert_eq!(m.piece, PieceType::Horse); // 角 promoted → Horse
        assert!(!m.is_drop);
        assert_eq!(m.from_file, 8);
        assert_eq!(m.from_rank, 8);
    }

    #[test]
    fn parse_drop_move() {
        let m = parse_kif_move("４五角打", None).unwrap();
        assert_eq!(m.dest_file, 4);
        assert_eq!(m.dest_rank, 5);
        assert_eq!(m.piece, PieceType::Bishop);
        assert!(m.is_drop);
    }

    #[test]
    fn parse_same_square_move() {
        let m = parse_kif_move("同歩(77)", Some((7, 6))).unwrap();
        assert_eq!(m.dest_file, 7);
        assert_eq!(m.dest_rank, 6);
        assert_eq!(m.piece, PieceType::Pawn);
        assert_eq!(m.from_file, 7);
        assert_eq!(m.from_rank, 7);
    }

    #[test]
    fn parse_same_square_move_without_prev_dest_fails() {
        assert!(parse_kif_move("同歩(77)", None).is_none());
    }

    #[test]
    fn resolve_kif_terminal_made_prefix_sennichite_is_draw() {
        assert_eq!(
            resolve_kif_terminal("まで64手で千日手", SideToMove::White, SideToMove::Black),
            Some(GameOutcome::Draw)
        );
    }

    #[test]
    fn resolve_kif_terminal_made_prefix_jishogi_is_draw() {
        assert_eq!(
            resolve_kif_terminal("まで120手で持将棋", SideToMove::Black, SideToMove::Black),
            Some(GameOutcome::Draw)
        );
    }

    #[test]
    fn resolve_kif_terminal_made_prefix_tsumi_resolves_like_resignation() {
        // Mate declared on White's turn -- Black (the opponent) wins, same polarity as 投了.
        assert_eq!(
            resolve_kif_terminal("まで77手で詰み", SideToMove::White, SideToMove::Black),
            Some(GameOutcome::BlackWins)
        );
    }

    #[test]
    fn resolve_kif_terminal_standalone_tsumi_resolves_like_resignation() {
        assert_eq!(
            resolve_kif_terminal("詰み", SideToMove::Black, SideToMove::Black),
            Some(GameOutcome::WhiteWins)
        );
    }

    #[test]
    fn resolve_kif_terminal_made_prefix_unrecognized_suffix_is_unknown() {
        // Still `Some` (a terminal marker was present), not `None` -- distinguishes "seen but
        // undetermined" from "no terminal seen at all" for the reason-code taxonomy.
        // 反則勝ち is deliberately unhandled this round (see plan) -- must still resolve to
        // `Some(Unknown)`, not silently guess a winner.
        assert_eq!(
            resolve_kif_terminal("まで999手で反則勝ち", SideToMove::Black, SideToMove::Black),
            Some(GameOutcome::Unknown)
        );
    }

    #[test]
    fn kif_reason_for_chudan_is_interrupted() {
        assert_eq!(
            kif_reason_for("中断", GameOutcome::Unknown),
            "kif_interrupted"
        );
    }

    #[test]
    fn kif_reason_for_unrecognized_made_suffix_is_terminal_undetermined() {
        assert_eq!(
            kif_reason_for("まで999手で謎", GameOutcome::Unknown),
            "kif_terminal_undetermined"
        );
    }

    #[test]
    fn kif_reason_for_resolved_marker_is_kif_marker() {
        assert_eq!(kif_reason_for("投了", GameOutcome::BlackWins), "kif_marker");
    }

    #[test]
    fn decode_plain_utf8_unchanged() {
        let src = "手合割：平手\n手数----指手\n   1 ７六歩(77)\n";
        assert_eq!(decode_kif_bytes(src.as_bytes()), src);
    }

    #[test]
    fn decode_strips_utf8_bom() {
        let src = "手合割：平手\n手数----指手\n   1 ７六歩(77)\n";
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(src.as_bytes());
        assert_eq!(decode_kif_bytes(&bytes), src);
    }

    #[test]
    fn decode_shift_jis_fallback() {
        let src = "手合割：平手\n手数----指手\n   1 ７六歩(77)\n";
        let (sjis_bytes, _, had_errors) = encoding_rs::SHIFT_JIS.encode(src);
        assert!(!had_errors, "test KIF text must be Shift_JIS-representable");
        assert_eq!(decode_kif_bytes(&sjis_bytes), src);
    }
}
