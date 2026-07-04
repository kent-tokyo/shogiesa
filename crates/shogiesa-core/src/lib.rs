use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 5;

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

/// Whether a USI `info` line's score is a confirmed evaluation or a search bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScoreBound {
    #[default]
    Exact,
    Lowerbound,
    Upperbound,
}

/// One MultiPV candidate line from a `label --multipv N` (N≥2) pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateMove {
    pub multipv: u32,
    pub bestmove: String,
    pub score: Score,
    #[serde(default)]
    pub score_bound: ScoreBound,
    pub pv: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub engine: String,
    pub engine_version: Option<String>,
    pub depth: u32,
    pub score: Score,
    /// Whether `score` is a confirmed evaluation or a search bound (e.g. an aspiration-window
    /// fail-high/low). Only ever set from the engine's own bestmove line, independent of
    /// whether MultiPV was used — `CandidateMove.score_bound` covers runner-up ranks.
    #[serde(default)]
    pub score_bound: ScoreBound,
    pub bestmove: String,
    pub nodes: Option<u64>,
    pub time_ms: Option<u64>,
    pub pv: Option<Vec<String>>,
    /// `score_cp(bestmove) - score_cp(runner_up)` from a MultiPV≥2 label pass.
    /// `None` when MultiPV wasn't used, either score was a mate score, or the
    /// runner-up's score was a lowerbound/upperbound rather than a confirmed eval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_margin_cp: Option<i32>,
    /// Every MultiPV rank from the search, populated only when the engine was run with
    /// MultiPV≥2 (empty otherwise, matching `policy_margin_cp`'s convention).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<CandidateMove>,
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

/// Reason a position failed one of `QualityConfig`'s gates. `as_str()` values match the
/// strings `filter`'s stderr drop-reason breakdown has always printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityReason {
    MinObservations,
    Phase,
    Mate,
    InCheck,
    Capture,
    EvalMin,
    EvalMax,
    ScoreSwing,
    PolicyMargin,
    BestmoveDisagreement,
    EngineDisagreement,
    EngineScoreSwing,
    NonExactScore,
    MissingPolicyMargin,
    LowActualDepth,
}

impl QualityReason {
    pub fn as_str(self) -> &'static str {
        match self {
            QualityReason::MinObservations => "min_observations",
            QualityReason::Phase => "phase",
            QualityReason::Mate => "mate",
            QualityReason::InCheck => "in_check",
            QualityReason::Capture => "capture",
            QualityReason::EvalMin => "eval_min",
            QualityReason::EvalMax => "eval_max",
            QualityReason::ScoreSwing => "score_swing",
            QualityReason::PolicyMargin => "policy_margin",
            QualityReason::BestmoveDisagreement => "bestmove_disagreement",
            QualityReason::EngineDisagreement => "engine_disagreement",
            QualityReason::EngineScoreSwing => "engine_score_swing",
            QualityReason::NonExactScore => "non_exact_score",
            QualityReason::MissingPolicyMargin => "missing_policy_margin",
            QualityReason::LowActualDepth => "low_actual_depth",
        }
    }
}

/// Configuration for `evaluate_quality`'s gates — the single place `filter`'s pass/fail logic
/// lives, so other consumers can reuse the exact same decision instead of reimplementing it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct QualityConfig {
    pub min_observations: u32,
    pub allowed_phases: Option<Vec<GamePhase>>,
    pub exclude_mate: bool,
    pub exclude_in_check: bool,
    pub exclude_capture: bool,
    pub eval_min: Option<i32>,
    pub eval_max: Option<i32>,
    pub max_score_swing_cp: Option<i32>,
    pub min_policy_margin_cp: Option<i32>,
    pub require_bestmove_agreement: bool,
    pub require_engine_agreement: bool,
    pub max_engine_score_swing_cp: Option<i32>,
    /// Reject a record if any observation's `score` is a search bound rather than a confirmed
    /// evaluation. Independent of `min_policy_margin_cp` (which only checks margins that were
    /// actually computed).
    pub require_exact_score: bool,
    /// Reject a record if no observation has a computed `policy_margin_cp` at all. Unlike
    /// `min_policy_margin_cp` (a no-op when every margin is `None`), this catches "we never
    /// confirmed a margin" rather than only "the margin we confirmed was too small."
    pub require_policy_margin: bool,
    /// Reject a record if any *non-mate* observation's achieved `depth` is below this. Mate
    /// observations are exempt: an engine stopping short of the requested depth is dominantly
    /// caused by finding a forced mate (a confirmed, high-confidence result), not a weak search
    /// -- gating on depth without this exemption would penalize the most reliable observations.
    pub min_depth_reached: Option<u32>,
}

