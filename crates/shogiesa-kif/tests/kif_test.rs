use std::collections::HashSet;
use std::path::Path;

use shogiesa_core::{ExtractConfig, GameOutcome, SideToMove};
use shogiesa_kif::{extract_from_path, extract_from_str, extract_moves_from_str};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn extract_sample_kif_count() {
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample.kif"), &config, &mut seen).unwrap();
    assert_eq!(records.len(), 5, "sample.kif has 5 moves");
}

#[test]
fn first_move_sfen_matches_csa() {
    // +7776FU in CSA = ７六歩(77) in KIF — same resulting SFEN
    let kif =
        "手合割：平手\n先手：A\n後手：B\n手数----指手\n   1 ７六歩(77)   (0:01/0)\n   2 投了\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: Some(1),
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(kif, "test.kif", &config, &mut seen).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].sfen,
        "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2"
    );
}

#[test]
fn source_kind_is_kif() {
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample.kif"), &config, &mut seen).unwrap();
    assert!(records.iter().all(|r| r.source.kind == "kif"));
}

#[test]
fn ply_filter_works() {
    let config = ExtractConfig {
        min_ply: 2,
        max_ply: Some(3),
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample.kif"), &config, &mut seen).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].source.ply, 2);
    assert_eq!(records[1].source.ply, 3);
}

#[test]
fn dedup_works() {
    let kif =
        "手合割：平手\n先手：A\n後手：B\n手数----指手\n   1 ７六歩(77)   (0:01/0)\n   2 投了\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: None,
        every_n: 1,
        dedup: true,
    };
    let mut seen = HashSet::new();
    let r1 = extract_from_str(kif, "g1.kif", &config, &mut seen).unwrap();
    let r2 = extract_from_str(kif, "g2.kif", &config, &mut seen).unwrap();
    assert_eq!(r1.len(), 1);
    assert_eq!(r2.len(), 0); // duplicate
}

#[test]
fn unknown_handicap_errors() {
    let kif = "手合割：謎の駒落ち\n手数----指手\n   1 ７六歩(77)\n";
    let mut seen = HashSet::new();
    let result = extract_from_str(kif, "h.kif", &ExtractConfig::default(), &mut seen);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("unsupported handicap")
    );
}

#[test]
fn handicap_rook_drop_initial_sfen() {
    // 飛車落ち: White's rook removed; White moves first
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: Some(1),
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample_handicap.kif"), &config, &mut seen).unwrap();
    assert_eq!(records.len(), 1);
    // SFEN after White's 3d pawn move: rook at 8b is absent, White moved first
    let sfen = &records[0].sfen;
    // No rook on rank b (White's rank 2)
    let board_part = sfen.split_whitespace().next().unwrap();
    let rank_b = board_part.split('/').nth(1).unwrap();
    assert!(
        !rank_b.contains('r'),
        "White's rook should be absent in 飛車落ち: {rank_b}"
    );
}

#[test]
fn handicap_types_all_parse() {
    let handicaps = [
        "平手",
        "香落ち",
        "右香落ち",
        "角落ち",
        "飛車落ち",
        "二枚落ち",
        "四枚落ち",
        "六枚落ち",
        "八枚落ち",
        "十枚落ち",
    ];
    for h in handicaps {
        let kif = format!("手合割：{h}\n手数----指手\n   1 投了\n");
        let mut seen = HashSet::new();
        let result = extract_from_str(&kif, "test.kif", &ExtractConfig::default(), &mut seen);
        assert!(result.is_ok(), "handicap {h:?} should parse without error");
    }
}

