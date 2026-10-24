mod extraction;
mod model;
mod quality;
mod score;

pub use extraction::*;
pub use model::*;
pub use quality::*;
pub use score::*;

pub mod board;
pub use board::{
    Board, BoardError, PieceType, UsiMove, UsiMoveError, parse_usi_move, zobrist_from_sfen,
};

pub mod schema;

pub mod manifest;

pub mod quality_report;

pub mod sfen;

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(engine: &str, depth: u32, cp: i32, bestmove: &str) -> Observation {
        Observation {
            engine: engine.to_string(),
            engine_version: None,
            depth,
            requested_depth: None,
            requested_nodes: None,
            search_limit_kind: SearchLimitKind::Depth,
            score: Score::Cp { value: cp },
            score_perspective: ScorePerspective::SideToMove,
            score_bound: ScoreBound::Exact,
            bestmove: bestmove.to_string(),
            bestmove_kind: None,
            nodes: None,
            time_ms: None,
            seldepth: None,
            nps: None,
            hashfull: None,
            pv: None,
            policy_margin_cp: None,
            candidates: Vec::new(),
            engine_options_hash: None,
            weight_sha256: None,
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

    #[test]
    fn cp_boundary_arithmetic_is_widened_and_schema_values_saturate() {
        assert_eq!(
            cp_from_black_perspective_i64(
                i32::MIN,
                ScorePerspective::SideToMove,
                SideToMove::White,
            ),
            2_147_483_648
        );
        assert_eq!(
            cp_from_black_perspective(i32::MIN, ScorePerspective::SideToMove, SideToMove::White,),
            i32::MAX
        );
        assert_eq!(score_swing_i64(&[i32::MIN, i32::MAX]), Some(4_294_967_295));
        assert_eq!(score_swing(&[i32::MIN, i32::MAX]), Some(i32::MAX));
    }

    fn obs_mate(engine: &str, depth: u32, moves: i32, bestmove: &str) -> Observation {
        Observation {
            engine: engine.to_string(),
            engine_version: None,
            depth,
            requested_depth: None,
            requested_nodes: None,
            search_limit_kind: SearchLimitKind::Depth,
            score: Score::Mate { moves },
            score_perspective: ScorePerspective::SideToMove,
            score_bound: ScoreBound::Exact,
            bestmove: bestmove.to_string(),
            bestmove_kind: None,
            nodes: None,
            time_ms: None,
            seldepth: None,
            nps: None,
            hashfull: None,
            pv: None,
            policy_margin_cp: None,
            candidates: Vec::new(),
            engine_options_hash: None,
            weight_sha256: None,
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
    fn evaluate_quality_uses_wide_cp_at_i32_boundaries() {
        let swing_config = QualityConfig {
            max_score_swing_cp: Some(i32::MAX),
            ..Default::default()
        };
        let decision = evaluate_quality(
            &simple_rec(vec![
                obs("a", 4, i32::MIN, "7g7f"),
                obs("a", 6, i32::MAX, "7g7f"),
            ]),
            &swing_config,
        );
        assert_eq!(decision.reasons, vec![QualityReason::ScoreSwing]);
        assert_eq!(decision.score_swing_cp, Some(i32::MAX));

        let eval_config = QualityConfig {
            eval_max: Some(i32::MAX),
            ..Default::default()
        };
        let decision = evaluate_quality(
            &simple_rec_white_to_move(vec![obs("a", 4, i32::MIN, "7g7f")]),
            &eval_config,
        );
        assert_eq!(decision.reasons, vec![QualityReason::EvalMax]);
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

    #[test]
    fn deserializes_legacy_observation_without_new_fields() {
        // Pre-schema-v11 JSONL has none of the fixed-node/telemetry/envelope keys at all.
        // #[serde(default)] must still load it, with search_limit_kind defaulting to Depth --
        // exactly what every pre-v11 observation always was.
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
        assert_eq!(observation.search_limit_kind, SearchLimitKind::Depth);
        assert_eq!(observation.requested_nodes, None);
        assert_eq!(observation.seldepth, None);
        assert_eq!(observation.nps, None);
        assert_eq!(observation.hashfull, None);
        assert_eq!(observation.engine_options_hash, None);
        assert_eq!(observation.weight_sha256, None);
    }

    #[test]
    fn position_record_without_game_result_key_deserializes_to_none() {
        // Pre-schema-v10 JSONL has no `game_result` key at all on its records. #[serde(default)]
        // must still load it as None rather than failing to parse.
        let json = serde_json::json!({
            "schema_version": 9,
            "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "source": { "kind": "csa", "path": "game.csa", "ply": 10 },
            "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
            "observations": []
        });
        let record: PositionRecord = serde_json::from_value(json).unwrap();
        assert_eq!(record.game_result, None);
    }

    #[test]
    fn game_outcome_serializes_snake_case() {
        for (outcome, expected) in [
            (GameOutcome::BlackWins, "black_wins"),
            (GameOutcome::WhiteWins, "white_wins"),
            (GameOutcome::Draw, "draw"),
            (GameOutcome::Unknown, "unknown"),
        ] {
            let serialized = serde_json::to_string(&outcome).unwrap();
            assert_eq!(serialized, format!("\"{expected}\""));
        }
    }
}
