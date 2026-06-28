use std::io::Write;
use std::path::Path;

use assert_cmd::{Command, cargo::cargo_bin};
use predicates::prelude::*;
use tempfile::NamedTempFile;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn shogiesa() -> Command {
    Command::cargo_bin("shogiesa").unwrap()
}

// --- extract ---

#[test]
fn extract_creates_jsonl() {
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "extract",
            "--input",
            fixture("sample.csa").to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 5, "sample.csa has 5 moves → 5 positions");
    // Each line is valid JSON with schema_version
    for line in &lines {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["schema_version"], 1);
    }
}

#[test]
fn extract_ply_filter_flag() {
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "extract",
            "--input",
            fixture("sample.csa").to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--min-ply",
            "3",
            "--max-ply",
            "4",
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2);
}

// --- report ---

#[test]
fn report_shows_stats() {
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "extract",
            "--input",
            fixture("sample.csa").to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    shogiesa()
        .args(["report", "--input", out.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("positions"))
        .stdout(predicate::str::contains("phase distribution"))
        .stdout(predicate::str::contains("duplicate SFENs"))
        .stdout(predicate::str::contains("balance warnings"));
}

// --- validate (normal mode) ---

#[test]
fn validate_clean_data_exits_0() {
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "extract",
            "--input",
            fixture("sample.csa").to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    shogiesa()
        .args(["validate", "--input", out.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("OK"));
}

#[test]
fn validate_broken_json_shows_warn_but_exits_0() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "{{not valid json}}").unwrap();
    f.flush().unwrap();

    shogiesa()
        .args(["validate", "--input", f.path().to_str().unwrap()])
        .assert()
        .success() // no --strict → exit 0
        .stdout(predicate::str::contains("WARN"));
}

// --- validate --strict ---

#[test]
fn validate_strict_clean_data_exits_0() {
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "extract",
            "--input",
            fixture("sample.csa").to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    shogiesa()
        .args([
            "validate",
            "--input",
            out.path().to_str().unwrap(),
            "--strict",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("OK"));
}

#[test]
fn validate_strict_broken_json_exits_1() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "{{not valid json}}").unwrap();
    f.flush().unwrap();

    shogiesa()
        .args([
            "validate",
            "--input",
            f.path().to_str().unwrap(),
            "--strict",
        ])
        .assert()
        .failure();
}

#[test]
fn validate_strict_tag_mismatch_exits_1() {
    // Craft a record where side_to_move tag says "black" but SFEN says "w"
    let bad_record = serde_json::json!({
        "schema_version": 1,
        "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2",
        "source": { "kind": "csa", "path": "test.csa", "ply": 1 },
        "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
        "observations": []
    });
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "{}", bad_record).unwrap();
    f.flush().unwrap();

    shogiesa()
        .args([
            "validate",
            "--input",
            f.path().to_str().unwrap(),
            "--strict",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("tag mismatch"));
}

// --- label ---

#[test]
fn label_adds_observations() {
    let pos = NamedTempFile::new().unwrap();
    let obs = NamedTempFile::new().unwrap();

    // 1. extract 5 positions from sample.csa
    shogiesa()
        .args([
            "extract",
            "--input",
            fixture("sample.csa").to_str().unwrap(),
            "--out",
            pos.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // 2. label with fake engine, depths 4,6
    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            cargo_bin("fake-usi-engine").to_str().unwrap(),
            "--depths",
            "4,6",
            "--out",
            obs.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // 3. each line should have 2 observations (one per depth)
    let content = std::fs::read_to_string(obs.path()).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 5);

    for line in &lines {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let obs = v["observations"].as_array().unwrap();
        assert_eq!(obs.len(), 2, "expected 2 observations (depth 4 and 6)");
        assert_eq!(obs[0]["score"]["kind"], "cp");
        assert_eq!(obs[0]["score"]["value"], 100);
        assert_eq!(obs[0]["bestmove"], "7g7f");
    }
}

#[test]
fn label_appends_to_existing_observations() {
    let pos = NamedTempFile::new().unwrap();
    let obs1 = NamedTempFile::new().unwrap();
    let obs2 = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "extract",
            "--input",
            fixture("sample.csa").to_str().unwrap(),
            "--out",
            pos.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // First label pass: depth 4
    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            cargo_bin("fake-usi-engine").to_str().unwrap(),
            "--depths",
            "4",
            "--out",
            obs1.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Second label pass: depth 6 on top of first
    shogiesa()
        .args([
            "label",
            "--input",
            obs1.path().to_str().unwrap(),
            "--engine",
            cargo_bin("fake-usi-engine").to_str().unwrap(),
            "--depths",
            "6",
            "--out",
            obs2.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(obs2.path()).unwrap();
    let first_line = content.lines().next().unwrap();
    let v: serde_json::Value = serde_json::from_str(first_line).unwrap();
    // Should have 2 observations total (4 from first pass + 6 from second)
    assert_eq!(v["observations"].as_array().unwrap().len(), 2);
}

// --- filter ---

/// Build a labeled JSONL string with custom observations inline.
fn make_labeled_jsonl(records: &[serde_json::Value]) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    for rec in records {
        writeln!(f, "{rec}").unwrap();
    }
    f.flush().unwrap();
    f
}

fn obs(bestmove: &str, score_cp: i32, depth: u32) -> serde_json::Value {
    serde_json::json!({
        "engine": "test",
        "engine_version": null,
        "depth": depth,
        "score": { "kind": "cp", "value": score_cp },
        "bestmove": bestmove,
        "nodes": null,
        "time_ms": null,
        "pv": null
    })
}

fn obs_mate(bestmove: &str, moves: i32, depth: u32) -> serde_json::Value {
    serde_json::json!({
        "engine": "test",
        "engine_version": null,
        "depth": depth,
        "score": { "kind": "mate", "moves": moves },
        "bestmove": bestmove,
        "nodes": null,
        "time_ms": null,
        "pv": null
    })
}

fn position(phase: &str, observations: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "source": { "kind": "csa", "path": "test.csa", "ply": 1 },
        "tags": { "phase": phase, "side_to_move": "black", "in_check": false, "has_capture": false },
        "observations": observations
    })
}