#[test]
fn same_square_notation_extracts_correctly() {
    // Ply 1 is filtered out by min_ply=2, so if `prev_dest` were only updated
    // after the min_ply filter, ply 4's "同" would resolve against a stale
    // (or missing) destination instead of ply 3's real destination.
    let kif = "手合割：平手\n先手：A\n後手：B\n手数----指手\n\
   1 ７六歩(77)   (0:01/0)\n   2 ３四歩(33)   (0:01/0)\n\
   3 ２二角成(88)  (0:01/0)\n   4 同　銀(31)   (0:01/0)\n   5 投了\n";
    let config = ExtractConfig {
        min_ply: 2,
        max_ply: None,
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(kif, "same.kif", &config, &mut seen).unwrap();
    // Plies 2, 3, 4 should all extract (ply 1 filtered out by min_ply, game
    // does not get truncated by the "同" move at ply 4).
    assert_eq!(records.len(), 3);
    assert_eq!(
        records.iter().map(|r| r.source.ply).collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
}

#[test]
fn variation_branch_is_extracted() {
    let kif = "手合割：平手\n先手：A\n後手：B\n手数----指手\n\
   1 ７六歩(77)   (0:01/0)\n   2 ３四歩(33)   (0:01/0)\n\
\n変化：2手\n   2 ８四歩(83)   (0:01/0)\n   3 ７八金(69)   (0:01/0)\n";
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_str(kif, "var.kif", &config, &mut seen).unwrap();

    // Mainline (2 moves) + variation (2 moves) = 4 records total.
    assert_eq!(records.len(), 4);

    let mainline: Vec<_> = records
        .iter()
        .filter(|r| r.source.path == "var.kif")
        .collect();
    let variation: Vec<_> = records
        .iter()
        .filter(|r| r.source.path == "var.kif#var1@2")
        .collect();
    assert_eq!(mainline.len(), 2);
    assert_eq!(variation.len(), 2);
    assert_eq!(
        mainline.iter().map(|r| r.source.ply).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        variation.iter().map(|r| r.source.ply).collect::<Vec<_>>(),
        vec![2, 3]
    );

    // The variation must branch from the mainline's move-1 state, not from scratch: replaying
    // "7g7f, 8d8e" directly as its own 2-move game must produce the same ply-2 SFEN as the
    // variation's first record — proving it branched from the correct checkpoint.
    let standalone_kif = "手合割：平手\n手数----指手\n\
   1 ７六歩(77)   (0:01/0)\n   2 ８四歩(83)   (0:01/0)\n";
    let mut seen2 = HashSet::new();
    let standalone = extract_from_str(
        standalone_kif,
        "standalone.kif",
        &ExtractConfig::default(),
        &mut seen2,
    )
    .unwrap();
    assert_eq!(variation[0].sfen, standalone[1].sfen);

    // And it must differ from the mainline's own move-2 SFEN (a different move was played).
    assert_ne!(variation[0].sfen, mainline[1].sfen);
}

#[test]
fn variation_records_share_root_id_with_mainline() {
    let kif = "手合割：平手\n先手：A\n後手：B\n手数----指手\n\
   1 ７六歩(77)   (0:01/0)\n   2 ３四歩(33)   (0:01/0)\n\
\n変化：2手\n   2 ８四歩(83)   (0:01/0)\n   3 ７八金(69)   (0:01/0)\n";
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_str(kif, "var.kif", &config, &mut seen).unwrap();

    for rec in &records {
        assert_eq!(
            rec.source.root_id.as_deref(),
            Some("var.kif"),
            "mainline and variation records share the same root_id"
        );
    }
    let mainline_rec = records.iter().find(|r| r.source.path == "var.kif").unwrap();
    let variation_rec = records
        .iter()
        .find(|r| r.source.path == "var.kif#var1@2")
        .unwrap();
    assert_eq!(mainline_rec.source.variation_id, None);
    assert_eq!(mainline_rec.source.branch_from_ply, None);
    assert_eq!(variation_rec.source.variation_id.as_deref(), Some("var1"));
    assert_eq!(variation_rec.source.branch_from_ply, Some(2));
}

#[test]
fn multiple_sibling_variations_get_distinct_paths() {
    let kif = "手合割：平手\n手数----指手\n\
   1 ７六歩(77)   (0:01/0)\n   2 ３四歩(33)   (0:01/0)\n   3 ２六歩(27)   (0:01/0)\n\
\n変化：2手\n   2 ８四歩(83)   (0:01/0)\n\
\n変化：3手\n   3 ４四歩(43)   (0:01/0)\n";
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_str(kif, "multi.kif", &config, &mut seen).unwrap();

    let mainline_count = records
        .iter()
        .filter(|r| r.source.path == "multi.kif")
        .count();
    let var1_count = records
        .iter()
        .filter(|r| r.source.path == "multi.kif#var1@2")
        .count();
    let var2_count = records
        .iter()
        .filter(|r| r.source.path == "multi.kif#var2@3")
        .count();
    assert_eq!(mainline_count, 3);
    assert_eq!(var1_count, 1);
    assert_eq!(var2_count, 1);
}

#[test]
fn malformed_variation_reference_is_skipped_not_fatal() {
    let kif = "手合割：平手\n手数----指手\n\
   1 ７六歩(77)   (0:01/0)\n   2 ３四歩(33)   (0:01/0)\n\
\n変化：99手\n   99 ８四歩(83)   (0:01/0)\n";
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_str(kif, "bad.kif", &config, &mut seen).unwrap();
    // Mainline still extracts fully despite the out-of-range variation reference.
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|r| r.source.path == "bad.kif"));
}

