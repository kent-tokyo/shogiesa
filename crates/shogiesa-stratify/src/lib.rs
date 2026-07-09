//! Thin adapter mapping shogiesa's `PositionRecord` onto `stratifykit-core`'s generic
//! feature/bucket/group concepts. Holds shogi vocabulary (phase, side, eval-bucket, ply, source
//! root) so `stratifykit-core` doesn't have to; holds no bucketing/quota/sampling logic of its
//! own -- that stays in `stratifykit-core`.

use shogiesa_core::{PositionRecord, Score, SourceInfo, cp_from_black_perspective};
use stratifykit_core::bucket_floor;

/// The eval-bucket dimension of [`bucket_key`], factored out as a structured, orderable value so
/// callers that need a numeric span (e.g. `distribution`, enumerating every bucket between the
/// observed min and max) don't have to re-parse `bucket_key`'s string output back into a number.
///
/// Declaration order (`Unlabeled < Cp < Mate`) mirrors `report`'s existing
/// `i32::MIN`=unlabeled-sorts-first / `i32::MAX`=mate-sorts-last sentinel convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EvalBucket {
    Unlabeled,
    Cp(i32),
    Mate,
}

impl std::fmt::Display for EvalBucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalBucket::Cp(v) => write!(f, "{v}"),
            EvalBucket::Mate => write!(f, "mate"),
            EvalBucket::Unlabeled => write!(f, "_none_"),
        }
    }
}

pub fn eval_bucket_of(rec: &PositionRecord) -> EvalBucket {
    rec.observations
        .iter()
        .max_by_key(|o| o.depth)
        .map(|o| match o.score {
            Score::Cp { value } => {
                let black_value =
                    cp_from_black_perspective(value, o.score_perspective, rec.tags.side_to_move);
                EvalBucket::Cp((black_value.div_euclid(200)) * 200)
            }
            Score::Mate { .. } => EvalBucket::Mate,
        })
        .unwrap_or(EvalBucket::Unlabeled)
}

/// Composite phase/side/eval-bucket key for one record. Shared by every caller that needs "which
/// bucket is this record in" (balance, stratify, select --strategy coverage, distribution) so
/// their notion of "bucket" can never drift apart.
pub fn bucket_key(rec: &PositionRecord, by_phase: bool, by_side: bool, by_eval: bool) -> String {
    let mut key = String::new();
    if by_phase {
        key.push_str(&format!("{}:", rec.tags.phase));
    }
    if by_side {
        key.push_str(&format!("{}:", rec.tags.side_to_move));
    }
    if by_eval {
        key.push_str(&format!("{}:", eval_bucket_of(rec)));
    }
    key
}

pub fn feature_phase(rec: &PositionRecord) -> String {
    rec.tags.phase.to_string()
}

pub fn feature_side(rec: &PositionRecord) -> String {
    rec.tags.side_to_move.to_string()
}

pub fn feature_eval_bucket(rec: &PositionRecord) -> String {
    eval_bucket_of(rec).to_string()
}

pub fn feature_ply_bin(rec: &PositionRecord, bucket_size: u32) -> u32 {
    bucket_floor(rec.source.ply, bucket_size)
}

/// The grouping key used to keep a game's mainline and its variations in the same sampling
/// group. Prefers `source.root_id` (set by extractors that produce variations, e.g.
/// shogiesa-kif) since it doesn't depend on parsing a string convention back out of `path`;
/// falls back to stripping `path`'s `#varN@ply` suffix for extractors that never set `root_id`.
pub fn group_key(source: &SourceInfo) -> String {
    source
        .root_id
        .as_deref()
        .unwrap_or_else(|| split_root_path(&source.path))
        .to_string()
}

