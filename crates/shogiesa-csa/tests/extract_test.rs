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
    let moves = extract_moves_from_str(&content, "sample.csa").unwrap().0;
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
    let moves = extract_moves_from_str(&content, "sample.csa").unwrap().0;
    assert_eq!(moves.len(), 5);
    let expected = ["success", "failure", "success", "failure", "success"];
    for (mv, exp) in moves.iter().zip(expected) {
        assert_eq!(mv.outcome, GameOutcome::BlackWins);
        assert_eq!(mv.outcome.for_mover(mv.mover), exp, "ply {}", mv.ply);
    }
}

#[test]
fn moves_tsumi_resolves_like_toryo() {
    let csa = "V2.2\nPI\n+\n+7776FU\n%TSUMI\n";
    let moves = extract_moves_from_str(csa, "test").unwrap().0;
    // %TSUMI's mover is White (Black already moved once) -- the side to move is mated, so the
    // opponent (Black) wins, same polarity as %TORYO.
    assert_eq!(moves[0].outcome, GameOutcome::BlackWins);
    assert_eq!(moves[0].outcome.for_mover(moves[0].mover), "success");
}

#[test]
fn moves_kachi_mover_wins() {
    let csa = "V2.2\nPI\n+\n+7776FU\n%KACHI\n";
    let moves = extract_moves_from_str(csa, "test").unwrap().0;
    // %KACHI's mover is White (Black already moved once), opposite sign from %TORYO.
    assert_eq!(moves[0].outcome, GameOutcome::WhiteWins);
    assert_eq!(moves[0].outcome.for_mover(moves[0].mover), "failure");
}

#[test]
fn moves_sennichite_is_draw() {
    let content = std::fs::read_to_string(fixture("sample_draw.csa")).unwrap();
    let moves = extract_moves_from_str(&content, "sample_draw.csa")
        .unwrap()
        .0;
    assert!(moves.iter().all(|m| m.outcome == GameOutcome::Draw));
}

#[test]
fn moves_chudan_is_unknown() {
    let csa = "V2.2\nPI\n+\n+7776FU\n%CHUDAN\n";
    let moves = extract_moves_from_str(csa, "test").unwrap().0;
    assert_eq!(moves[0].outcome, GameOutcome::Unknown);
}

#[test]
fn moves_no_terminal_action_is_unknown() {
    let csa = "V2.2\nPI\n+\n+7776FU\n-3334FU\n";
    let moves = extract_moves_from_str(csa, "test").unwrap().0;
    assert!(moves.iter().all(|m| m.outcome == GameOutcome::Unknown));
}

#[test]
fn moves_promotion_is_reflected_in_usi_move() {
    // Black's bishop 8h -> 2b, explicitly promoting (CSA piece code UM = Horse).
    let csa = "V2.2\nPI\n+\n+8822UM\n%TORYO\n";
    let moves = extract_moves_from_str(csa, "test").unwrap().0;
    assert_eq!(moves[0].usi_move, "8h2b+");
}

#[test]
fn moves_non_promotion_has_no_plus_suffix() {
    let csa = "V2.2\nPI\n+\n+7776FU\n%TORYO\n";
    let moves = extract_moves_from_str(csa, "test").unwrap().0;
    assert_eq!(moves[0].usi_move, "7g7f");
}

// --- extract_from_str's PositionRecord.game_result (distinct from RawMove.outcome above) ---
//
// Winner-polarity matters more here than a passing test count suggests: a wrong-polarity bug
// (labeling a black win as a white win) is a plausible-looking wrong label, worse than
// `Unknown` -- it would silently corrupt exactly the raw-vs-curated WDL diagnostic this field
// exists to enable. `extract_from_str` resolves `game_result` by delegating to
// `extract_moves_from_str` (see extract.rs) rather than re-deriving side-to-move parity, so these
// tests exercise the actual wiring, not a second hand-rolled resolver.