#[test]
fn variation_past_max_ply_does_not_abort_later_siblings() {
    let kif = "手合割：平手\n手数----指手\n\
   1 ７六歩(77)   (0:01/0)\n   2 ３四歩(33)   (0:01/0)\n\
\n変化：2手\n   2 ８四歩(83)   (0:01/0)\n   3 ７八金(69)   (0:01/0)\n\
\n変化：2手\n   2 ２六歩(27)   (0:01/0)\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: Some(2),
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(kif, "game.kif", &config, &mut seen).unwrap();

    let var1: Vec<_> = records
        .iter()
        .filter(|r| r.source.path == "game.kif#var1@2")
        .collect();
    let var2: Vec<_> = records
        .iter()
        .filter(|r| r.source.path == "game.kif#var2@2")
        .collect();
    // var1's second move (ply 3) exceeds max_ply and must not extract, but must also not abort
    // scanning for var2, which comes after it in the file.
    assert_eq!(var1.len(), 1);
    assert_eq!(
        var2.len(),
        1,
        "sibling variation after a max-ply-truncated variation must still extract"
    );
}

#[test]
fn moves_state_is_pre_move_sfen() {
    let content = std::fs::read_to_string(fixture("sample.kif")).unwrap();
    let moves = extract_moves_from_str(&content, "sample.kif").unwrap().0;
    assert_eq!(
        moves[0].sfen_before, "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "first move's pre-state must be the initial position, not post-move"
    );
    assert!(moves[1].sfen_before.contains(" w "));
}

#[test]
fn moves_toryo_and_summary_line_agree_and_alternate_by_mover() {
    let content = std::fs::read_to_string(fixture("sample.kif")).unwrap();
    let moves = extract_moves_from_str(&content, "sample.kif").unwrap().0;
    assert_eq!(moves.len(), 5);
    let expected = ["success", "failure", "success", "failure", "success"];
    for (mv, exp) in moves.iter().zip(expected) {
        assert_eq!(mv.outcome, GameOutcome::BlackWins);
        assert_eq!(mv.outcome.for_mover(mv.mover), exp, "ply {}", mv.ply);
    }
}

#[test]
fn moves_promotion_is_reflected_in_usi_move() {
    // Ply 3, "２二角成(88)", is an explicit promotion in this fixture.
    let content = std::fs::read_to_string(fixture("sample.kif")).unwrap();
    let moves = extract_moves_from_str(&content, "sample.kif").unwrap().0;
    assert_eq!(moves[2].usi_move, "8h2b+");
}

#[test]
fn moves_jishogi_is_draw() {
    let kif = "手合割：平手\n先手：A\n後手：B\n手数----指手\n   1 ７六歩(77)   (0:01/0)\n持将棋\n";
    let moves = extract_moves_from_str(kif, "test.kif").unwrap().0;
    assert_eq!(moves[0].outcome, GameOutcome::Draw);
}

#[test]
fn moves_made_prefix_sennichite_is_draw() {
    // Regression: a "まで"-prefixed summary line must fall through to the 千日手/持将棋 check,
    // not stop at the 先手/後手の勝ち check and give up.
    let kif = "手合割：平手\n先手：A\n後手：B\n手数----指手\n   1 ７六歩(77)   (0:01/0)\nまで1手で千日手\n";
    let moves = extract_moves_from_str(kif, "test.kif").unwrap().0;
    assert_eq!(moves[0].outcome, GameOutcome::Draw);
}

