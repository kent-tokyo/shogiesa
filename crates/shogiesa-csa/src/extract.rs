use std::collections::HashSet;
use std::fs;
use std::io::BufRead;
use std::path::Path;

use csa::{Action, GameRecord};
pub use shogiesa_core::ExtractConfig;
use shogiesa_core::{
    GameOutcome, GameResultInfo, PositionRecord, PositionTags, RawMove, SourceInfo, phase_from_ply,
};
use thiserror::Error;
use tracing::warn;

use crate::board::{apply_csa_action, board_from_csa_position, from_csa_color, from_csa_piece};

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("CSA parse: {0}")]
    Csa(String),
}

impl From<shogiesa_core::BoardError> for ExtractError {
    fn from(e: shogiesa_core::BoardError) -> Self {
        ExtractError::Csa(e.to_string())
    }
}

pub fn extract_from_str(
    content: &str,
    source_path: &str,
    config: &ExtractConfig,
    seen: &mut HashSet<String>,
) -> Result<Vec<PositionRecord>, ExtractError> {
    // Resolved via a full-game walk (see `extract_moves_from_str`) rather than re-derived here,
    // so game-result resolution can't drift out of sync with -- or, worse, invert the winner of --
    // the one already-tested implementation. Independent of `config.max_ply`: the terminal action
    // this needs to see can be well past whatever ply the position-emitting loop below truncates
    // at.
    //
    // `extract_moves_from_str` propagates a mid-game board error via `?` (unlike this function's
    // own loop below, which warns-and-breaks to keep whatever positions it collected so far) --
    // must not let that abort this whole extraction, or a structurally-valid CSA file that hits
    // an illegal/inconsistent move partway through would silently yield zero positions instead of
    // the ones before the break. Degrade to `Unknown` provenance instead; `csa::parse_csa` below
    // still surfaces a genuine structural parse error.
    let (outcome, result_source) = match extract_moves_from_str(content, source_path) {
        Ok((_, o, reason)) => (o, reason),
        Err(_) => (GameOutcome::Unknown, "csa_walk_error"),
    };

    let record: GameRecord =
        csa::parse_csa(content).map_err(|e| ExtractError::Csa(format!("{e:?}")))?;

    let mut board = board_from_csa_position(&record.start_pos);
    let mut out = Vec::new();
    let mut ply: u32 = 0;

    for mr in &record.moves {
        if let Action::Move(csa_color, from, to, _) = mr.action {
            let mover = from_csa_color(csa_color);
            let has_capture = from.file != 0 && board.is_capture(to.file, to.rank, mover);

            if let Err(e) = apply_csa_action(&mut board, mr.action) {
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

            let in_check = board.is_in_check();
            let tags = PositionTags {
                phase: phase_from_ply(ply),
                side_to_move: board.side,
                in_check,
                has_capture,
            };
            let source = SourceInfo {
                kind: "csa".to_string(),
                path: source_path.to_string(),
                ply,
                // CSA extraction has no variation concept -- split falls back to path-based
                // grouping for these records (see split_root_path in shogiesa-cli).
                root_id: None,
                variation_id: None,
                branch_from_ply: None,
            };
            let mut rec = PositionRecord::new(sfen, source, tags);
            rec.game_result = Some(GameResultInfo {
                outcome,
                result_source: result_source.to_string(),
            });
            out.push(rec);
        }
    }

    Ok(out)
}

/// Unfiltered full-game walk producing one `RawMove` per ply, for lineprior-style export. Unlike
/// `extract_from_str`, this captures the SFEN BEFORE each move (not after) and the USI move
/// token itself, and applies no `min_ply`/`every_n`/`dedup` filtering -- every ply is needed for
/// correct before/after pairing, and outcome resolution needs the whole game regardless of any
/// caller-side ply truncation. CSA has no variation concept, so every returned move shares the
/// same resolved `outcome`.
///
/// Also returns the resolved `GameOutcome` directly (not just backfilled onto each `RawMove`),
/// since `extract_from_str` needs it even when this game has zero moves before its terminal
/// action -- a case where `out` would otherwise be empty and the outcome would be lost.
pub fn extract_moves_from_str(
    content: &str,
    source_path: &str,
) -> Result<(Vec<RawMove>, GameOutcome, &'static str), ExtractError> {
    let record: GameRecord =
        csa::parse_csa(content).map_err(|e| ExtractError::Csa(format!("{e:?}")))?;

    let mut board = board_from_csa_position(&record.start_pos);
    let mut out = Vec::new();
    let mut ply: u32 = 0;
    let mut outcome = GameOutcome::Unknown;
    let mut reason: &'static str = "csa_no_terminal";

    for mr in &record.moves {
        match mr.action {
            Action::Move(csa_color, from, to, to_pt) => {
                let mover = from_csa_color(csa_color);
                let sfen_before = board.to_sfen();
                let to_piece = from_csa_piece(to_pt).ok_or(ExtractError::Csa(
                    "move to an unrepresentable piece type".to_string(),
                ))?;
                let promote = from.file != 0
                    && board.piece_at(from.file, from.rank).map(|(_, p)| p) != Some(to_piece);
                let usi_move = if from.file == 0 {
                    // Drops are always a base (unpromoted) piece type by rule.
                    shogiesa_core::UsiMove::Drop {
                        piece: to_piece,
                        to_file: to.file,
                        to_rank: to.rank,
                    }
                } else {
                    shogiesa_core::UsiMove::Normal {
                        from_file: from.file,
                        from_rank: from.rank,
                        to_file: to.file,
                        to_rank: to.rank,
                        promote,
                    }
                }
                .to_usi_string();

                apply_csa_action(&mut board, mr.action)?;
                ply += 1;

                out.push(RawMove {
                    ply,
                    mover,
                    sfen_before,
                    usi_move,
                    source: SourceInfo {
                        kind: "csa".to_string(),
                        path: source_path.to_string(),
                        ply,
                        root_id: None,
                        variation_id: None,
                        branch_from_ply: None,
                    },
                    // Backfilled below once the terminal action (if any) is seen.
                    outcome: GameOutcome::Unknown,
                });
            }
            terminal => {
                // `board.side` is whoever's turn it is right here == the mover of this terminal
                // action (every variant but `IllegalAction` is silent about its own color).
                outcome = resolve_csa_outcome(terminal, board.side);
                reason = match terminal {
                    Action::Chudan => "csa_interrupted",
                    Action::Matta | Action::Fuzumi | Action::Error => "csa_terminal_undetermined",
                    _ => "csa_terminal",
                };
            }
        }
    }

    for mv in &mut out {
        mv.outcome = outcome;
    }
    Ok((out, outcome, reason))
}

fn resolve_csa_outcome(action: Action, side_to_move: shogiesa_core::SideToMove) -> GameOutcome {
    use shogiesa_core::SideToMove;
    let wins = |color: SideToMove| match color {
        SideToMove::Black => GameOutcome::BlackWins,
        SideToMove::White => GameOutcome::WhiteWins,
    };
    let opponent = |color: SideToMove| match color {
        SideToMove::Black => SideToMove::White,
        SideToMove::White => SideToMove::Black,
    };
    match action {
        Action::Toryo | Action::TimeUp | Action::IllegalMove | Action::Tsumi => {
            wins(opponent(side_to_move))
        }
        Action::Kachi => wins(side_to_move),
        Action::IllegalAction(color) => wins(opponent(from_csa_color(color))),
        Action::Sennichite | Action::Hikiwake | Action::Jishogi => GameOutcome::Draw,
        Action::Chudan | Action::Matta | Action::Fuzumi | Action::Error => GameOutcome::Unknown,
        Action::Move(..) => unreachable!("handled by the Move arm above"),
    }
}

pub fn extract_from_path(
    path: &Path,
    config: &ExtractConfig,
    seen: &mut HashSet<String>,
) -> Result<Vec<PositionRecord>, ExtractError> {
    let content = fs::read_to_string(path)?;
    let source = path.to_string_lossy().into_owned();
    extract_from_str(&content, &source, config, seen)
}

pub fn extract_from_reader(
    mut reader: impl BufRead,
    source_path: &str,
    config: &ExtractConfig,
    seen: &mut HashSet<String>,
) -> Result<Vec<PositionRecord>, ExtractError> {
    let mut content = String::new();
    reader.read_to_string(&mut content)?;
    extract_from_str(&content, source_path, config, seen)
}
