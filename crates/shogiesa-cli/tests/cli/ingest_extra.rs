use super::*;

// --- from-match ---

#[test]
fn from_match_extracts_positions_from_startpos_kifu() {
    let out = closed_temp_file();
    shogiesa()
        .args([
            "from-match",
            "--input",
            fixture("match_engine2_wins.txt").to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    let records: Vec<serde_json::Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // "7g7f 3c3d 2g2f" -> 3 plies
    assert_eq!(records.len(), 3);
    assert_eq!(records[0]["source"]["ply"], 1);
    assert_eq!(records[0]["source"]["kind"], "from_match");
    assert_eq!(
        records[0]["source"]["path"],
        fixture("match_engine2_wins.txt").to_str().unwrap()
    );
    assert_eq!(records[0]["tags"]["side_to_move"], "white"); // Black just moved
    assert_eq!(records[1]["tags"]["side_to_move"], "black");
    assert_eq!(records[2]["source"]["ply"], 3);
    assert!(records[0]["observations"].as_array().unwrap().is_empty());
}

#[test]
fn from_match_losing_side_engine1_keeps_only_engine2_win_games() {
    let out = closed_temp_file();
    shogiesa()
        .args([
            "from-match",
            "--input",
            fixture("match_engine1_wins.txt").to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--losing-side",
            "engine1",
        ])
        .assert()
        .success();
    // match_engine1_wins.txt has "# Result: Engine1 Win" -> Engine1 did NOT lose -> excluded
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 0);
}

#[test]
fn from_match_losing_side_engine2_keeps_only_engine1_win_games() {
    let out = closed_temp_file();
    shogiesa()
        .args([
            "from-match",
            "--input",
            fixture("match_engine1_wins.txt").to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--losing-side",
            "engine2",
        ])
        .assert()
        .success();
    // "# Result: Engine1 Win" -> Engine2 lost -> included
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 2);
}

