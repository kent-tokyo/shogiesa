use std::collections::HashMap;

use crate::*;

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
    saturating_cp_i32(cp_from_black_perspective_i64(cp, perspective, side_to_move))
}

/// Wide counterpart of [`cp_from_black_perspective`]. Use this for arithmetic and comparisons so
/// negating `i32::MIN` remains representable instead of panicking or wrapping.
pub fn cp_from_black_perspective_i64(
    cp: i32,
    perspective: ScorePerspective,
    side_to_move: SideToMove,
) -> i64 {
    let cp = i64::from(cp);
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
    saturating_cp_i32(cp_from_side_to_move_perspective_i64(
        cp,
        perspective,
        side_to_move,
    ))
}

/// Wide counterpart of [`cp_from_side_to_move_perspective`].
pub fn cp_from_side_to_move_perspective_i64(
    cp: i32,
    perspective: ScorePerspective,
    side_to_move: SideToMove,
) -> i64 {
    let cp = i64::from(cp);
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

pub(crate) fn saturating_cp_i32(cp: i64) -> i32 {
    cp.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Cp swing (max - min) across at least 2 scores; `None` if fewer than 2.
/// Values wider than the schema's `i32` storage are saturated at `i32::MAX`.
pub fn score_swing(cp_scores: &[i32]) -> Option<i32> {
    score_swing_i64(cp_scores).map(saturating_cp_i32)
}

/// Wide cp swing for filtering, ranking, and diagnostics that must distinguish the full `i32`
/// input range without overflowing.
pub fn score_swing_i64(cp_scores: &[i32]) -> Option<i64> {
    if cp_scores.len() < 2 {
        return None;
    }
    let lo = *cp_scores.iter().min().unwrap();
    let hi = *cp_scores.iter().max().unwrap();
    Some(i64::from(hi) - i64::from(lo))
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
    engine_score_swing_i64(observations).map(saturating_cp_i32)
}

/// Wide counterpart of [`engine_score_swing`] for exact threshold comparisons.
pub fn engine_score_swing_i64(observations: &[Observation]) -> Option<i64> {
    let deepest = deepest_per_engine(observations);
    let cp: Vec<i32> = deepest
        .iter()
        .filter_map(|o| match o.score {
            Score::Cp { value } => Some(value),
            Score::Mate { .. } => None,
        })
        .collect();
    score_swing_i64(&cp)
}
