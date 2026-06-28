use std::collections::HashSet;
use std::fs;
use std::io::BufRead;
use std::path::Path;

use csa::{Action, GameRecord};
pub use shogiesa_core::ExtractConfig;
use shogiesa_core::{PositionRecord, PositionTags, SourceInfo, phase_from_ply};
use thiserror::Error;
use tracing::warn;

use crate::board::{apply_csa_action, board_from_csa_position};

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
    let record: GameRecord =
        csa::parse_csa(content).map_err(|e| ExtractError::Csa(format!("{e:?}")))?;

    let mut board = board_from_csa_position(&record.start_pos);
    let mut out = Vec::new();
    let mut ply: u32 = 0;

    for mr in &record.moves {
        if matches!(mr.action, Action::Move(..)) {
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

            let side = board.side;
            // ponytail: in_check and has_capture need move-gen; always false for now
            let tags = PositionTags {
                phase: phase_from_ply(ply),
                side_to_move: side,
                in_check: false,
                has_capture: false,
            };
            let source = SourceInfo {
                kind: "csa".to_string(),
                path: source_path.to_string(),
                ply,
            };
            out.push(PositionRecord::new(sfen, source, tags));
        }
    }

    Ok(out)
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