#[test]
fn moves_made_prefix_jishogi_is_draw() {
    let kif = "手合割：平手\n先手：A\n後手：B\n手数----指手\n   1 ７六歩(77)   (0:01/0)\nまで1手で持将棋\n";
    let moves = extract_moves_from_str(kif, "test.kif").unwrap().0;
    assert_eq!(moves[0].outcome, GameOutcome::Draw);
}

#[test]
fn moves_chudan_is_unknown() {
    let kif = "手合割：平手\n先手：A\n後手：B\n手数----指手\n   1 ７六歩(77)   (0:01/0)\n中断\n";
    let moves = extract_moves_from_str(kif, "test.kif").unwrap().0;
    assert_eq!(moves[0].outcome, GameOutcome::Unknown);
}

#[test]
fn moves_handicap_first_mover_is_not_hardcoded_to_black() {
    // 飛車落ち: White ("Upper") moves first. The fixture's own summary line, "まで2手で後手の
    // 勝ち", names Black ("Lower") the winner -- 先手/後手 track move ORDER, not a fixed color,
    // so resolving this correctly requires knowing White moved first in this game.
    let content = std::fs::read_to_string(fixture("sample_handicap.kif")).unwrap();
    let moves = extract_moves_from_str(&content, "sample_handicap.kif")
        .unwrap()
        .0;
    assert_eq!(moves.len(), 2);
    assert_eq!(moves[0].mover, SideToMove::White);
    assert_eq!(moves[0].outcome, GameOutcome::BlackWins);
    assert_eq!(moves[0].outcome.for_mover(moves[0].mover), "failure");
    assert_eq!(moves[1].mover, SideToMove::Black);
    assert_eq!(moves[1].outcome.for_mover(moves[1].mover), "success");
}

#[test]
fn moves_variation_branch_outcome_is_unknown_but_shares_sequence_root() {
    let kif = "手合割：平手\n先手：A\n後手：B\n手数----指手\n\
   1 ７六歩(77)   (0:01/0)\n   2 ３四歩(33)   (0:01/0)\nまで2手で先手の勝ち\n\
\n変化：2手\n   2 ８四歩(83)   (0:01/0)\n   3 ７八金(69)   (0:01/0)\n";
    let moves = extract_moves_from_str(kif, "var.kif").unwrap().0;
    let mainline: Vec<_> = moves
        .iter()
        .filter(|m| m.source.path == "var.kif")
        .collect();
    let variation: Vec<_> = moves
        .iter()
        .filter(|m| m.source.path == "var.kif#var1@2")
        .collect();
    assert_eq!(mainline.len(), 2);
    assert_eq!(variation.len(), 2);
    assert!(mainline.iter().all(|m| m.outcome == GameOutcome::BlackWins));
    assert!(variation.iter().all(|m| m.outcome == GameOutcome::Unknown));
    // Both share the mainline's root_id, so a downstream split-by-sequence groups them together.
    assert_eq!(mainline[0].source.root_id.as_deref(), Some("var.kif"));
    assert_eq!(variation[0].source.root_id.as_deref(), Some("var.kif"));
}

#[test]
fn jsonl_roundtrip() {
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample.kif"), &config, &mut seen).unwrap();
    for rec in &records {
        let json = serde_json::to_string(rec).unwrap();
        let back: shogiesa_core::PositionRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sfen, rec.sfen);
        assert_eq!(back.schema_version, shogiesa_core::SCHEMA_VERSION);
    }
}

// --- extract_from_str's PositionRecord.game_result (distinct from RawMove.outcome above) ---
//
// Winner-polarity matters more here than a passing test count suggests: a wrong-polarity bug
// (labeling a black win as a white win) is a plausible-looking wrong label, worse than
// `Unknown` -- it would silently corrupt exactly the raw-vs-curated WDL diagnostic this field
// exists to enable. `extract_from_str` resolves `game_result` by delegating to
// `extract_moves_from_str` (see lib.rs) rather than re-deriving side-to-move parity, so these
// tests exercise the actual wiring, not a second hand-rolled resolver.

