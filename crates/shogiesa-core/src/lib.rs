use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 3;

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
    /// True when every distinct engine's deepest observation agrees on bestmove.
    /// `None` if fewer than 2 distinct engines are represented.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_bestmove_agreement: Option<bool>,
    /// Cp swing across each distinct engine's deepest observation.
    /// `None` if fewer than 2 engines have a cp-scored observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_score_swing_cp: Option<i32>,
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
    /// `score_cp(bestmove) - score_cp(runner_up)` from a MultiPV≥2 label pass.
    /// `None` when MultiPV wasn't used, either score was a mate score, or the
    /// runner-up's score was a lowerbound/upperbound rather than a confirmed eval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_margin_cp: Option<i32>,
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

/// Each distinct engine's deepest observation, taken as that engine's "vote". Engines
/// searched to different depths are compared at their respective best-available answers, so a
/// depth mismatch between engines can itself surface as disagreement — that's intentional, not
/// a limitation to fix.
fn deepest_per_engine(observations: &[Observation]) -> Vec<&Observation> {
    let mut by_engine: HashMap<&str, &Observation> = HashMap::new();
    for obs in observations {
        by_engine
            .entry(obs.engine.as_str())
            .and_modify(|existing| {
                if obs.depth > existing.depth {
                    *existing = obs;
                }
            })
            .or_insert(obs);
    }
    by_engine.into_values().collect()
}

/// Whether every distinct engine's deepest observation agrees on bestmove.
/// `None` if fewer than 2 distinct engines are represented in `observations`.
pub fn engine_bestmove_agreement(observations: &[Observation]) -> Option<bool> {
    let deepest = deepest_per_engine(observations);
    if deepest.len() < 2 {
        return None;
    }
    let first = deepest[0].bestmove.as_str();
    Some(deepest.iter().all(|o| o.bestmove == first))
}

/// Cp swing across each distinct engine's deepest observation.
/// `None` if fewer than 2 engines have a cp-scored observation.
pub fn engine_score_swing(observations: &[Observation]) -> Option<i32> {
    let deepest = deepest_per_engine(observations);
    let cp: Vec<i32> = deepest
        .iter()
        .filter_map(|o| match o.score {
            Score::Cp { value } => Some(value),
            Score::Mate { .. } => None,
        })
        .collect();
    score_swing(&cp)
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
            engine_bestmove_agreement: engine_bestmove_agreement(&self.observations),
            engine_score_swing_cp: engine_score_swing(&self.observations),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(engine: &str, depth: u32, cp: i32, bestmove: &str) -> Observation {
        Observation {
            engine: engine.to_string(),
            engine_version: None,
            depth,
            score: Score::Cp { value: cp },
            bestmove: bestmove.to_string(),
            nodes: None,
            time_ms: None,
            pv: None,
            policy_margin_cp: None,
        }
    }

    #[test]
    fn engine_bestmove_agreement_none_with_one_engine() {
        let observations = vec![obs("a", 4, 10, "7g7f"), obs("a", 6, 12, "7g7f")];
        assert_eq!(engine_bestmove_agreement(&observations), None);
    }

    #[test]
    fn engine_bestmove_agreement_true_when_engines_agree() {
        let observations = vec![obs("a", 4, 10, "7g7f"), obs("b", 4, 12, "7g7f")];
        assert_eq!(engine_bestmove_agreement(&observations), Some(true));
    }

    #[test]
    fn engine_bestmove_agreement_false_when_engines_disagree() {
        let observations = vec![obs("a", 4, 10, "7g7f"), obs("b", 4, 12, "2g2f")];
        assert_eq!(engine_bestmove_agreement(&observations), Some(false));
    }

    #[test]
    fn engine_bestmove_agreement_uses_deepest_observation_per_engine() {
        // engine "a"'s deepest observation (depth 6) disagrees with "b", even though its
        // shallower depth-4 observation happened to agree.
        let observations = vec![
            obs("a", 4, 10, "7g7f"),
            obs("a", 6, 15, "2g2f"),
            obs("b", 4, 12, "7g7f"),
        ];
        assert_eq!(engine_bestmove_agreement(&observations), Some(false));
    }

    #[test]
    fn engine_score_swing_uses_deepest_observation_per_engine() {
        let observations = vec![
            obs("a", 4, 0, "7g7f"),
            obs("a", 6, 100, "7g7f"),
            obs("b", 4, 40, "7g7f"),
        ];
        // swing should be computed from a's depth-6 score (100) and b's depth-4 score (40),
        // not a's shallower depth-4 score (0).
        assert_eq!(engine_score_swing(&observations), Some(60));
    }

    #[test]
    fn engine_score_swing_none_with_one_engine() {
        let observations = vec![obs("a", 4, 10, "7g7f"), obs("a", 6, 20, "7g7f")];
        assert_eq!(engine_score_swing(&observations), None);
    }
}