fn split_root_path(source_path: &str) -> &str {
    source_path.split("#var").next().unwrap_or(source_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shogiesa_core::{GamePhase, Observation, PositionTags, ScorePerspective, SideToMove};

    fn record_at(
        phase: GamePhase,
        side: SideToMove,
        ply: u32,
        root_id: Option<&str>,
        path: &str,
    ) -> PositionRecord {
        PositionRecord::new(
            "startpos".to_string(),
            SourceInfo {
                kind: "test".to_string(),
                path: path.to_string(),
                ply,
                root_id: root_id.map(str::to_string),
                variation_id: None,
                branch_from_ply: None,
            },
            PositionTags {
                phase,
                side_to_move: side,
                in_check: false,
                has_capture: false,
            },
        )
    }

    fn cp_obs(depth: u32, value: i32) -> Observation {
        Observation {
            engine: "test".to_string(),
            engine_version: None,
            depth,
            requested_depth: None,
            score: Score::Cp { value },
            score_perspective: ScorePerspective::SideToMove,
            score_bound: shogiesa_core::ScoreBound::default(),
            bestmove: "7g7f".to_string(),
            bestmove_kind: None,
            nodes: None,
            time_ms: None,
            pv: None,
            policy_margin_cp: None,
            candidates: Vec::new(),
            was_timeout_salvaged: false,
        }
    }

    fn mate_obs(depth: u32) -> Observation {
        Observation {
            score: Score::Mate { moves: 3 },
            ..cp_obs(depth, 0)
        }
    }

    #[test]
    fn feature_phase_and_side_match_display() {
        let rec = record_at(GamePhase::Opening, SideToMove::Black, 5, None, "g.csa");
        assert_eq!(feature_phase(&rec), "opening");
        assert_eq!(feature_side(&rec), "black");
    }

    #[test]
    fn feature_eval_bucket_floors_to_200cp_grid() {
        let mut rec = record_at(GamePhase::Middlegame, SideToMove::Black, 30, None, "g.csa");
        rec.observations.push(cp_obs(8, 250));
        assert_eq!(feature_eval_bucket(&rec), "200");
    }

    #[test]
    fn eval_bucket_of_floors_cp_to_200_and_flips_white_perspective() {
        // side_to_move=White with score_perspective=SideToMove: -50cp side-to-move-relative
        // flips to +50cp black-relative, floor(50/200)*200 = 0.
        let mut rec = record_at(GamePhase::Opening, SideToMove::White, 1, None, "g.csa");
        rec.observations.push(cp_obs(4, -50));
        assert_eq!(eval_bucket_of(&rec), EvalBucket::Cp(0));

        let mut rec = record_at(GamePhase::Opening, SideToMove::Black, 1, None, "g.csa");
        rec.observations.push(cp_obs(4, -250));
        assert_eq!(eval_bucket_of(&rec), EvalBucket::Cp(-400));
    }

    #[test]
    fn eval_bucket_of_picks_deepest_observation() {
        let mut rec = record_at(GamePhase::Opening, SideToMove::Black, 1, None, "g.csa");
        rec.observations.push(cp_obs(4, 900));
        rec.observations.push(cp_obs(8, 100));
        assert_eq!(eval_bucket_of(&rec), EvalBucket::Cp(0));
    }

    #[test]
    fn eval_bucket_of_classifies_mate_and_unlabeled() {
        let mut rec = record_at(GamePhase::Opening, SideToMove::Black, 1, None, "g.csa");
        rec.observations.push(mate_obs(4));
        assert_eq!(eval_bucket_of(&rec), EvalBucket::Mate);

        let rec = record_at(GamePhase::Opening, SideToMove::Black, 1, None, "g.csa");
        assert_eq!(eval_bucket_of(&rec), EvalBucket::Unlabeled);
    }

    #[test]
    fn eval_bucket_display_matches_bucket_keys_pre_refactor_tokens() {
        assert_eq!(EvalBucket::Cp(-400).to_string(), "-400");
        assert_eq!(EvalBucket::Mate.to_string(), "mate");
        assert_eq!(EvalBucket::Unlabeled.to_string(), "_none_");
    }

    #[test]
    fn feature_ply_bin_floors_to_bucket_size() {
        let rec = record_at(GamePhase::Endgame, SideToMove::White, 47, None, "g.csa");
        assert_eq!(feature_ply_bin(&rec, 20), 40);
    }

    #[test]
    fn group_key_prefers_root_id_over_path() {
        let rec = record_at(
            GamePhase::Opening,
            SideToMove::Black,
            1,
            Some("root-1"),
            "g.csa#var1@10",
        );
        assert_eq!(group_key(&rec.source), "root-1");
    }

    #[test]
    fn group_key_falls_back_to_stripped_path() {
        let rec = record_at(
            GamePhase::Opening,
            SideToMove::Black,
            1,
            None,
            "g.csa#var1@10",
        );
        assert_eq!(group_key(&rec.source), "g.csa");
    }
}
