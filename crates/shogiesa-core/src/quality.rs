use serde::{Deserialize, Serialize};

use crate::*;

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
    TimeoutSalvaged,
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
    RequestedDepthNotReached,
}

impl QualityReason {
    pub fn as_str(self) -> &'static str {
        match self {
            QualityReason::MinObservations => "min_observations",
            QualityReason::Phase => "phase",
            QualityReason::Mate => "mate",
            QualityReason::InCheck => "in_check",
            QualityReason::Capture => "capture",
            QualityReason::TimeoutSalvaged => "timeout_salvaged",
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
            QualityReason::RequestedDepthNotReached => "requested_depth_not_reached",
        }
    }
}

/// Configuration for `evaluate_quality`'s gates — the single place `filter`'s pass/fail logic
/// lives, so other consumers can reuse the exact same decision instead of reimplementing it.
/// Derives `Deserialize` (not just `Serialize`) so a `tune --preset-out` JSON file's resolved
/// config can round-trip straight into `filter --preset` without a second hand-written mapping.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QualityConfig {
    pub min_observations: u32,
    pub allowed_phases: Option<Vec<GamePhase>>,
    pub exclude_mate: bool,
    pub exclude_in_check: bool,
    pub exclude_capture: bool,
    /// Reject a record if any observation is a timeout-salvaged, degraded-but-real result (see
    /// `Observation::was_timeout_salvaged`) rather than a full completion or a genuine
    /// engine-initiated early stop.
    pub exclude_timeout_salvaged: bool,
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
    /// Reject a record if any *non-mate* observation's `requested_depth` is `Some` and the
    /// achieved `depth` fell short of it. Unlike `min_depth_reached` (a fixed floor the caller
    /// picks), this checks each observation against the depth *it itself* was asked to reach —
    /// which matters once caching or incremental re-labeling means different observations in the
    /// same dataset were requested to different depths. Mate is exempt for the same reason as
    /// `min_depth_reached`: a forced mate found short of the requested depth is a confirmed,
    /// high-confidence result, not a weak search.
    ///
    /// Exception: a timeout-salvaged observation (`was_timeout_salvaged`) carrying an unconfirmed
    /// mate score is *not* granted this exemption by default -- it never actually confirmed the
    /// mate the way a genuine engine-initiated early stop does. Set `allow_timeout_salvaged_mate`
    /// to restore the blanket exemption for that case too.
    pub require_requested_depth_reached: bool,
    /// When `false` (the default), a timeout-salvaged observation carrying a mate score does
    /// *not* get `require_requested_depth_reached`'s usual mate exemption -- it's less
    /// trustworthy than a genuine early-stop-on-forced-mate. Set `true` to restore the blanket
    /// exemption for salvaged mates too. No effect unless `require_requested_depth_reached` is
    /// also set.
    pub allow_timeout_salvaged_mate: bool,
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

    if config.exclude_timeout_salvaged {
        configured_gates += 1;
        if obs.iter().any(|o| o.was_timeout_salvaged) {
            reasons.push(QualityReason::TimeoutSalvaged);
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

    // Why a separate vector, not just reusing `cp_scores` from above: `cp_scores` also feeds
    // `score_swing_cp` (max - min) below, which is invariant under a uniform per-record sign flip
    // (every observation in one record shares the same `side_to_move`) and must stay
    // side-to-move-relative -- only `eval_min`/`eval_max`, which compare against a fixed
    // black-perspective threshold, need normalization.
    if config.eval_min.is_some() || config.eval_max.is_some() {
        let black_cp_scores: Vec<i64> = obs
            .iter()
            .filter_map(|o| match o.score {
                Score::Cp { value } => Some(cp_from_black_perspective_i64(
                    value,
                    o.score_perspective,
                    rec.tags.side_to_move,
                )),
                Score::Mate { .. } => None,
            })
            .collect();
        if config.eval_min.is_some() {
            configured_gates += 1;
            if config
                .eval_min
                .is_some_and(|min| black_cp_scores.iter().any(|&v| v < i64::from(min)))
            {
                reasons.push(QualityReason::EvalMin);
            }
        }
        if config.eval_max.is_some() {
            configured_gates += 1;
            if config
                .eval_max
                .is_some_and(|max| black_cp_scores.iter().any(|&v| v > i64::from(max)))
            {
                reasons.push(QualityReason::EvalMax);
            }
        }
    }

    let score_swing_cp_wide = score_swing_i64(&cp_scores);
    let score_swing_cp = score_swing_cp_wide.map(crate::score::saturating_cp_i32);
    if config.max_score_swing_cp.is_some() {
        configured_gates += 1;
        if let Some(max_swing) = config.max_score_swing_cp
            && score_swing_cp_wide.is_some_and(|swing| swing > i64::from(max_swing))
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

    if config.require_requested_depth_reached {
        configured_gates += 1;
        if obs.iter().any(|o| {
            requested_depth_underreached(o)
                || (!config.allow_timeout_salvaged_mate
                    && o.was_timeout_salvaged
                    && matches!(o.score, Score::Mate { .. })
                    && o.requested_depth.is_some_and(|rd| o.depth < rd))
        }) {
            reasons.push(QualityReason::RequestedDepthNotReached);
        }
    }

    let bestmove_agreement = bestmove_agreement(obs);
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
            && engine_score_swing_i64(obs).is_some_and(|swing| swing > i64::from(max_swing))
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
