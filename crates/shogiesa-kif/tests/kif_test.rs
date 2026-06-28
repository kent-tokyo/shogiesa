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
fn jsonl_roundtrip() {
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample.kif"), &config, &mut seen).unwrap();
    for rec in &records {
        let json = serde_json::to_string(rec).unwrap();
        let back: shogiesa_core::PositionRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sfen, rec.sfen);
        assert_eq!(back.schema_version, 1);
    }
}
