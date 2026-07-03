use std::fmt;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SideToMove {
    Black,
    White,
}

impl fmt::Display for SideToMove {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SideToMove::Black => write!(f, "black"),
            SideToMove::White => write!(f, "white"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GamePhase {
    Opening,
    Middlegame,
    Endgame,
}

impl fmt::Display for GamePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GamePhase::Opening => write!(f, "opening"),
            GamePhase::Middlegame => write!(f, "middlegame"),
            GamePhase::Endgame => write!(f, "endgame"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilityInfo {
    /// Max minus min of all cp-scored observations. None if fewer than 2 cp observations.
    pub score_swing_cp: Option<i32>,
    /// True when all observations agree on the same bestmove.
    pub bestmove_agreement: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionRecord {
    pub schema_version: u32,
    pub sfen: String,
    pub source: SourceInfo,
    pub tags: PositionTags,
    pub observations: Vec<Observation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stability: Option<StabilityInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    pub kind: String,
    pub path: String,
    pub ply: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionTags {
    pub phase: GamePhase,
    pub side_to_move: SideToMove,
    pub in_check: bool,
    pub has_capture: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Score {
    Cp { value: i32 },
    Mate { moves: i32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub engine: String,
    pub engine_version: Option<String>,
    pub depth: u32,
    pub score: Score,
    pub bestmove: String,
    pub nodes: Option<u64>,
    pub time_ms: Option<u64>,
    pub pv: Option<Vec<String>>,
}

/// Cp swing (max - min) across at least 2 scores; `None` if fewer than 2.
pub fn score_swing(cp_scores: &[i32]) -> Option<i32> {
    if cp_scores.len() < 2 {
        return None;
    }
    let lo = *cp_scores.iter().min().unwrap();
    let hi = *cp_scores.iter().max().unwrap();
    Some(hi - lo)
}

impl PositionRecord {
    pub fn new(sfen: String, source: SourceInfo, tags: PositionTags) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            sfen,
            source,
            tags,
            observations: Vec::new(),
            stability: None,
        }
    }

    /// Compute and populate `self.stability` from current observations.
    pub fn fill_stability(&mut self) {
        if self.observations.is_empty() {
            return;
        }
        let cp_scores: Vec<i32> = self
            .observations
            .iter()
            .filter_map(|o| match o.score {
                Score::Cp { value } => Some(value),
                Score::Mate { .. } => None,
            })
            .collect();
        let first = &self.observations[0].bestmove;
        let bestmove_agreement = self.observations.iter().all(|o| &o.bestmove == first);
        self.stability = Some(StabilityInfo {
            score_swing_cp: score_swing(&cp_scores),
            bestmove_agreement,
        });
    }
}

pub fn phase_from_ply(ply: u32) -> GamePhase {
    match ply {
        0..=20 => GamePhase::Opening,
        21..=100 => GamePhase::Middlegame,
        _ => GamePhase::Endgame,
    }
}

pub mod board;
pub use board::{Board, BoardError, PieceType, zobrist_from_sfen};

pub mod sfen;

/// Shared configuration for position extraction (used by shogiesa-csa and shogiesa-kif).
#[derive(Debug, Clone)]
pub struct ExtractConfig {
    pub min_ply: u32,
    pub max_ply: Option<u32>,
    pub every_n: u32,
    pub dedup: bool,
}

impl Default for ExtractConfig {
    fn default() -> Self {
        Self {
            min_ply: 1,
            max_ply: None,
            every_n: 1,
            dedup: false,
        }
    }
}
