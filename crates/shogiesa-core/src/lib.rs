use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 9;

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
    /// Identifier shared by a game's mainline and every variation branching from it (e.g. the
    /// mainline's own `path`). `None` on records from extractors that don't produce variations
    /// (CSA) or on JSONL predating this field. Lets `split` group a mainline with its variations
    /// without depending on `path`'s `#varN@ply` suffix convention.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_id: Option<String>,
    /// This branch's identifier (e.g. `"var1"`) among its mainline's variations. `None` on the
    /// mainline itself and on records with no variation concept at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variation_id: Option<String>,
    /// The mainline ply this variation branched from. `None` on the mainline itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_from_ply: Option<u32>,
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

/// Which side `Score::Cp`'s sign is relative to. USI's `info score cp` is side-to-move-relative
/// by protocol convention (positive = good for whoever is on move), and `shogiesa-usi` stores
/// that raw value with no sign-flipping -- so every `Observation` produced before this field
/// existed, and every one produced by `label` today, is implicitly `SideToMove`. This field makes
/// that convention explicit in the schema instead of leaving it an undocumented assumption that
/// every consumer has to independently get right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScorePerspective {
    #[default]
    SideToMove,
    Black,
}

/// Classifies a `bestmove` response into "no move at all" categories, distinct from an ordinary
/// move string. USI defines `resign`/`win`/`none` as literal non-move tokens; comparing them as
/// if they were moves (as plain `bestmove` string equality does) conflates "the engine considers
/// the position decided" with "the engines disagree on the best move" -- two different things
/// that `bestmove_agreement`-style checks need to tell apart. `NoMove` names the literal USI
/// token `"none"`, distinct from Rust's `Option::None` (which instead means "an ordinary move,
/// no special handling needed" on `Observation.bestmove_kind` below).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BestMoveKind {
    Resign,
    Win,
    NoMove,
}

/// Classifies a literal USI `bestmove` token, `None` for an ordinary move string. The single
/// source of truth for the resign/win/none token set -- `shogiesa-usi`'s live classification and
/// `effective_bestmove_kind`'s legacy-JSONL fallback below both call this instead of each keeping
/// their own copy of the match.
pub fn classify_bestmove_token(token: &str) -> Option<BestMoveKind> {
    match token {
        "resign" => Some(BestMoveKind::Resign),
        "win" => Some(BestMoveKind::Win),
        "none" => Some(BestMoveKind::NoMove),
        _ => None,
    }
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
    /// The depth `label` asked the engine to search to, distinct from `depth` (what it actually
    /// reached) — an engine can stop early on a forced mate. `None` on records labeled before
    /// this field existed. Lets `require_requested_depth_reached` tell "requested 12, reached 8"
    /// apart from "requested 8, reached 8", which `min_depth_reached` alone cannot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_depth: Option<u32>,
    pub score: Score,
    /// Which side `score`'s cp sign is relative to. `#[serde(default)]` without
    /// `skip_serializing_if`, unlike this struct's other optional fields -- every future
    /// observation should self-describe its perspective explicitly rather than leave a reader to
    /// assume the default forever. Old JSONL still parses unchanged (absent → `SideToMove`,
    /// exactly what that data always meant).
    #[serde(default)]
    pub score_perspective: ScorePerspective,
    /// Whether `score` is a confirmed evaluation or a search bound (e.g. an aspiration-window
    /// fail-high/low). Only ever set from the engine's own bestmove line, independent of
    /// whether MultiPV was used — `CandidateMove.score_bound` covers runner-up ranks.
    #[serde(default)]
    pub score_bound: ScoreBound,
    pub bestmove: String,
    /// Set only when `bestmove` is a special USI token (`resign`/`win`/`none`) rather than an
    /// ordinary move -- `None` for the common case, matching this struct's convention for
    /// `policy_margin_cp`/`candidates`. `None` on records labeled before this field existed, even
    /// if `bestmove` happens to hold one of those literal strings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bestmove_kind: Option<BestMoveKind>,
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
    /// `true` when `--timeout-ms` elapsed before `bestmove` arrived and this observation is the
    /// degraded-but-real result salvaged from the last `info` line rather than a normal
    /// completion or the engine's own early stop (e.g. a forced mate). `false` on records
    /// labeled before this field existed -- a genuine no-op on all pre-v9 data, since the new
    /// `exclude_timeout_salvaged` gate only ever fires on data re-labeled with this field set.
    #[serde(default)]
    pub was_timeout_salvaged: bool,
}