#[test]
fn from_match_no_losing_side_keeps_all_games() {
    let dir = TempDir::new().unwrap();
    std::fs::copy(
        fixture("match_engine1_wins.txt"),
        dir.path().join("match_engine1_wins.txt"),
    )
    .unwrap();
    std::fs::copy(
        fixture("match_engine2_wins.txt"),
        dir.path().join("match_engine2_wins.txt"),
    )
    .unwrap();
    let out = closed_temp_file();
    shogiesa()
        .args([
            "from-match",
            "--input",
            dir.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    // 2 plies + 3 plies from the two games, regardless of who won
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 5);
}

#[test]
fn from_match_handles_directory_of_kifu_files() {
    let dir = TempDir::new().unwrap();
    std::fs::copy(
        fixture("match_engine1_wins.txt"),
        dir.path().join("match_engine1_wins.txt"),
    )
    .unwrap();
    std::fs::copy(
        fixture("match_engine2_wins.txt"),
        dir.path().join("match_engine2_wins.txt"),
    )
    .unwrap();
    let out = closed_temp_file();
    shogiesa()
        .args([
            "from-match",
            "--input",
            dir.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("2 games read"));
    let content = std::fs::read_to_string(out.path()).unwrap();
    let paths: HashSet<String> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["source"]["path"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(paths.len(), 2);
}

#[test]
fn from_match_promotion_and_drop_tokens_apply_correctly() {
    // The exact real kifu line verified during planning (34 move tokens, including a promotion
    // "9g5c+" and several drops "B*7e"/"P*9i"/"L*5d"/"S*6h"/"N*5g"). If any token failed to parse
    // or apply, extraction would stop early (fewer than 34 records).
    let out = closed_temp_file();
    shogiesa()
        .args([
            "from-match",
            "--input",
            fixture("match_promotions_and_drops.txt").to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    let records: Vec<serde_json::Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(records.len(), 34);
    assert_eq!(records.last().unwrap()["source"]["ply"], 34);
}

#[test]
fn from_match_position_sfen_line_extracts_from_the_given_starting_position() {
    // `position sfen ...` games (e.g. from a strength gate run with `--positions`, not
    // `startpos`) are parsed via `Board::from_sfen` and replayed from that starting position --
    // not skipped. The fixture's SFEN is the standard initial position (move_count 1), so after
    // one move (`7g7f`) exactly one position (ply 1) is extracted.
    let out = closed_temp_file();
    shogiesa()
        .args([
            "from-match",
            "--input",
            fixture("match_position_sfen_standard.txt")
                .to_str()
                .unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    let records: Vec<serde_json::Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["source"]["ply"], 1);
    assert_eq!(
        records[0]["sfen"],
        "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2"
    );
}

#[test]
fn from_match_position_sfen_nonstandard_start_continues_ply_from_move_count() {
    // A real gate-opening position (e.g. from a strength-gate run with `--positions`) has a
    // move_count > 1 and can have White to move first. Ply must continue from that move_count,
    // not restart at 0/1 -- otherwise every downstream ply-dependent behavior (phase
    // classification, ply histograms/distribution, --min-ply/--max-ply filters) would silently
    // misclassify every position extracted from a non-startpos game.
    let out = closed_temp_file();
    shogiesa()
        .args([
            "from-match",
            "--input",
            fixture("match_position_sfen_nonstandard_start.txt")
                .to_str()
                .unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    let records: Vec<serde_json::Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["source"]["ply"], 22);
    assert_eq!(records[1]["source"]["ply"], 23);
}

#[test]
fn from_match_position_sfen_with_king_in_hand_is_skipped_not_crashed() {
    // A malformed SFEN with a king literally in the hand field must be a clean skip (like any
    // other unparseable game), not a panic -- `PieceType::hand_idx()` has no entry for King, so
    // without `Sfen::parse` rejecting this up front, `Board::from_sfen` would panic instead of
    // returning an `Err` for `extract_from_match_kifu` to warn-and-skip on.
    let out = closed_temp_file();
    shogiesa()
        .args([
            "from-match",
            "--input",
            fixture("match_position_sfen_king_in_hand.txt")
                .to_str()
                .unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 0);
}

#[test]
fn from_match_no_txt_files_found_errors() {
    let dir = TempDir::new().unwrap();
    let out = closed_temp_file();
    shogiesa()
        .args([
            "from-match",
            "--input",
            dir.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no .txt kifu files found"));
}

#[test]
fn from_match_output_feeds_into_label_and_filter() {
    // Proves from-match's PositionRecord output is fully compatible with the existing pipeline,
    // end to end: from-match -> label -> filter.
    let extracted = closed_temp_file();
    shogiesa()
        .args([
            "from-match",
            "--input",
            fixture("match_engine2_wins.txt").to_str().unwrap(),
            "--out",
            extracted.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let labeled = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            extracted.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--out",
            labeled.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let filtered = closed_temp_file();
    shogiesa()
        .args([
            "filter",
            "--input",
            labeled.path().to_str().unwrap(),
            "--out",
            filtered.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(filtered.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 3);
}

// --- merge-observations ---

/// A position record at a distinct `ply` (and hence a distinct merge-alignment key even though
/// every fixture in this file shares the same SFEN/path), with the given `observations`.
pub(super) fn position_at_ply(ply: u32, observations: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 8,
        "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "source": { "kind": "csa", "path": "test.csa", "ply": ply },
        "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
        "observations": observations
    })
}

#[test]
fn merge_observations_keep_both_keeps_all_colliding_observations() {
    let primary = make_labeled_jsonl(&[
        position_at_ply(1, serde_json::json!([obs("7g7f", 50, 4)])), // shared, collides
        position_at_ply(2, serde_json::json!([obs("2g2f", 10, 4)])), // primary-only
    ]);
    let secondary = make_labeled_jsonl(&[
        position_at_ply(1, serde_json::json!([obs("3c3d", 60, 4)])), // shared, collides with above
        position_at_ply(3, serde_json::json!([obs("8c8d", 20, 4)])), // secondary-only
    ]);
    let out = closed_temp_file();
    shogiesa()
        .args([
            "merge-observations",
            "--primary",
            primary.path().to_str().unwrap(),
            "--secondary",
            secondary.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("1 merged"))
        .stderr(predicate::str::contains("1 primary-only"))
        .stderr(predicate::str::contains("1 secondary-only"))
        // KeepBoth appends unconditionally without ever checking for a collision (see
        // merge_observations_into) -- "0 colliding" is correct, not a bug.
        .stderr(predicate::str::contains("0 colliding"));

    let records: Vec<serde_json::Value> = std::fs::read_to_string(out.path())
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(records.len(), 3);
    let by_ply = |ply: u64| records.iter().find(|r| r["source"]["ply"] == ply).unwrap();
    // keep-both (default): both colliding observations survive on the shared ply=1 record
    let bestmoves: HashSet<String> = by_ply(1)["observations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["bestmove"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        bestmoves,
        HashSet::from(["7g7f".to_string(), "3c3d".to_string()])
    );
    assert!(by_ply(1)["stability"].is_null());
    // primary-only and secondary-only records pass through unchanged
    assert_eq!(
        by_ply(2)["observations"][0]["bestmove"],
        serde_json::json!("2g2f")
    );
    assert_eq!(
        by_ply(3)["observations"][0]["bestmove"],
        serde_json::json!("8c8d")
    );
}

#[test]
fn merge_observations_prefer_primary_drops_secondary_on_collision() {
    let primary =
        make_labeled_jsonl(&[position_at_ply(1, serde_json::json!([obs("7g7f", 50, 4)]))]);
    let secondary =
        make_labeled_jsonl(&[position_at_ply(1, serde_json::json!([obs("3c3d", 60, 4)]))]);
    let out = closed_temp_file();
    shogiesa()
        .args([
            "merge-observations",
            "--primary",
            primary.path().to_str().unwrap(),
            "--secondary",
            secondary.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--on-collision",
            "prefer-primary",
        ])
        .assert()
        .success();
    let record: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(out.path()).unwrap().trim()).unwrap();
    let observations = record["observations"].as_array().unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0]["bestmove"], serde_json::json!("7g7f"));
}

#[test]
fn merge_observations_prefer_secondary_replaces_primary_on_collision() {
    let primary =
        make_labeled_jsonl(&[position_at_ply(1, serde_json::json!([obs("7g7f", 50, 4)]))]);
    let secondary =
        make_labeled_jsonl(&[position_at_ply(1, serde_json::json!([obs("3c3d", 60, 4)]))]);
    let out = closed_temp_file();
    shogiesa()
        .args([
            "merge-observations",
            "--primary",
            primary.path().to_str().unwrap(),
            "--secondary",
            secondary.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--on-collision",
            "prefer-secondary",
        ])
        .assert()
        .success();
    let record: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(out.path()).unwrap().trim()).unwrap();
    let observations = record["observations"].as_array().unwrap();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0]["bestmove"], serde_json::json!("3c3d"));
}