#[test]
fn filter_no_observations_excluded() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 0);
}

#[test]
fn filter_bestmove_agreement_passes() {
    let f = make_labeled_jsonl(&[position(
        "middlegame",
        serde_json::json!([obs("7g7f", 50, 4), obs("7g7f", 55, 6),]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--require-bestmove-agreement",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 1);
}

#[test]
fn filter_bestmove_disagreement_excluded() {
    let f = make_labeled_jsonl(&[position(
        "middlegame",
        serde_json::json!([
            obs("7g7f", 50, 4),
            obs("2b3c", 55, 6), // different bestmove
        ]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--require-bestmove-agreement",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 0);
}

#[test]
fn filter_score_swing_excluded() {
    let f = make_labeled_jsonl(&[position(
        "middlegame",
        serde_json::json!([
            obs("7g7f", 50, 4),
            obs("7g7f", 300, 6), // swing = 250 cp
        ]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--max-score-swing-cp",
            "150",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 0);
}

#[test]
fn filter_exclude_mate() {
    let f = make_labeled_jsonl(&[
        position("middlegame", serde_json::json!([obs("7g7f", 50, 4)])),
        position("middlegame", serde_json::json!([obs_mate("7g7f", 3, 4)])),
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--exclude-mate",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 1);
}

#[test]
fn filter_eval_range() {
    let f = make_labeled_jsonl(&[
        position("middlegame", serde_json::json!([obs("7g7f", 1500, 4)])), // too high (>1200)
        position("middlegame", serde_json::json!([obs("7g7f", 100, 4)])),  // OK
        position("middlegame", serde_json::json!([obs("7g7f", -1500, 4)])), // too low (<-1200)
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--eval-min=-1200",
            "--eval-max=1200",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 1);
}

#[test]
fn filter_phase() {
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", 50, 4)])),
        position("endgame", serde_json::json!([obs("7g7f", 50, 4)])),
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--phase",
            "middlegame,endgame",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 2);
}

#[test]
fn filter_end_to_end_with_label() {
    // extract → label (fake engine) → filter with bestmove agreement → all pass
    let pos = NamedTempFile::new().unwrap();
    let obs_file = NamedTempFile::new().unwrap();
    let filtered = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "extract",
            "--input",
            fixture("sample.csa").to_str().unwrap(),
            "--out",
            pos.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            cargo_bin("fake-usi-engine").to_str().unwrap(),
            "--depths",
            "4,6",
            "--out",
            obs_file.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    shogiesa()
        .args([
            "filter",
            "--input",
            obs_file.path().to_str().unwrap(),
            "--out",
            filtered.path().to_str().unwrap(),
            "--require-bestmove-agreement",
            "--eval-min=-1200",
            "--eval-max=1200",
        ])
        .assert()
        .success();

    // fake engine always returns bestmove 7g7f, cp 100 → all 5 positions should pass
    let content = std::fs::read_to_string(filtered.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 5);
}