/// `cp`, converted to Black's perspective (positive = good for Black), given the perspective it
/// was actually recorded in and which side was to move. USI's `score cp` is side-to-move-relative
/// by protocol convention -- this is the one place that convention gets undone, so every consumer
/// wanting "is Black winning" (not "is whoever's turn it is winning") calls this instead of
/// re-deriving the sign flip independently, which matters once eval-range gates/buckets/
/// histograms need one shared reference frame regardless of whose turn a position was.
pub fn cp_from_black_perspective(
    cp: i32,
    perspective: ScorePerspective,
    side_to_move: SideToMove,
) -> i32 {
    match perspective {
        ScorePerspective::Black => cp,
        ScorePerspective::SideToMove => {
            if side_to_move == SideToMove::Black {
                cp
            } else {
                -cp
            }
        }
    }
}

/// Inverse of `cp_from_black_perspective` -- converts to side-to-move-relative regardless of how
/// `cp` was actually recorded. Forward-compatible with a future `ScorePerspective::Black`-tagged
/// observation, not just today's all-`SideToMove` data.
pub fn cp_from_side_to_move_perspective(
    cp: i32,
    perspective: ScorePerspective,
    side_to_move: SideToMove,
) -> i32 {
    match perspective {
        ScorePerspective::SideToMove => cp,
        ScorePerspective::Black => {
            if side_to_move == SideToMove::Black {
                cp
            } else {
                -cp
            }
        }
    }
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

/// `obs.bestmove_kind` if set, else classifies the literal `bestmove` string -- so older JSONL
/// (labeled before `bestmove_kind` existed) gets the same resign/win/none handling as freshly
/// labeled data, instead of only benefiting records re-labeled after this existed.
pub fn effective_bestmove_kind(obs: &Observation) -> Option<BestMoveKind> {
    obs.bestmove_kind
        .or_else(|| classify_bestmove_token(&obs.bestmove))
}

/// True if any observation's bestmove is a special token (resign/win/none) rather than an
/// ordinary move.
pub fn has_special_bestmove(observations: &[Observation]) -> bool {
    observations
        .iter()
        .any(|o| effective_bestmove_kind(o).is_some())
}

/// Whether `obs` fell short of the depth `label` asked it to reach, excluding mate results (a
/// forced mate found short of the requested depth is a confirmed, high-confidence result, not a
/// weak search -- same exemption `evaluate_quality`'s `require_requested_depth_reached` gate
/// applies). `false` when `requested_depth` is `None` (legacy pre-schema-v6 data, or an
/// observation `label` wasn't asked to reach a specific depth).
pub fn requested_depth_underreached(obs: &Observation) -> bool {
    obs.requested_depth.is_some_and(|rd| obs.depth < rd) && !matches!(obs.score, Score::Mate { .. })
}

/// Bestmove agreement over an iterator of observations, considering only ordinary moves --
/// shared by `bestmove_agreement` (all observations) and `engine_bestmove_agreement` (one
/// per-engine "vote" each). Vacuously true when fewer than 2 ordinary-move observations remain,
/// matching this codebase's existing "agreement" convention for 0-or-1-observation records.
fn ordinary_bestmove_agreement<'a>(observations: impl Iterator<Item = &'a Observation>) -> bool {
    let ordinary: Vec<&Observation> = observations
        .filter(|o| effective_bestmove_kind(o).is_none())
        .collect();
    ordinary.len() < 2 || ordinary.iter().all(|o| o.bestmove == ordinary[0].bestmove)
}

/// Whether every observation agrees on bestmove, excluding special tokens (resign/win/none) from
/// the comparison -- one engine giving up isn't an opinion about which move is best, so folding a
/// resign in as "disagreement" would corrupt agreement-based gates with false positives unrelated
/// to actual position ambiguity.
pub fn bestmove_agreement(observations: &[Observation]) -> bool {
    ordinary_bestmove_agreement(observations.iter())
}

