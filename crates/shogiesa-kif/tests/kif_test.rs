use std::collections::HashSet;
use std::path::Path;

use shogiesa_core::ExtractConfig;
use shogiesa_kif::{extract_from_path, extract_from_str};

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