/// Result of evaluating a `PositionRecord` against a `QualityConfig`.
#[derive(Debug, Clone, Serialize)]
pub struct QualityDecision {
    /// True iff `reasons` is empty (every configured gate passed).
    pub keep: bool,
    /// Fraction of the *configured* gates this record passed — a plain, transparent readout of
    /// `reasons`/the gates, not an independently weighted score. `1.0` if nothing was configured.
    pub score: f32,
    /// Every gate this record failed, in the same order the gates are checked below.
    pub reasons: Vec<QualityReason>,
    pub score_swing_cp: Option<i32>,
    /// Vacuously true for 0 or 1 observations, matching `StabilityInfo::fill_stability()`.
    pub bestmove_agreement: bool,
    pub cp_count: usize,
    pub mate_count: usize,
}

/// Evaluate every gate in `config` against `rec`, collecting *all* failing reasons (does not
/// short-circuit on the first failure) so callers can see the complete picture, not just one
/// reason. `filter` takes `reasons.first()` to keep its existing first-reason-only stderr tally.
pub fn evaluate_quality(rec: &PositionRecord, config: &QualityConfig) -> QualityDecision {
    let obs = &rec.observations;
    let mut reasons = Vec::new();
    let mut configured_gates = 0u32;

    if config.min_observations > 0 {
        configured_gates += 1;
        if (obs.len() as u32) < config.min_observations {
            reasons.push(QualityReason::MinObservations);
        }
    }

    if config.allowed_phases.is_some() {
        configured_gates += 1;
        if config
            .allowed_phases
            .as_ref()
            .is_some_and(|p| !p.contains(&rec.tags.phase))
        {
            reasons.push(QualityReason::Phase);
        }
    }

    if config.exclude_mate {
        configured_gates += 1;
        if obs.iter().any(|o| matches!(o.score, Score::Mate { .. })) {
            reasons.push(QualityReason::Mate);
        }
    }

    if config.exclude_in_check {
        configured_gates += 1;
        if rec.tags.in_check {
            reasons.push(QualityReason::InCheck);
        }
    }

    if config.exclude_capture {
        configured_gates += 1;
        if rec.tags.has_capture {
            reasons.push(QualityReason::Capture);
        }
    }

    let cp_scores: Vec<i32> = obs
        .iter()
        .filter_map(|o| match o.score {
            Score::Cp { value } => Some(value),
            Score::Mate { .. } => None,
        })
        .collect();
    let cp_count = cp_scores.len();
    let mate_count = obs.len() - cp_count;

    if config.eval_min.is_some() {
        configured_gates += 1;
        if config
            .eval_min
            .is_some_and(|min| cp_scores.iter().any(|&v| v < min))
        {
            reasons.push(QualityReason::EvalMin);
        }
    }
    if config.eval_max.is_some() {
        configured_gates += 1;
        if config
            .eval_max
            .is_some_and(|max| cp_scores.iter().any(|&v| v > max))
        {
            reasons.push(QualityReason::EvalMax);
        }
    }

    let score_swing_cp = score_swing(&cp_scores);
    if config.max_score_swing_cp.is_some() {
        configured_gates += 1;
        if let Some(max_swing) = config.max_score_swing_cp
            && score_swing_cp.is_some_and(|swing| swing > max_swing)
        {
            reasons.push(QualityReason::ScoreSwing);
        }
    }

    if config.min_policy_margin_cp.is_some() {
        configured_gates += 1;
        if config.min_policy_margin_cp.is_some_and(|min| {
            obs.iter()
                .any(|o| o.policy_margin_cp.is_some_and(|m| m < min))
        }) {
            reasons.push(QualityReason::PolicyMargin);
        }
    }

    if config.require_exact_score {
        configured_gates += 1;
        if obs.iter().any(|o| o.score_bound != ScoreBound::Exact) {
            reasons.push(QualityReason::NonExactScore);
        }
    }

    if config.require_policy_margin {
        configured_gates += 1;
        if !obs.iter().any(|o| o.policy_margin_cp.is_some()) {
            reasons.push(QualityReason::MissingPolicyMargin);
        }
    }

    if let Some(min_depth) = config.min_depth_reached {
        configured_gates += 1;
        if obs
            .iter()
            .any(|o| o.depth < min_depth && !matches!(o.score, Score::Mate { .. }))
        {
            reasons.push(QualityReason::LowActualDepth);
        }
    }

    let bestmove_agreement = obs.is_empty() || obs.iter().all(|o| o.bestmove == obs[0].bestmove);
    if config.require_bestmove_agreement {
        configured_gates += 1;
        if obs.len() >= 2 && !bestmove_agreement {
            reasons.push(QualityReason::BestmoveDisagreement);
        }
    }

    if config.require_engine_agreement {
        configured_gates += 1;
        if engine_bestmove_agreement(obs) == Some(false) {
            reasons.push(QualityReason::EngineDisagreement);
        }
    }

    if config.max_engine_score_swing_cp.is_some() {
        configured_gates += 1;
        if let Some(max_swing) = config.max_engine_score_swing_cp
            && engine_score_swing(obs).is_some_and(|swing| swing > max_swing)
        {
            reasons.push(QualityReason::EngineScoreSwing);
        }
    }

    let score = if configured_gates == 0 {
        1.0
    } else {
        1.0 - reasons.len() as f32 / configured_gates as f32
    };

    QualityDecision {
        keep: reasons.is_empty(),
        score,
        reasons,
        score_swing_cp,
        bestmove_agreement,
        cp_count,
        mate_count,
    }
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
            score_bound: ScoreBound::Exact,
            bestmove: bestmove.to_string(),
            nodes: None,
            time_ms: None,
            pv: None,
            policy_margin_cp: None,
            candidates: Vec::new(),
        }
    }

    fn obs_with_bound(
        engine: &str,
        depth: u32,
        cp: i32,
        bestmove: &str,
        score_bound: ScoreBound,
    ) -> Observation {
        let mut o = obs(engine, depth, cp, bestmove);
        o.score_bound = score_bound;
        o
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

    fn obs_mate(engine: &str, depth: u32, moves: i32, bestmove: &str) -> Observation {
        Observation {
            engine: engine.to_string(),
            engine_version: None,
            depth,
            score: Score::Mate { moves },
            score_bound: ScoreBound::Exact,
            bestmove: bestmove.to_string(),
            nodes: None,
            time_ms: None,
            pv: None,
            policy_margin_cp: None,
            candidates: Vec::new(),
        }
    }

    fn obs_with_margin(
        engine: &str,
        depth: u32,
        cp: i32,
        bestmove: &str,
        margin: i32,
    ) -> Observation {
        let mut o = obs(engine, depth, cp, bestmove);
        o.policy_margin_cp = Some(margin);
        o
    }

    fn rec_with(
        phase: GamePhase,
        in_check: bool,
        has_capture: bool,
        observations: Vec<Observation>,
    ) -> PositionRecord {
        let mut r = PositionRecord::new(
            "startpos".to_string(),
            SourceInfo {
                kind: "test".to_string(),
                path: "test".to_string(),
                ply: 1,
            },
            PositionTags {
                phase,
                side_to_move: SideToMove::Black,
                in_check,
                has_capture,
            },
        );
        r.observations = observations;
        r
    }

    fn simple_rec(observations: Vec<Observation>) -> PositionRecord {
        rec_with(GamePhase::Middlegame, false, false, observations)
    }

    #[test]
    fn evaluate_quality_min_observations_gate() {
        let config = QualityConfig {
            min_observations: 2,
            ..Default::default()
        };
        let decision = evaluate_quality(&simple_rec(vec![obs("a", 4, 10, "7g7f")]), &config);
        assert!(!decision.keep);
        assert_eq!(decision.reasons, vec![QualityReason::MinObservations]);
    }

    #[test]
    fn evaluate_quality_phase_gate() {
        let config = QualityConfig {
            allowed_phases: Some(vec![GamePhase::Opening]),
            ..Default::default()
        };
        let rec = rec_with(
            GamePhase::Endgame,
            false,
            false,
            vec![obs("a", 4, 10, "7g7f")],
        );
        let decision = evaluate_quality(&rec, &config);
        assert_eq!(decision.reasons, vec![QualityReason::Phase]);
    }

    #[test]
    fn evaluate_quality_mate_gate() {
        let config = QualityConfig {
            exclude_mate: true,
            ..Default::default()
        };
        let decision = evaluate_quality(&simple_rec(vec![obs_mate("a", 4, 3, "7g7f")]), &config);
        assert_eq!(decision.reasons, vec![QualityReason::Mate]);
        assert_eq!(decision.mate_count, 1);
        assert_eq!(decision.cp_count, 0);
    }

    #[test]
    fn evaluate_quality_in_check_gate() {
        let config = QualityConfig {
            exclude_in_check: true,
            ..Default::default()
        };
        let rec = rec_with(
            GamePhase::Middlegame,
            true,
            false,
            vec![obs("a", 4, 10, "7g7f")],
        );
        assert_eq!(
            evaluate_quality(&rec, &config).reasons,
            vec![QualityReason::InCheck]
        );
    }

    #[test]
    fn evaluate_quality_capture_gate() {
        let config = QualityConfig {
            exclude_capture: true,
            ..Default::default()
        };
        let rec = rec_with(
            GamePhase::Middlegame,
            false,
            true,
            vec![obs("a", 4, 10, "7g7f")],
        );
        assert_eq!(
            evaluate_quality(&rec, &config).reasons,
            vec![QualityReason::Capture]
        );
    }

    #[test]
    fn evaluate_quality_eval_min_gate() {
        let config = QualityConfig {
            eval_min: Some(-100),
            ..Default::default()
        };
        let decision = evaluate_quality(&simple_rec(vec![obs("a", 4, -200, "7g7f")]), &config);
        assert_eq!(decision.reasons, vec![QualityReason::EvalMin]);
    }

    #[test]
    fn evaluate_quality_eval_max_gate() {
        let config = QualityConfig {
            eval_max: Some(100),
            ..Default::default()
        };
        let decision = evaluate_quality(&simple_rec(vec![obs("a", 4, 200, "7g7f")]), &config);
        assert_eq!(decision.reasons, vec![QualityReason::EvalMax]);
    }

    #[test]
    fn evaluate_quality_score_swing_gate() {
        let config = QualityConfig {
            max_score_swing_cp: Some(50),
            ..Default::default()
        };
        let observations = vec![obs("a", 4, 0, "7g7f"), obs("a", 6, 100, "7g7f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert_eq!(decision.reasons, vec![QualityReason::ScoreSwing]);
        assert_eq!(decision.score_swing_cp, Some(100));
    }

    #[test]
    fn evaluate_quality_policy_margin_gate() {
        let config = QualityConfig {
            min_policy_margin_cp: Some(100),
            ..Default::default()
        };
        let observations = vec![obs_with_margin("a", 4, 10, "7g7f", 20)];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert_eq!(decision.reasons, vec![QualityReason::PolicyMargin]);
    }

    #[test]
    fn evaluate_quality_require_exact_score_rejects_non_exact() {
        let config = QualityConfig {
            require_exact_score: true,
            ..Default::default()
        };
        let observations = vec![obs_with_bound("a", 4, 10, "7g7f", ScoreBound::Lowerbound)];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert_eq!(decision.reasons, vec![QualityReason::NonExactScore]);
    }

    #[test]
    fn evaluate_quality_require_exact_score_passes_exact() {
        let config = QualityConfig {
            require_exact_score: true,
            ..Default::default()
        };
        let observations = vec![obs("a", 4, 10, "7g7f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert!(decision.keep);
    }

    #[test]
    fn evaluate_quality_require_exact_score_passes_mate() {
        // Mate scores are inherently confirmed -- they carry ScoreBound::Exact by convention
        // (USI mate scores don't carry bound tokens), so this gate must not treat them as
        // unconfirmed.
        let config = QualityConfig {
            require_exact_score: true,
            ..Default::default()
        };
        let observations = vec![obs_mate("a", 4, 3, "7g7f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert!(decision.keep);
    }

    #[test]
    fn evaluate_quality_require_policy_margin_rejects_when_none_computed() {
        let config = QualityConfig {
            require_policy_margin: true,
            ..Default::default()
        };
        let observations = vec![obs("a", 4, 10, "7g7f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert_eq!(decision.reasons, vec![QualityReason::MissingPolicyMargin]);
    }

    #[test]
    fn evaluate_quality_require_policy_margin_passes_when_computed() {
        let config = QualityConfig {
            require_policy_margin: true,
            ..Default::default()
        };
        let observations = vec![obs_with_margin("a", 4, 10, "7g7f", 20)];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert!(decision.keep);
    }

    #[test]
    fn evaluate_quality_min_depth_reached_rejects_shallow_non_mate() {
        let config = QualityConfig {
            min_depth_reached: Some(10),
            ..Default::default()
        };
        let observations = vec![obs("a", 6, 10, "7g7f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert_eq!(decision.reasons, vec![QualityReason::LowActualDepth]);
    }

    #[test]
    fn evaluate_quality_min_depth_reached_exempts_shallow_mate() {
        // A shallow depth is dominantly caused by the engine finding a forced mate -- a
        // confirmed, high-confidence result, not a weak search. This gate must not penalize it.
        let config = QualityConfig {
            min_depth_reached: Some(10),
            ..Default::default()
        };
        let observations = vec![obs_mate("a", 6, 3, "7g7f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert!(decision.keep);
    }

    #[test]
    fn evaluate_quality_min_depth_reached_passes_deep_enough() {
        let config = QualityConfig {
            min_depth_reached: Some(10),
            ..Default::default()
        };
        let observations = vec![obs("a", 10, 10, "7g7f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert!(decision.keep);
    }

    #[test]
    fn evaluate_quality_bestmove_disagreement_gate() {
        let config = QualityConfig {
            require_bestmove_agreement: true,
            ..Default::default()
        };
        let observations = vec![obs("a", 4, 10, "7g7f"), obs("a", 6, 12, "2g2f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert_eq!(decision.reasons, vec![QualityReason::BestmoveDisagreement]);
        assert!(!decision.bestmove_agreement);
    }

    #[test]
    fn evaluate_quality_engine_disagreement_gate() {
        let config = QualityConfig {
            require_engine_agreement: true,
            ..Default::default()
        };
        let observations = vec![obs("a", 4, 10, "7g7f"), obs("b", 4, 12, "2g2f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert_eq!(decision.reasons, vec![QualityReason::EngineDisagreement]);
    }

    #[test]
    fn evaluate_quality_engine_score_swing_gate() {
        let config = QualityConfig {
            max_engine_score_swing_cp: Some(50),
            ..Default::default()
        };
        let observations = vec![obs("a", 4, 0, "7g7f"), obs("b", 4, 100, "7g7f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert_eq!(decision.reasons, vec![QualityReason::EngineScoreSwing]);
    }

    #[test]
    fn evaluate_quality_multi_gate_failure_keeps_first_reason_order() {
        // Fails both min_observations (needs 2, has 1) and exclude_mate — min_observations
        // must appear first, matching filter_reason's existing check order, since `filter`
        // takes reasons.first() for its stderr tally.
        let config = QualityConfig {
            min_observations: 2,
            exclude_mate: true,
            ..Default::default()
        };
        let decision = evaluate_quality(&simple_rec(vec![obs_mate("a", 4, 3, "7g7f")]), &config);
        assert_eq!(
            decision.reasons,
            vec![QualityReason::MinObservations, QualityReason::Mate]
        );
    }

    #[test]
    fn evaluate_quality_zero_observations_is_safe() {
        // With nothing short-circuiting anymore, every gate runs against empty `observations` —
        // must not panic (e.g. indexing obs[0]) and must report vacuous/empty stats.
        let config = QualityConfig {
            require_bestmove_agreement: true,
            require_engine_agreement: true,
            exclude_mate: true,
            max_score_swing_cp: Some(10),
            ..Default::default()
        };
        let decision = evaluate_quality(&simple_rec(vec![]), &config);
        assert!(decision.bestmove_agreement);
        assert_eq!(decision.cp_count, 0);
        assert_eq!(decision.mate_count, 0);
        assert_eq!(decision.score_swing_cp, None);
    }

    #[test]
    fn evaluate_quality_score_formula() {
        // 3 gates configured (min_observations, exclude_mate, max_score_swing_cp), 1 fails
        // (exclude_mate) -> score == 2/3.
        let config = QualityConfig {
            min_observations: 1,
            exclude_mate: true,
            max_score_swing_cp: Some(1000),
            ..Default::default()
        };
        let decision = evaluate_quality(&simple_rec(vec![obs_mate("a", 4, 3, "7g7f")]), &config);
        assert_eq!(decision.reasons, vec![QualityReason::Mate]);
        assert!((decision.score - 2.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn evaluate_quality_no_configured_gates_scores_perfect() {
        let decision = evaluate_quality(&simple_rec(vec![]), &QualityConfig::default());
        assert!(decision.keep);
        assert_eq!(decision.score, 1.0);
    }

    #[test]
    fn quality_reason_serializes_to_same_string_as_as_str() {
        for reason in [
            QualityReason::MinObservations,
            QualityReason::EngineScoreSwing,
            QualityReason::NonExactScore,
            QualityReason::MissingPolicyMargin,
            QualityReason::LowActualDepth,
        ] {
            let serialized = serde_json::to_string(&reason).unwrap();
            assert_eq!(serialized, format!("\"{}\"", reason.as_str()));
        }
    }
}