#[test]
fn extract_from_str_attaches_black_win_game_result() {
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample.csa"), &config, &mut seen).unwrap();
    assert!(!records.is_empty());
    for rec in &records {
        let gr = rec.game_result.as_ref().unwrap();
        assert_eq!(gr.outcome, GameOutcome::BlackWins);
        assert_eq!(gr.result_source, "csa_terminal");
    }
}

#[test]
fn extract_from_str_attaches_white_win_game_result() {
    // %KACHI's mover is White (Black already moved once) -- same fixture as moves_kachi_mover_wins.
    let csa = "V2.2\nPI\n+\n+7776FU\n%KACHI\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: None,
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(csa, "test", &config, &mut seen).unwrap();
    assert_eq!(records.len(), 1);
    let gr = records[0].game_result.as_ref().unwrap();
    assert_eq!(gr.outcome, GameOutcome::WhiteWins);
    assert_eq!(gr.result_source, "csa_terminal");
}

#[test]
fn extract_from_str_attaches_draw_game_result() {
    let config = ExtractConfig::default();
    let mut seen = HashSet::new();
    let records = extract_from_path(&fixture("sample_draw.csa"), &config, &mut seen).unwrap();
    assert!(!records.is_empty());
    for rec in &records {
        let gr = rec.game_result.as_ref().unwrap();
        assert_eq!(gr.outcome, GameOutcome::Draw);
    }
}

#[test]
fn extract_from_str_game_result_unknown_when_no_terminal() {
    let csa = "V2.2\nPI\n+\n+7776FU\n-3334FU\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: None,
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(csa, "test", &config, &mut seen).unwrap();
    assert!(!records.is_empty());
    for rec in &records {
        let gr = rec.game_result.as_ref().unwrap();
        assert_eq!(gr.outcome, GameOutcome::Unknown);
        assert_eq!(gr.result_source, "csa_no_terminal");
    }
}

#[test]
fn extract_from_str_chudan_result_source_is_interrupted() {
    let csa = "V2.2\nPI\n+\n+7776FU\n%CHUDAN\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: None,
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(csa, "test", &config, &mut seen).unwrap();
    assert_eq!(records.len(), 1);
    let gr = records[0].game_result.as_ref().unwrap();
    assert_eq!(gr.outcome, GameOutcome::Unknown);
    assert_eq!(gr.result_source, "csa_interrupted");
}

#[test]
fn extract_from_str_matta_result_source_is_terminal_undetermined() {
    let csa = "V2.2\nPI\n+\n+7776FU\n%MATTA\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: None,
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(csa, "test", &config, &mut seen).unwrap();
    assert_eq!(records.len(), 1);
    let gr = records[0].game_result.as_ref().unwrap();
    assert_eq!(gr.outcome, GameOutcome::Unknown);
    assert_eq!(gr.result_source, "csa_terminal_undetermined");
}

#[test]
fn extract_from_str_keeps_positions_before_a_mid_game_board_error() {
    // Move 1 is legal (black pawn 7g7f); move 2 references square (5,5), which is empty at that
    // point, so applying it fails with a board error. `extract_from_str`'s own loop already
    // warns-and-breaks on this (keeping move 1's position), but `game_result` resolution now goes
    // through `extract_moves_from_str`, which propagates this same error via `?` -- must not let
    // that empty out the whole extraction.
    let csa = "V2.2\nPI\n+\n+7776FU\n+5555FU\n%TORYO\n";
    let config = ExtractConfig {
        min_ply: 1,
        max_ply: None,
        every_n: 1,
        dedup: false,
    };
    let mut seen = HashSet::new();
    let records = extract_from_str(csa, "test", &config, &mut seen).unwrap();
    assert_eq!(
        records.len(),
        1,
        "move 1's position must survive move 2's board error, not be swallowed by it"
    );
    let gr = records[0].game_result.as_ref().unwrap();
    assert_eq!(
        gr.outcome,
        GameOutcome::Unknown,
        "a failed outcome-resolution walk degrades to Unknown rather than aborting extraction"
    );
    assert_eq!(gr.result_source, "csa_walk_error");
}
