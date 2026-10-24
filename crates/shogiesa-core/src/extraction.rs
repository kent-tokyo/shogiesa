use crate::*;

impl PositionRecord {
    pub fn new(sfen: String, source: SourceInfo, tags: PositionTags) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            sfen,
            source,
            tags,
            observations: Vec::new(),
            stability: None,
            game_result: None,
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
        let bestmove_agreement = bestmove_agreement(&self.observations);
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
