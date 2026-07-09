use std::collections::HashSet;
use std::path::Path;

use shogiesa_core::{GameOutcome, GamePhase, SideToMove};
use shogiesa_csa::{ExtractConfig, extract_from_path, extract_from_str, extract_moves_from_str};

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
        assert_eq!(back.schema_version, shogiesa_core::SCHEMA_VERSION);
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

#[test]
fn moves_state_is_pre_move_sfen() {
    let content = std::fs::read_to_string(fixture("sample.csa")).unwrap();
    let moves = extract_moves_from_str(&content, "sample.csa").unwrap();
    assert_eq!(
        moves[0].sfen_before, "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "first move's pre-state must be the initial position, not post-move"
    );
    // Ply 2's pre-state must be ply 1's post-move state.
    assert!(moves[1].sfen_before.contains(" w "));
}

#[test]
fn moves_toryo_resolves_outcome_and_alternates_success_failure_by_mover() {
    let content = std::fs::read_to_string(fixture("sample.csa")).unwrap();
    let moves = extract_moves_from_str(&content, "sample.csa").unwrap();
    assert_eq!(moves.len(), 5);
    let expected = ["success", "failure", "success", "failure", "success"];
    for (mv, exp) in moves.iter().zip(expected) {
        assert_eq!(mv.outcome, GameOutcome::BlackWins);
        assert_eq!(mv.outcome.for_mover(mv.mover), exp, "ply {}", mv.ply);
    }
}

#[test]
fn moves_kachi_mover_wins() {
    let csa = "V2.2\nPI\n+\n+7776FU\n%KACHI\n";
    let moves = extract_moves_from_str(csa, "test").unwrap();
    // %KACHI's mover is White (Black already moved once), opposite sign from %TORYO.
    assert_eq!(moves[0].outcome, GameOutcome::WhiteWins);
    assert_eq!(moves[0].outcome.for_mover(moves[0].mover), "failure");
}

#[test]
fn moves_sennichite_is_draw() {
    let content = std::fs::read_to_string(fixture("sample_draw.csa")).unwrap();
    let moves = extract_moves_from_str(&content, "sample_draw.csa").unwrap();
    assert!(moves.iter().all(|m| m.outcome == GameOutcome::Draw));
}

#[test]
fn moves_chudan_is_unknown() {
    let csa = "V2.2\nPI\n+\n+7776FU\n%CHUDAN\n";
    let moves = extract_moves_from_str(csa, "test").unwrap();
    assert_eq!(moves[0].outcome, GameOutcome::Unknown);
}

#[test]
fn moves_no_terminal_action_is_unknown() {
    let csa = "V2.2\nPI\n+\n+7776FU\n-3334FU\n";
    let moves = extract_moves_from_str(csa, "test").unwrap();
    assert!(moves.iter().all(|m| m.outcome == GameOutcome::Unknown));
}

#[test]
fn moves_promotion_is_reflected_in_usi_move() {
    // Black's bishop 8h -> 2b, explicitly promoting (CSA piece code UM = Horse).
    let csa = "V2.2\nPI\n+\n+8822UM\n%TORYO\n";
    let moves = extract_moves_from_str(csa, "test").unwrap();
    assert_eq!(moves[0].usi_move, "8h2b+");
}

#[test]
fn moves_non_promotion_has_no_plus_suffix() {
    let csa = "V2.2\nPI\n+\n+7776FU\n%TORYO\n";
    let moves = extract_moves_from_str(csa, "test").unwrap();
    assert_eq!(moves[0].usi_move, "7g7f");
}
