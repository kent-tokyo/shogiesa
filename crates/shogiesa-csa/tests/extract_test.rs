use std::collections::HashSet;
use std::path::Path;

use shogiesa_core::{GamePhase, SideToMove};
use shogiesa_csa::{ExtractConfig, extract_from_path, extract_from_str};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn extract_sample_csa_count() {
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample.csa"), &config, &mut seen).unwrap();
    assert_eq!(records.len(), 5);
}

#[test]
fn extract_initial_sfen_is_correct() {
    let csa = "V2.2\nPI\n+\n+7776FU\n%TORYO\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: Some(1),
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(csa, "test", &config, &mut seen).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].sfen,
        "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2"
    );
}

#[test]
fn extract_ply_filter() {
    let config = ExtractConfig {
        min_ply: 3,
        max_ply: Some(4),
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample.csa"), &config, &mut seen).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].source.ply, 3);
    assert_eq!(records[1].source.ply, 4);
}

#[test]
fn extract_dedup() {
    let csa = "V2.2\nPI\n+\n+7776FU\n%TORYO\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: None,
        every_n: 1,
        dedup: true,
    };
    let mut seen = HashSet::new();
    let r1 = extract_from_str(csa, "game1.csa", &config, &mut seen).unwrap();
    let r2 = extract_from_str(csa, "game2.csa", &config, &mut seen).unwrap();
    assert_eq!(r1.len(), 1);
    assert_eq!(r2.len(), 0);
}

#[test]
fn extract_phase_tag() {
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: Some(3),
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample.csa"), &config, &mut seen).unwrap();
    for rec in &records {
        assert_eq!(rec.tags.phase, GamePhase::Opening);
    }
}

#[test]
fn jsonl_roundtrip() {
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample.csa"), &config, &mut seen).unwrap();
    for rec in &records {
        let json = serde_json::to_string(rec).unwrap();
        // JSON should still use lowercase strings
        assert!(
            json.contains("\"opening\"")
                || json.contains("\"middlegame\"")
                || json.contains("\"endgame\"")
        );
        assert!(json.contains("\"black\"") || json.contains("\"white\""));
        let back: shogiesa_core::PositionRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sfen, rec.sfen);
        assert_eq!(back.schema_version, 1);
    }
}

#[test]
fn side_to_move_tag_matches_sfen() {
    let csa = "V2.2\nPI\n+\n+7776FU\n-3334FU\n%TORYO\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: Some(2),
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(csa, "test", &config, &mut seen).unwrap();
    assert_eq!(records.len(), 2);

    assert!(records[0].sfen.contains(" w "), "ply1 sfen should have 'w'");
    assert_eq!(records[0].tags.side_to_move, SideToMove::White);

    assert!(records[1].sfen.contains(" b "), "ply2 sfen should have 'b'");
    assert_eq!(records[1].tags.side_to_move, SideToMove::Black);
}