#[test]
fn extract_from_str_attaches_black_win_game_result() {
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample.kif"), &config, &mut seen).unwrap();
    assert!(!records.is_empty());
    for rec in &records {
        let gr = rec.game_result.as_ref().unwrap();
        assert_eq!(gr.outcome, GameOutcome::BlackWins);
        assert_eq!(gr.result_source, "kif_marker");
    }
}

#[test]
fn extract_from_str_attaches_white_win_game_result() {
    // Standard (non-handicap) game: first_mover = Black, so "後手の勝ち" names White the winner.
    let kif = "手合割：平手\n先手：A\n後手：B\n手数----指手\n   1 ７六歩(77)   (0:01/0)\nまで1手で後手の勝ち\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: None,
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(kif, "test.kif", &config, &mut seen).unwrap();
    assert_eq!(records.len(), 1);
    let gr = records[0].game_result.as_ref().unwrap();
    assert_eq!(gr.outcome, GameOutcome::WhiteWins);
    assert_eq!(gr.result_source, "kif_marker");
}

#[test]
fn extract_from_str_handicap_game_result_uses_move_order_not_fixed_color() {
    // Same fixture as moves_handicap_first_mover_is_not_hardcoded_to_black: White moves first
    // (飛車落ち), and "後手の勝ち" (Black wins) requires knowing move order, not a fixed color.
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample_handicap.kif"), &config, &mut seen).unwrap();
    assert!(!records.is_empty());
    for rec in &records {
        assert_eq!(
            rec.game_result.as_ref().unwrap().outcome,
            GameOutcome::BlackWins
        );
    }
}

#[test]
fn extract_from_str_attaches_draw_game_result() {
    let kif = "手合割：平手\n先手：A\n後手：B\n手数----指手\n   1 ７六歩(77)   (0:01/0)   (0:01/0)\n持将棋\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: None,
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(kif, "test.kif", &config, &mut seen).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].game_result.as_ref().unwrap().outcome,
        GameOutcome::Draw
    );
}

#[test]
fn extract_from_str_variation_branch_game_result_is_unknown() {
    let kif = "手合割：平手\n先手：A\n後手：B\n手数----指手\n\
   1 ７六歩(77)   (0:01/0)\n   2 ３四歩(33)   (0:01/0)\nまで2手で先手の勝ち\n\
\n変化：2手\n   2 ８四歩(83)   (0:01/0)\n   3 ７八金(69)   (0:01/0)\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: None,
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(kif, "var.kif", &config, &mut seen).unwrap();
    let mainline: Vec<_> = records
        .iter()
        .filter(|r| r.source.path == "var.kif")
        .collect();
    let variation: Vec<_> = records
        .iter()
        .filter(|r| r.source.path == "var.kif#var1@2")
        .collect();
    assert!(!mainline.is_empty());
    assert!(!variation.is_empty());
    assert!(
        mainline
            .iter()
            .all(|r| r.game_result.as_ref().unwrap().outcome == GameOutcome::BlackWins)
    );
    assert!(
        variation
            .iter()
            .all(|r| r.game_result.as_ref().unwrap().outcome == GameOutcome::Unknown)
    );
    assert!(
        variation
            .iter()
            .all(|r| { r.game_result.as_ref().unwrap().result_source == "kif_variation" })
    );
}

#[test]
fn extract_from_str_no_terminal_result_source_is_kif_no_terminal() {
    let kif = "手合割：平手\n先手：A\n後手：B\n手数----指手\n   1 ７六歩(77)   (0:01/0)\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: None,
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(kif, "test.kif", &config, &mut seen).unwrap();
    assert_eq!(records.len(), 1);
    let gr = records[0].game_result.as_ref().unwrap();
    assert_eq!(gr.outcome, GameOutcome::Unknown);
    assert_eq!(gr.result_source, "kif_no_terminal");
}

#[test]
fn extract_from_str_chudan_result_source_is_interrupted() {
    let kif = "手合割：平手\n先手：A\n後手：B\n手数----指手\n   1 ７六歩(77)   (0:01/0)\n中断\n";
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_str(kif, "test.kif", &config, &mut seen).unwrap();
    assert_eq!(records.len(), 1);
    let gr = records[0].game_result.as_ref().unwrap();
    assert_eq!(gr.outcome, GameOutcome::Unknown);
    assert_eq!(gr.result_source, "kif_interrupted");
}