/// Whether every distinct engine's deepest observation agrees on bestmove, excluding special
/// tokens the same way `bestmove_agreement` does.
/// `None` if fewer than 2 distinct engines are represented in `observations`.
pub fn engine_bestmove_agreement(observations: &[Observation]) -> Option<bool> {
    let deepest = deepest_per_engine(observations);
    if deepest.len() < 2 {
        return None;
    }
    Some(ordinary_bestmove_agreement(deepest.into_iter()))
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
        let black_cp_scores: Vec<i32> = obs
            .iter()
            .filter_map(|o| match o.score {
                Score::Cp { value } => Some(cp_from_black_perspective(
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
                .is_some_and(|min| black_cp_scores.iter().any(|&v| v < min))
            {
                reasons.push(QualityReason::EvalMin);
            }
        }
        if config.eval_max.is_some() {
            configured_gates += 1;
            if config
                .eval_max
                .is_some_and(|max| black_cp_scores.iter().any(|&v| v > max))
            {
                reasons.push(QualityReason::EvalMax);
            }
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

pub mod board;
pub use board::{
    Board, BoardError, PieceType, UsiMove, UsiMoveError, parse_usi_move, zobrist_from_sfen,
};

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
            requested_depth: None,
            score: Score::Cp { value: cp },
            score_perspective: ScorePerspective::SideToMove,
            score_bound: ScoreBound::Exact,
            bestmove: bestmove.to_string(),
            bestmove_kind: None,
            nodes: None,
            time_ms: None,
            pv: None,
            policy_margin_cp: None,
            candidates: Vec::new(),
            was_timeout_salvaged: false,
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
    fn bestmove_agreement_true_when_all_observations_are_resign() {
        // legacy JSONL: bestmove_kind absent, falls back to classifying the literal string
        let observations = vec![obs("a", 4, 10, "resign"), obs("b", 4, 12, "resign")];
        assert!(bestmove_agreement(&observations));
    }

    #[test]
    fn bestmove_agreement_true_when_one_resigns_and_one_moves() {
        // vacuous: only one ordinary-move observation remains after excluding the resign
        let observations = vec![obs("a", 4, 10, "resign"), obs("b", 4, 12, "7g7f")];
        assert!(bestmove_agreement(&observations));
    }

    #[test]
    fn bestmove_agreement_false_when_two_ordinary_moves_differ() {
        let observations = vec![
            obs("a", 4, 10, "resign"),
            obs("b", 4, 12, "7g7f"),
            obs("c", 4, 12, "2g2f"),
        ];
        assert!(!bestmove_agreement(&observations));
    }

    #[test]
    fn has_special_bestmove_true_when_any_observation_is_special() {
        let observations = vec![obs("a", 4, 10, "7g7f"), obs("b", 4, 12, "resign")];
        assert!(has_special_bestmove(&observations));
    }

    #[test]
    fn has_special_bestmove_false_for_ordinary_moves_only() {
        let observations = vec![obs("a", 4, 10, "7g7f"), obs("b", 4, 12, "2g2f")];
        assert!(!has_special_bestmove(&observations));
    }

    #[test]
    fn effective_bestmove_kind_prefers_explicit_field_over_literal_string() {
        let mut o = obs("a", 4, 10, "not_actually_resign");
        o.bestmove_kind = Some(BestMoveKind::Resign);
        assert_eq!(effective_bestmove_kind(&o), Some(BestMoveKind::Resign));
    }

    #[test]
    fn engine_bestmove_agreement_ignores_a_resigning_engine() {
        let observations = vec![obs("a", 4, 10, "7g7f"), obs("b", 4, 12, "resign")];
        assert_eq!(engine_bestmove_agreement(&observations), Some(true));
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
            requested_depth: None,
            score: Score::Mate { moves },
            score_perspective: ScorePerspective::SideToMove,
            score_bound: ScoreBound::Exact,
            bestmove: bestmove.to_string(),
            bestmove_kind: None,
            nodes: None,
            time_ms: None,
            pv: None,
            policy_margin_cp: None,
            candidates: Vec::new(),
            was_timeout_salvaged: false,
        }
    }

    fn obs_with_requested_depth(
        engine: &str,
        depth: u32,
        requested_depth: u32,
        cp: i32,
        bestmove: &str,
    ) -> Observation {
        let mut o = obs(engine, depth, cp, bestmove);
        o.requested_depth = Some(requested_depth);
        o
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
                root_id: None,
                variation_id: None,
                branch_from_ply: None,
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

    /// Like `simple_rec`, but White to move -- every other fixture in this module is Black to
    /// move, under which `cp_from_black_perspective` is a no-op and couldn't catch a perspective
    /// bug even if one existed. Tests that actually need the sign flip to matter use this.
    fn simple_rec_white_to_move(observations: Vec<Observation>) -> PositionRecord {
        let mut r = rec_with(GamePhase::Middlegame, false, false, observations);
        r.tags.side_to_move = SideToMove::White;
        r
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
    fn evaluate_quality_eval_min_gate_uses_black_perspective_when_white_to_move() {
        // White to move, raw (side-to-move-relative) cp +50 ("White is slightly better")
        // converts to black-perspective -50. eval_min=0 must reject this on the converted value
        // (-50 < 0) even though the *raw* value (+50) would NOT have triggered the gate -- this
        // fails if the gate ever regresses to comparing the raw side-to-move cp directly.
        let config = QualityConfig {
            eval_min: Some(0),
            ..Default::default()
        };
        let decision = evaluate_quality(
            &simple_rec_white_to_move(vec![obs("a", 4, 50, "7g7f")]),
            &config,
        );
        assert_eq!(decision.reasons, vec![QualityReason::EvalMin]);
    }

    #[test]
    fn evaluate_quality_eval_max_gate_uses_black_perspective_when_white_to_move() {
        // White to move, raw cp -50 ("White is slightly worse") converts to black-perspective
        // +50. eval_max=0 must reject this on the converted value (+50 > 0) even though the raw
        // value (-50) would NOT have triggered the gate.
        let config = QualityConfig {
            eval_max: Some(0),
            ..Default::default()
        };
        let decision = evaluate_quality(
            &simple_rec_white_to_move(vec![obs("a", 4, -50, "7g7f")]),
            &config,
        );
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
    fn evaluate_quality_require_requested_depth_reached_rejects_underreach() {
        let config = QualityConfig {
            require_requested_depth_reached: true,
            ..Default::default()
        };
        let observations = vec![obs_with_requested_depth("a", 8, 12, 10, "7g7f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert_eq!(
            decision.reasons,
            vec![QualityReason::RequestedDepthNotReached]
        );
    }

    #[test]
    fn evaluate_quality_require_requested_depth_reached_exempts_mate() {
        // Same rationale as min_depth_reached: a forced mate found short of the requested depth
        // is a confirmed, high-confidence result, not a weak search.
        let config = QualityConfig {
            require_requested_depth_reached: true,
            ..Default::default()
        };
        let mut o = obs_mate("a", 8, 3, "7g7f");
        o.requested_depth = Some(12);
        let decision = evaluate_quality(&simple_rec(vec![o]), &config);
        assert!(decision.keep);
    }

    #[test]
    fn evaluate_quality_require_requested_depth_reached_passes_when_met() {
        let config = QualityConfig {
            require_requested_depth_reached: true,
            ..Default::default()
        };
        let observations = vec![obs_with_requested_depth("a", 12, 12, 10, "7g7f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert!(decision.keep);
    }

    #[test]
    fn evaluate_quality_require_requested_depth_reached_is_noop_when_unset() {
        // requested_depth: None (legacy pre-field data) must not trip the gate.
        let config = QualityConfig {
            require_requested_depth_reached: true,
            ..Default::default()
        };
        let observations = vec![obs("a", 6, 10, "7g7f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert!(decision.keep);
    }

    #[test]
    fn evaluate_quality_require_requested_depth_reached_rejects_salvaged_mate_underreach_by_default()
     {
        // A timeout-salvaged observation carrying a mate score didn't actually confirm the mate
        // the way a genuine engine-initiated early stop does -- unlike
        // `evaluate_quality_require_requested_depth_reached_exempts_mate` (a non-salvaged mate),
        // this must NOT get the blanket mate exemption unless `allow_timeout_salvaged_mate` is set.
        let config = QualityConfig {
            require_requested_depth_reached: true,
            ..Default::default()
        };
        let mut o = obs_mate("a", 8, 3, "7g7f");
        o.requested_depth = Some(12);
        o.was_timeout_salvaged = true;
        let decision = evaluate_quality(&simple_rec(vec![o]), &config);
        assert_eq!(
            decision.reasons,
            vec![QualityReason::RequestedDepthNotReached]
        );
    }

    #[test]
    fn evaluate_quality_allow_timeout_salvaged_mate_restores_the_exemption() {
        let config = QualityConfig {
            require_requested_depth_reached: true,
            allow_timeout_salvaged_mate: true,
            ..Default::default()
        };
        let mut o = obs_mate("a", 8, 3, "7g7f");
        o.requested_depth = Some(12);
        o.was_timeout_salvaged = true;
        let decision = evaluate_quality(&simple_rec(vec![o]), &config);
        assert!(decision.keep);
    }

    #[test]
    fn evaluate_quality_exclude_timeout_salvaged_rejects_a_salvaged_observation() {
        let config = QualityConfig {
            exclude_timeout_salvaged: true,
            ..Default::default()
        };
        let mut o = obs("a", 6, 10, "7g7f");
        o.was_timeout_salvaged = true;
        let decision = evaluate_quality(&simple_rec(vec![o]), &config);
        assert_eq!(decision.reasons, vec![QualityReason::TimeoutSalvaged]);
    }

    #[test]
    fn evaluate_quality_exclude_timeout_salvaged_keeps_a_clean_observation() {
        let config = QualityConfig {
            exclude_timeout_salvaged: true,
            ..Default::default()
        };
        let observations = vec![obs("a", 6, 10, "7g7f")];
        let decision = evaluate_quality(&simple_rec(observations), &config);
        assert!(decision.keep);
    }

    #[test]
    fn evaluate_quality_exclude_timeout_salvaged_is_noop_when_unset() {
        let mut o = obs("a", 6, 10, "7g7f");
        o.was_timeout_salvaged = true;
        let decision = evaluate_quality(&simple_rec(vec![o]), &QualityConfig::default());
        assert!(decision.keep);
    }

    #[test]
    fn requested_depth_underreached_true_when_short_of_a_non_mate_request() {
        let o = obs_with_requested_depth("a", 8, 12, 10, "7g7f");
        assert!(requested_depth_underreached(&o));
    }

    #[test]
    fn requested_depth_underreached_false_when_met() {
        let o = obs_with_requested_depth("a", 12, 12, 10, "7g7f");
        assert!(!requested_depth_underreached(&o));
    }

    #[test]
    fn requested_depth_underreached_false_when_no_requested_depth_recorded() {
        let o = obs("a", 6, 10, "7g7f");
        assert!(!requested_depth_underreached(&o));
    }

    #[test]
    fn requested_depth_underreached_false_for_a_short_mate() {
        let mut o = obs_mate("a", 8, 3, "7g7f");
        o.requested_depth = Some(12);
        assert!(!requested_depth_underreached(&o));
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

    #[test]
    fn observation_without_requested_depth_key_deserializes_to_none() {
        // Pre-schema-v6 JSONL has no `requested_depth` key at all on its observations.
        // #[serde(default)] must still load it as None rather than failing to parse.
        let json = serde_json::json!({
            "engine": "myengine",
            "engine_version": null,
            "depth": 8,
            "score": { "kind": "cp", "value": 43 },
            "bestmove": "7g7f",
            "nodes": null,
            "time_ms": null,
            "pv": null
        });
        let observation: Observation = serde_json::from_value(json).unwrap();
        assert_eq!(observation.requested_depth, None);
    }
}
