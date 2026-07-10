use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;

use assert_cmd::{Command, cargo::cargo_bin};
use predicates::prelude::*;
use tempfile::{NamedTempFile, TempDir};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn shogiesa() -> Command {
    Command::cargo_bin("shogiesa").unwrap()
}

// ponytail: `fake-usi-engine` lives in a sibling crate, so plain `cargo test`
// only builds its unit-test harness, not the plain bin CARGO_BIN_EXE_ needs.
// Build it explicitly once, then reuse assert_cmd's normal lookup.
fn fake_usi_engine_bin() -> std::path::PathBuf {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let status = std::process::Command::new(cargo)
            .args(["build", "-p", "fake-usi-engine"])
            .status()
            .expect("failed to run cargo build");
        assert!(status.success(), "failed to build fake-usi-engine");
    });
    cargo_bin("fake-usi-engine")
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
        assert_eq!(v["schema_version"], shogiesa_core::SCHEMA_VERSION);
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

#[test]
fn report_shows_labeled_diagnostics() {
    let pos = NamedTempFile::new().unwrap();
    let obs = NamedTempFile::new().unwrap();

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
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4,6",
            "--multipv",
            "2",
            "--out",
            obs.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    shogiesa()
        .args(["report", "--input", obs.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("cp/mate ratio"))
        .stdout(predicate::str::contains("avg score swing"))
        .stdout(predicate::str::contains("avg policy margin"))
        .stdout(predicate::str::contains("score swing distribution"))
        .stdout(predicate::str::contains("eval bucket x phase"))
        .stdout(predicate::str::contains("eval bucket x side"))
        .stdout(predicate::str::contains("multipv coverage"))
        .stdout(predicate::str::contains("score bound (multipv candidates)"))
        .stdout(predicate::str::contains("score bound (observations)"))
        .stdout(predicate::str::contains("exact"));
}

#[test]
fn report_hides_multipv_coverage_without_multipv() {
    let pos = NamedTempFile::new().unwrap();
    let obs = NamedTempFile::new().unwrap();

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
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--out",
            obs.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    shogiesa()
        .args(["report", "--input", obs.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("multipv coverage").not());
}

#[test]
fn report_shows_engine_disagreement() {
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

    // engineA's default bestmove (7g7f)
    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--engine-name",
            "engineA",
            "--depths",
            "4",
            "--out",
            obs1.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // engineB, forced to disagree via `setoption name Bestmove value 2g2f` (sent through
    // --engine-option, since label never passes extra argv to the spawned engine)
    shogiesa()
        .args([
            "label",
            "--input",
            obs1.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--engine-name",
            "engineB",
            "--engine-option",
            "Bestmove=2g2f",
            "--depths",
            "4",
            "--out",
            obs2.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    shogiesa()
        .args(["report", "--input", obs2.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "engine disagree:      5  (100.0% of 5 multi-engine positions)",
        ));
}

#[test]
fn report_eval_bucket_normalizes_to_black_perspective() {
    // Black to move, raw cp -250 ("Black is worse by 250") -- black-perspective is also -250.
    // White to move, raw cp +250 ("White is better by 250") -- black-perspective is ALSO -250,
    // since White winning means Black losing. Both belong in the same 200cp bucket (floor(-250/
    // 200)*200 = -400) once normalized to one shared reference frame; if `report` used the raw
    // side-to-move-relative value directly instead, the White-to-move record would land in the
    // +200 bucket instead, splitting what should be one bucket of 2 into two buckets of 1 each.
    let f = make_labeled_jsonl(&[
        position("middlegame", serde_json::json!([obs("7g7f", -250, 4)])),
        position_white_to_move("middlegame", serde_json::json!([obs("3c3d", 250, 4)])),
    ]);
    shogiesa()
        .args(["report", "--input", f.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(" -400.. -201:     2"));
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

// --- schema backward compatibility ---

fn position_with_version(
    schema_version: u32,
    observations: serde_json::Value,
    stability: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut v = serde_json::json!({
        "schema_version": schema_version,
        "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "source": { "kind": "csa", "path": "test.csa", "ply": 1 },
        "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
        "observations": observations,
    });
    if let Some(s) = stability {
        v["stability"] = s;
    }
    v
}

/// Runs `validate --strict` / `report` / `pack`→`unpack` against a schema-vN-shaped JSONL
/// record, asserting every step succeeds and the record survives the pack round-trip. Proves
/// old (lower-`schema_version`) data — missing fields added in later versions — still works
/// under current code, i.e. the `#[serde(default)]` contract on those fields holds.
fn assert_schema_compat(record: serde_json::Value) {
    let f = make_labeled_jsonl(&[record]);

    shogiesa()
        .args([
            "validate",
            "--input",
            f.path().to_str().unwrap(),
            "--strict",
        ])
        .assert()
        .success();

    shogiesa()
        .args(["report", "--input", f.path().to_str().unwrap()])
        .assert()
        .success();

    let packed = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "pack",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            packed.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let unpacked = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "unpack",
            "--input",
            packed.path().to_str().unwrap(),
            "--out",
            unpacked.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(unpacked.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 1);
}

#[test]
fn schema_v1_minimal_round_trips() {
    // v1: no policy_margin_cp, no candidates, no stability at all.
    assert_schema_compat(position_with_version(
        1,
        serde_json::json!([obs("7g7f", 50, 4)]),
        None,
    ));
}

#[test]
fn schema_v2_policy_margin_round_trips() {
    // v2: adds Observation.policy_margin_cp.
    assert_schema_compat(position_with_version(
        2,
        serde_json::json!([obs_with_margin("7g7f", 50, 4, 30)]),
        None,
    ));
}

#[test]
fn schema_v3_engine_stability_round_trips() {
    // v3: adds StabilityInfo.engine_bestmove_agreement / engine_score_swing_cp.
    assert_schema_compat(position_with_version(
        3,
        serde_json::json!([obs_with_margin("7g7f", 50, 4, 30)]),
        Some(serde_json::json!({
            "score_swing_cp": null,
            "bestmove_agreement": true,
            "engine_bestmove_agreement": true,
            "engine_score_swing_cp": 20
        })),
    ));
}

#[test]
fn schema_v4_candidates_round_trips() {
    // v4: adds Observation.candidates / CandidateMove.score_bound.
    let mut observation = obs_with_margin("7g7f", 50, 4, 30);
    observation["candidates"] = serde_json::json!([
        { "multipv": 1, "bestmove": "7g7f", "score": { "kind": "cp", "value": 50 }, "score_bound": "exact", "pv": null },
        { "multipv": 2, "bestmove": "2g2f", "score": { "kind": "cp", "value": 20 }, "score_bound": "lowerbound", "pv": null }
    ]);
    assert_schema_compat(position_with_version(
        4,
        serde_json::json!([observation]),
        Some(serde_json::json!({
            "score_swing_cp": null,
            "bestmove_agreement": true,
            "engine_bestmove_agreement": true,
            "engine_score_swing_cp": 20
        })),
    ));
}

#[test]
fn schema_v5_score_bound_round_trips() {
    // v5: adds Observation.score_bound (top-level, distinct from CandidateMove.score_bound).
    // No "score_bound" key at all here -- proves #[serde(default)] still loads it as Exact.
    assert_schema_compat(position_with_version(
        5,
        serde_json::json!([obs_with_margin("7g7f", 50, 4, 30)]),
        Some(serde_json::json!({
            "score_swing_cp": null,
            "bestmove_agreement": true,
            "engine_bestmove_agreement": true,
            "engine_score_swing_cp": 20
        })),
    ));
}

#[test]
fn schema_v6_source_root_id_round_trips() {
    // v7: adds SourceInfo.root_id/variation_id/branch_from_ply. No such keys at all in
    // `source` here -- proves #[serde(default)] still loads them as None.
    assert_schema_compat(position_with_version(
        6,
        serde_json::json!([obs_with_margin("7g7f", 50, 4, 30)]),
        Some(serde_json::json!({
            "score_swing_cp": null,
            "bestmove_agreement": true,
            "engine_bestmove_agreement": true,
            "engine_score_swing_cp": 20
        })),
    ));
}

#[test]
fn schema_v7_score_perspective_and_bestmove_kind_round_trips() {
    // v8: adds Observation.score_perspective/bestmove_kind. Neither key is present here --
    // proves #[serde(default)] still loads them as SideToMove/None.
    assert_schema_compat(position_with_version(
        7,
        serde_json::json!([obs_with_margin("7g7f", 50, 4, 30)]),
        Some(serde_json::json!({
            "score_swing_cp": null,
            "bestmove_agreement": true,
            "engine_bestmove_agreement": true,
            "engine_score_swing_cp": 20
        })),
    ));
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
            fake_usi_engine_bin().to_str().unwrap(),
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
fn label_multipv_populates_margin() {
    let pos = NamedTempFile::new().unwrap();
    let obs = NamedTempFile::new().unwrap();

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
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--multipv",
            "2",
            "--out",
            obs.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(obs.path()).unwrap();
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["observations"][0]["policy_margin_cp"], 310);
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
            fake_usi_engine_bin().to_str().unwrap(),
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
            fake_usi_engine_bin().to_str().unwrap(),
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

#[test]
fn label_skip_existing_avoids_duplicate() {
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

    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--out",
            obs1.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Re-run with --skip-existing over depths 4 (already covered) and 6 (new)
    shogiesa()
        .args([
            "label",
            "--input",
            obs1.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4,6",
            "--skip-existing",
            "--out",
            obs2.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(obs2.path()).unwrap();
    let first_line = content.lines().next().unwrap();
    let v: serde_json::Value = serde_json::from_str(first_line).unwrap();
    let obs = v["observations"].as_array().unwrap();
    assert_eq!(
        obs.len(),
        2,
        "depth 4 skipped (not duplicated), depth 6 added"
    );
    let depths: Vec<u64> = obs.iter().map(|o| o["depth"].as_u64().unwrap()).collect();
    assert_eq!(depths, vec![4, 6]);
}

#[test]
fn label_skip_existing_no_duplicate_on_early_stop_divergence() {
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

    // `--engine-option EarlyStopDepth=5` sends `setoption name EarlyStopDepth value 5` over
    // stdin, which fake-usi-engine honors the same as its `--early-stop-depth 5` argv flag —
    // `label` only sends USI protocol messages to the spawned engine, never extra argv.
    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--engine-option",
            "EarlyStopDepth=5",
            "--depths",
            "8",
            "--out",
            obs1.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Re-run --skip-existing --depths 8: the prior observation only reached depth 5 (< 8), so
    // this cannot skip and must re-call the engine — which deterministically re-achieves depth
    // 5 again. The post-call dedup (keyed on achieved depth) must replace, not duplicate, it.
    shogiesa()
        .args([
            "label",
            "--input",
            obs1.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--engine-option",
            "EarlyStopDepth=5",
            "--depths",
            "8",
            "--skip-existing",
            "--out",
            obs2.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(obs2.path()).unwrap();
    let first_line = content.lines().next().unwrap();
    let v: serde_json::Value = serde_json::from_str(first_line).unwrap();
    let obs = v["observations"].as_array().unwrap();
    assert_eq!(
        obs.len(),
        1,
        "under-reached depth must be replaced, not duplicated, across skip-existing re-runs"
    );
    assert_eq!(obs[0]["depth"], 5);
}

#[test]
fn label_replace_existing_overwrites() {
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

    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--out",
            obs1.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // Re-label the same depth with --replace-existing: without the flag this would produce 2
    // observations at depth 4 (as label_appends_to_existing_observations demonstrates).
    shogiesa()
        .args([
            "label",
            "--input",
            obs1.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--replace-existing",
            "--out",
            obs2.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(obs2.path()).unwrap();
    let first_line = content.lines().next().unwrap();
    let v: serde_json::Value = serde_json::from_str(first_line).unwrap();
    let obs = v["observations"].as_array().unwrap();
    assert_eq!(obs.len(), 1, "replaced, not duplicated");
    assert_eq!(obs[0]["depth"], 4);
}

#[test]
fn label_jobs_2_is_unordered_by_default_and_preserve_order_matches_input_order() {
    // Each position's sfen move-count field (the trailing token) is distinct (ply + 1) so
    // fake-usi-engine's SlowMoveCount option can single out ply 1 to finish last -- deterministically,
    // not dependent on OS thread-scheduling jitter (a plain uniform-cost fixture previously let this
    // test pass even with the write-order branches swapped; see the sabotage check this test guards
    // against).
    fn tagged_position(ply: u32) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "sfen": format!("lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - {}", ply + 1),
            "source": { "kind": "csa", "path": "test.csa", "ply": ply },
            "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
            "observations": []
        })
    }
    fn plies_of(path: &std::path::Path) -> Vec<u64> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|l| {
                serde_json::from_str::<serde_json::Value>(l).unwrap()["source"]["ply"]
                    .as_u64()
                    .unwrap()
            })
            .collect()
    }

    let positions: Vec<serde_json::Value> = (0..8u32).map(tagged_position).collect();
    let input = make_labeled_jsonl(&positions);
    // ply 1 (sfen move-count 2) is made artificially slow -- 300ms dwarfs every other position's
    // near-instant fake search, so it deterministically finishes last regardless of which worker
    // draws it or how the OS schedules the two worker threads.
    let slow_position_opts = [
        "--engine-option",
        "SlowMoveCount=2",
        "--engine-option",
        "SlowDelayMs=300",
    ];

    // Default (no flag): interrupt-safe, write-as-completed -- same set of records, but order is
    // not guaranteed to match input order.
    let unordered_out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            input.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--jobs",
            "2",
            "--out",
            unordered_out.path().to_str().unwrap(),
        ])
        .args(slow_position_opts)
        .assert()
        .success();
    let plies = plies_of(unordered_out.path());
    let mut sorted_plies = plies.clone();
    sorted_plies.sort_unstable();
    assert_eq!(
        sorted_plies,
        (0..8u64).collect::<Vec<_>>(),
        "default output must still produce the same set of records"
    );
    assert_ne!(
        plies,
        (0..8u64).collect::<Vec<_>>(),
        "default output should reflect completion order, not input order -- ply 1 was made \
         artificially slow so it must land somewhere other than its input position"
    );

    // --preserve-order: opt-in to strict input-order output (trades interrupt-safety for order).
    let ordered_out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            input.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--jobs",
            "2",
            "--preserve-order",
            "--out",
            ordered_out.path().to_str().unwrap(),
        ])
        .args(slow_position_opts)
        .assert()
        .success();
    assert_eq!(
        plies_of(ordered_out.path()),
        (0..8u64).collect::<Vec<_>>(),
        "--preserve-order must produce output order matching input order, even though ply 1 was \
         the last to finish"
    );
}

#[test]
fn label_skip_and_replace_existing_conflict() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--skip-existing",
            "--replace-existing",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .failure();
}

#[test]
fn label_manifest_records_engine_and_depths() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = NamedTempFile::new().unwrap();
    let manifest_path = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4,6",
            "--multipv",
            "2",
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["command"], "label");
    assert_eq!(manifest["engine_name"], "FakeUsiEngine");
    assert_eq!(manifest["depths"], serde_json::json!([4, 6]));
    assert_eq!(manifest["multipv"], 2);
    assert_eq!(manifest["records_read"], 1);
    assert_eq!(manifest["records_kept"], 1);
    assert_eq!(manifest["records_dropped"], 0);
    assert_eq!(manifest["engine_launch_failures"], 0);
    assert_eq!(manifest["timeout_salvaged_count"], 0);
}

#[test]
fn label_manifest_reports_timeout_salvaged_count() {
    // SlowMoveCount/SlowDelayMs (setoption, since `label` never passes extra argv to the engine
    // it spawns) makes the engine sleep before its normal response. `label`'s single
    // `--timeout-ms` value governs both engine-launch handshake AND every `analyse()` call --
    // unlike the shogiesa-usi unit tests, which use a generous fixed handshake timeout and a
    // separate short per-call one -- so this can't be as tight as the unit-level test: 2000ms
    // leaves comfortable headroom for process spawn + handshake even under load, and a 2250ms
    // delay (250ms past the timeout on both sides of the fixed 500ms stop-grace window) still
    // exercises the salvage path end to end through the real `label` CLI without flaking under
    // scheduler jitter.
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = NamedTempFile::new().unwrap();
    let manifest_path = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--timeout-ms",
            "2000",
            "--engine-option",
            "SlowMoveCount=1",
            "--engine-option",
            "SlowDelayMs=2250",
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["timeout_salvaged_count"], 1);

    let output: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(out.path()).unwrap().trim()).unwrap();
    assert_eq!(output["observations"][0]["depth"], 4);
    assert_eq!(
        output["observations"][0]["was_timeout_salvaged"], true,
        "the real timed-out analyse() result must be written through to the observation, not just counted"
    );
}

#[test]
fn label_cache_dir_hits_on_second_run_and_output_matches() {
    let pos = NamedTempFile::new().unwrap();
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

    let cache_dir = TempDir::new().unwrap();
    let out1 = NamedTempFile::new().unwrap();
    let manifest1 = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4,6",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
            "--out",
            out1.path().to_str().unwrap(),
            "--manifest",
            manifest1.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let manifest1: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest1.path()).unwrap()).unwrap();
    assert_eq!(manifest1["cache_hits"], 0, "first run: nothing cached yet");
    let observations_total = manifest1["observations_total"].as_u64().unwrap();
    assert_eq!(manifest1["cache_misses"], observations_total);

    // Re-run against the same (now-populated) cache dir and input, with a fresh --out.
    let out2 = NamedTempFile::new().unwrap();
    let manifest2 = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4,6",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
            "--out",
            out2.path().to_str().unwrap(),
            "--manifest",
            manifest2.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let manifest2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest2.path()).unwrap()).unwrap();
    assert_eq!(
        manifest2["cache_hits"], observations_total,
        "second run: every observation should come from cache"
    );
    assert_eq!(manifest2["cache_misses"], 0);

    assert_eq!(
        std::fs::read_to_string(out1.path()).unwrap(),
        std::fs::read_to_string(out2.path()).unwrap(),
        "cached output must be identical to the freshly-labeled output"
    );
}

#[test]
fn label_manifest_reports_throughput_metrics() {
    let pos = NamedTempFile::new().unwrap();
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

    let out = NamedTempFile::new().unwrap();
    let manifest = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4,6",
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("rec/s"));
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();

    assert!(
        manifest["records_per_sec"].as_f64().unwrap() > 0.0,
        "records_per_sec must be a positive rate"
    );
    // fake-usi-engine always reports a fixed time_ms -- confirms the average is computed from
    // real Observation.time_ms values, not a placeholder.
    assert_eq!(manifest["average_engine_time_ms"], 50.0);
    // No --preserve-order passed -> the (now default) unordered/interrupt-safe mode.
    assert_eq!(manifest["preserve_order"], false);
    assert!(
        manifest.get("cache_hit_rate").is_none(),
        "cache_hit_rate must be absent without --cache-dir"
    );
    assert!(
        manifest.get("worker_count").is_none(),
        "worker_count would duplicate the existing jobs field -- must not exist"
    );
    assert_eq!(manifest["jobs"], 1);
}

#[test]
fn label_manifest_preserve_order_flag_is_reported_true_when_passed() {
    let pos = NamedTempFile::new().unwrap();
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

    let out = NamedTempFile::new().unwrap();
    let manifest = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--preserve-order",
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    assert_eq!(manifest["preserve_order"], true);
}

#[test]
fn label_manifest_cache_hit_rate_reflects_hits_and_misses() {
    let pos = NamedTempFile::new().unwrap();
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

    let cache_dir = TempDir::new().unwrap();
    let out1 = NamedTempFile::new().unwrap();
    let manifest1 = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4,6",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
            "--out",
            out1.path().to_str().unwrap(),
            "--manifest",
            manifest1.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let manifest1: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest1.path()).unwrap()).unwrap();
    assert_eq!(
        manifest1["cache_hit_rate"], 0.0,
        "first run: nothing cached yet"
    );

    let out2 = NamedTempFile::new().unwrap();
    let manifest2 = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4,6",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
            "--out",
            out2.path().to_str().unwrap(),
            "--manifest",
            manifest2.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let manifest2: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest2.path()).unwrap()).unwrap();
    assert_eq!(
        manifest2["cache_hit_rate"], 1.0,
        "second run: every observation cached"
    );
}

/// Looks up a record by `source.ply` in a labeled JSONL file -- needed because `label`'s default
/// unordered output doesn't preserve input line order (see the write-order tests above), so tests
/// asserting per-record outcomes must find records by key, not by line position.
fn record_at_ply(path: &std::path::Path, ply: u32) -> serde_json::Value {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|v| v["source"]["ply"].as_u64() == Some(ply as u64))
        .unwrap_or_else(|| panic!("no record at ply {ply}"))
}

#[test]
fn label_resume_from_skips_already_covered_and_labels_the_rest() {
    // The original full corpus -- as if this were the un-killed input, no observations yet.
    let original = make_labeled_jsonl(&[
        position_at_ply(1, serde_json::json!([])),
        position_at_ply(2, serde_json::json!([])),
        position_at_ply(3, serde_json::json!([])),
        position_at_ply(4, serde_json::json!([])),
    ]);
    // Simulates a killed run's partial --out: only plies 1 and 3 finished before the kill, each
    // already labeled with a bestmove fake-usi-engine's default config would never produce ("7g7f")
    // -- if the final output still shows this bestmove, the engine was never re-invoked for it.
    let partial_out = make_labeled_jsonl(&[
        position_at_ply(
            1,
            serde_json::json!([obs_with_engine("FakeUsiEngine", "9i9h", 999, 4)]),
        ),
        position_at_ply(
            3,
            serde_json::json!([obs_with_engine("FakeUsiEngine", "9i9h", 999, 4)]),
        ),
    ]);

    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            original.path().to_str().unwrap(),
            "--resume-from",
            partial_out.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--jobs",
            "2",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    for ply in [1, 3] {
        let rec = record_at_ply(out.path(), ply);
        let observations = rec["observations"].as_array().unwrap();
        assert_eq!(
            observations.len(),
            1,
            "ply {ply} was already covered -- resuming must not duplicate it"
        );
        assert_eq!(
            observations[0]["bestmove"], "9i9h",
            "ply {ply}'s pre-existing observation must survive untouched, proving the engine \
             was never re-invoked for it"
        );
    }
    for ply in [2, 4] {
        let rec = record_at_ply(out.path(), ply);
        let observations = rec["observations"].as_array().unwrap();
        assert_eq!(observations.len(), 1, "ply {ply} must get freshly labeled");
        assert_eq!(
            observations[0]["bestmove"], "7g7f",
            "ply {ply} wasn't in --resume-from, so it must be labeled by the real engine call"
        );
    }
}

#[test]
fn label_resume_from_tolerates_an_unreadable_record_and_relabels_it() {
    let original = make_labeled_jsonl(&[
        position_at_ply(1, serde_json::json!([])),
        position_at_ply(2, serde_json::json!([])),
    ]);
    // Ply 1's line indexes fine (sfen/source.path/source.ply are all valid), but its
    // "observations" field isn't an array -- the full PositionRecord parse fails when this line
    // is actually read back for its observations. This must not abort the whole run: the
    // position is relabeled from scratch instead, the same tolerance `load_records` (the
    // full-load path this replaced) already had for a broken line.
    let mut partial_out = NamedTempFile::new().unwrap();
    writeln!(
        partial_out,
        "{}",
        serde_json::json!({
            "schema_version": 8,
            "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "source": { "kind": "csa", "path": "test.csa", "ply": 1 },
            "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
            "observations": "not-an-array"
        })
    )
    .unwrap();
    partial_out.flush().unwrap();

    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            original.path().to_str().unwrap(),
            "--resume-from",
            partial_out.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    for ply in [1, 2] {
        let rec = record_at_ply(out.path(), ply);
        let observations = rec["observations"].as_array().unwrap();
        assert_eq!(
            observations.len(),
            1,
            "ply {ply} must still get labeled by the real engine despite the unreadable resume line"
        );
        assert_eq!(observations[0]["bestmove"], "7g7f");
    }
}

#[test]
fn label_resume_from_missing_path_is_a_noop() {
    let input = make_labeled_jsonl(&[position_at_ply(1, serde_json::json!([]))]);
    let missing = std::env::temp_dir().join("shogiesa_test_resume_from_does_not_exist.jsonl");
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            input.path().to_str().unwrap(),
            "--resume-from",
            missing.to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let rec = record_at_ply(out.path(), 1);
    assert_eq!(rec["observations"].as_array().unwrap().len(), 1);
}

#[test]
fn label_resume_from_same_as_out_is_rejected() {
    let input = make_labeled_jsonl(&[position_at_ply(1, serde_json::json!([]))]);
    let same = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            input.path().to_str().unwrap(),
            "--resume-from",
            same.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--out",
            same.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--resume-from must not be the same path as --out",
        ));
}

#[test]
fn label_manifest_reports_resumed_count() {
    let original = make_labeled_jsonl(&[
        position_at_ply(1, serde_json::json!([])),
        position_at_ply(2, serde_json::json!([])),
    ]);
    let partial_out = make_labeled_jsonl(&[position_at_ply(
        1,
        serde_json::json!([obs_with_engine("FakeUsiEngine", "9i9h", 999, 4)]),
    )]);
    let out = NamedTempFile::new().unwrap();
    let manifest = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            original.path().to_str().unwrap(),
            "--resume-from",
            partial_out.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    assert_eq!(manifest["resumed_count"], 1);
    assert_eq!(
        manifest["resume_from"],
        partial_out.path().to_str().unwrap()
    );
}

// --- cache ---

/// Populates a real `label --cache-dir` cache (5 positions x 2 depths = 10 entries, one engine)
/// via the actual extract -> label pipeline, so `cache` subcommand tests exercise the real
/// sharded/atomic file layout `label_cache_path`/`write_cache_entry_atomically` produce, not a
/// hand-approximated one.
fn populate_cache_dir() -> TempDir {
    let pos = NamedTempFile::new().unwrap();
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

    let cache_dir = TempDir::new().unwrap();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4,6",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    cache_dir
}

fn cache_entry_paths(cache_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    for shard in std::fs::read_dir(cache_dir).unwrap() {
        let shard_path = shard.unwrap().path();
        if !shard_path.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&shard_path).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                paths.push(path);
            }
        }
    }
    paths
}

#[test]
fn cache_stats_reports_entry_count_and_engine_distribution() {
    let cache_dir = populate_cache_dir();
    shogiesa()
        .args([
            "cache",
            "stats",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("cache entries : 10"))
        .stdout(predicate::str::contains("FakeUsiEngine"));
}

#[test]
fn cache_verify_detects_corruption_and_does_not_claim_schema_awareness() {
    let cache_dir = populate_cache_dir();
    let entries = cache_entry_paths(cache_dir.path());
    std::fs::write(&entries[0], "{not valid json").unwrap();

    shogiesa()
        .args([
            "cache",
            "verify",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("cache entries : 10"))
        .stdout(predicate::str::contains("corrupted     : 1"))
        .stdout(predicate::str::contains(
            "no schema_version/engine_fingerprint metadata",
        ));
}

#[test]
fn cache_prune_dry_run_deletes_nothing_by_default() {
    let cache_dir = populate_cache_dir();
    let entries = cache_entry_paths(cache_dir.path());
    std::fs::write(&entries[0], "{not valid json").unwrap();

    shogiesa()
        .args([
            "cache",
            "prune",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
            "--corrupted-only",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("dry run: 1/10"));
    assert_eq!(
        cache_entry_paths(cache_dir.path()).len(),
        10,
        "dry run must not delete anything"
    );
}

#[test]
fn cache_prune_yes_corrupted_only_removes_only_corrupted_entries() {
    let cache_dir = populate_cache_dir();
    let entries = cache_entry_paths(cache_dir.path());
    std::fs::write(&entries[0], "{not valid json").unwrap();

    shogiesa()
        .args([
            "cache",
            "prune",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
            "--corrupted-only",
            "--yes",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "deleted 1/1 matched entries (10 total)",
        ));
    let remaining = cache_entry_paths(cache_dir.path());
    assert_eq!(remaining.len(), 9);
    assert!(
        !remaining.contains(&entries[0]),
        "the corrupted entry must be gone"
    );
    for surviving in &entries[1..] {
        assert!(remaining.contains(surviving), "valid entries must survive");
    }
}

#[test]
fn cache_prune_yes_older_than_days_removes_only_aged_entries() {
    let cache_dir = populate_cache_dir();
    let entries = cache_entry_paths(cache_dir.path());

    // Back-date one entry's mtime by 60 days; the rest were just written (age ~0). stdlib-only
    // (File::set_modified, stable since 1.75) -- no new dependency needed.
    let sixty_days_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(60 * 86400);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&entries[0])
        .unwrap();
    file.set_modified(sixty_days_ago).unwrap();

    shogiesa()
        .args([
            "cache",
            "prune",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
            "--older-than-days",
            "30",
            "--yes",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "deleted 1/1 matched entries (10 total)",
        ));
    let remaining = cache_entry_paths(cache_dir.path());
    assert_eq!(remaining.len(), 9);
    assert!(!remaining.contains(&entries[0]));
}

#[test]
fn cache_prune_requires_at_least_one_filter() {
    let cache_dir = populate_cache_dir();
    shogiesa()
        .args([
            "cache",
            "prune",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "requires --corrupted-only, --legacy-only, and/or --older-than-days",
        ));
}

/// A bare-`Observation` JSON payload -- the cache format every entry used before the v2
/// envelope existed, still readable via `parse_cache_entry`'s v1 fallback.
fn legacy_v1_cache_payload() -> String {
    serde_json::json!({
        "engine": "FakeUsiEngine", "engine_version": null, "depth": 4,
        "score": {"kind": "cp", "value": 50}, "bestmove": "7g7f",
        "nodes": null, "time_ms": null, "pv": null
    })
    .to_string()
}

#[test]
fn cache_write_path_produces_v2_envelope_not_bare_observation() {
    let cache_dir = populate_cache_dir();
    let entries = cache_entry_paths(cache_dir.path());
    let content = std::fs::read_to_string(&entries[0]).unwrap();
    let v: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(
        v.get("observation").is_some(),
        "v2 entries wrap the Observation under an `observation` key, not at the top level"
    );
    assert_eq!(v["cache_schema_version"], 1);
    assert_eq!(v["schema_version"], shogiesa_core::SCHEMA_VERSION);
}

#[test]
fn cache_verify_and_stats_distinguish_v1_legacy_from_v2_entries() {
    let cache_dir = populate_cache_dir();
    let entries = cache_entry_paths(cache_dir.path());
    // Simulate a cache dir populated before this round: one pre-existing bare-Observation entry
    // alongside the freshly-written v2 ones.
    std::fs::write(&entries[0], legacy_v1_cache_payload()).unwrap();

    shogiesa()
        .args([
            "cache",
            "verify",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("corrupted     : 0"))
        .stdout(predicate::str::contains("legacy (v1, no metadata): 1"));

    shogiesa()
        .args([
            "cache",
            "stats",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("legacy (v1, no metadata): 1"))
        .stdout(predicate::str::contains(
            "schema_version distribution (v2 entries only):",
        ));
}

#[test]
fn cache_prune_legacy_only_removes_only_v1_entries() {
    let cache_dir = populate_cache_dir();
    let entries = cache_entry_paths(cache_dir.path());
    std::fs::write(&entries[0], legacy_v1_cache_payload()).unwrap();

    shogiesa()
        .args([
            "cache",
            "prune",
            "--cache-dir",
            cache_dir.path().to_str().unwrap(),
            "--legacy-only",
            "--yes",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "deleted 1/1 matched entries (10 total)",
        ));
    let remaining = cache_entry_paths(cache_dir.path());
    assert_eq!(remaining.len(), 9);
    assert!(
        !remaining.contains(&entries[0]),
        "the legacy v1 entry must be gone"
    );
    for surviving in &entries[1..] {
        assert!(
            remaining.contains(surviving),
            "v2 entries must survive --legacy-only"
        );
    }
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

fn obs_with_engine(engine: &str, bestmove: &str, score_cp: i32, depth: u32) -> serde_json::Value {
    serde_json::json!({
        "engine": engine,
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

fn obs_timeout_salvaged(bestmove: &str, score_cp: i32, depth: u32) -> serde_json::Value {
    serde_json::json!({
        "engine": "test",
        "engine_version": null,
        "depth": depth,
        "score": { "kind": "cp", "value": score_cp },
        "bestmove": bestmove,
        "nodes": null,
        "time_ms": null,
        "pv": null,
        "was_timeout_salvaged": true
    })
}

/// A timeout-salvaged mate observation that fell short of `requested_depth` -- the case
/// `require_requested_depth_reached`'s default mate exemption no longer covers (unlike a
/// genuine, non-salvaged early-stop-on-forced-mate), unless `allow_timeout_salvaged_mate` is set.
fn obs_mate_salvaged_underreach(
    bestmove: &str,
    moves: i32,
    depth: u32,
    requested_depth: u32,
) -> serde_json::Value {
    serde_json::json!({
        "engine": "test",
        "engine_version": null,
        "depth": depth,
        "requested_depth": requested_depth,
        "score": { "kind": "mate", "moves": moves },
        "bestmove": bestmove,
        "nodes": null,
        "time_ms": null,
        "pv": null,
        "was_timeout_salvaged": true
    })
}

fn obs_with_margin(bestmove: &str, score_cp: i32, depth: u32, margin: i32) -> serde_json::Value {
    serde_json::json!({
        "engine": "test",
        "engine_version": null,
        "depth": depth,
        "score": { "kind": "cp", "value": score_cp },
        "bestmove": bestmove,
        "nodes": null,
        "time_ms": null,
        "pv": null,
        "policy_margin_cp": margin
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

/// Like `position`, but White to move -- every other fixture in this file is Black to move,
/// under which black-perspective cp conversion is a no-op and couldn't catch a perspective bug
/// even if one existed (`balance --by eval-bucket`/`report`'s eval stats need this to actually
/// exercise the sign flip, not just the Black-to-move identity case).
fn position_white_to_move(phase: &str, observations: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL w - 1",
        "source": { "kind": "csa", "path": "test.csa", "ply": 1 },
        "tags": { "phase": phase, "side_to_move": "white", "in_check": false, "has_capture": false },
        "observations": observations
    })
}

/// Like `position`, but with caller-chosen `sfen`/`source.path` -- needed to build fixtures with
/// deliberately varied (or deliberately duplicated) sfens, which `position`'s single hardcoded
/// sfen can't express, for `sample`/`select`'s hash-ordering golden-output tests.
fn position_with_path(
    sfen: &str,
    path: &str,
    observations: serde_json::Value,
) -> serde_json::Value {
    position_with_path_and_phase(sfen, path, "middlegame", observations)
}

/// Like `position_with_path`, but with a caller-chosen `phase` too -- needed for `balance`'s
/// golden-output tests, which must vary `phase` (its default bucket key) across records.
fn position_with_path_and_phase(
    sfen: &str,
    path: &str,
    phase: &str,
    observations: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "sfen": sfen,
        "source": { "kind": "csa", "path": path, "ply": 1 },
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
fn filter_bestmove_agreement_excludes_resign_from_comparison() {
    // one observation resigns, the other gives an ordinary move -- a resign isn't an opinion
    // about which move is best, so only one ordinary-move observation remains: vacuous agreement.
    let f = make_labeled_jsonl(&[position(
        "middlegame",
        serde_json::json!([obs("resign", 50, 4), obs("7g7f", 55, 6)]),
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
fn label_then_filter_keeps_record_when_one_engine_resigns() {
    let pos = NamedTempFile::new().unwrap();
    let obs1 = NamedTempFile::new().unwrap();
    let obs2 = NamedTempFile::new().unwrap();
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
            fake_usi_engine_bin().to_str().unwrap(),
            "--engine-name",
            "engineA",
            "--depths",
            "4",
            "--out",
            obs1.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // engineB always resigns -- a real USI-reported bestmove_kind, not the legacy-string fallback
    shogiesa()
        .args([
            "label",
            "--input",
            obs1.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--engine-name",
            "engineB",
            "--engine-option",
            "Bestmove=resign",
            "--depths",
            "4",
            "--out",
            obs2.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    shogiesa()
        .args([
            "filter",
            "--input",
            obs2.path().to_str().unwrap(),
            "--out",
            filtered.path().to_str().unwrap(),
            "--require-bestmove-agreement",
        ])
        .assert()
        .success();

    // all 5 positions kept: engineB's resign is excluded from the comparison, not counted as
    // disagreeing with engineA's ordinary move
    let content = std::fs::read_to_string(filtered.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 5);
}

#[test]
fn report_engine_disagreement_excludes_resign() {
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

    shogiesa()
        .args([
            "label",
            "--input",
            pos.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--engine-name",
            "engineA",
            "--depths",
            "4",
            "--out",
            obs1.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    shogiesa()
        .args([
            "label",
            "--input",
            obs1.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--engine-name",
            "engineB",
            "--engine-option",
            "Bestmove=resign",
            "--depths",
            "4",
            "--out",
            obs2.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    shogiesa()
        .args(["report", "--input", obs2.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "engine disagree:      0  (0.0% of 5 multi-engine positions)",
        ));
}

#[test]
fn report_shows_special_bestmove_rate() {
    let f = make_labeled_jsonl(&[
        position("middlegame", serde_json::json!([obs("resign", 50, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", 50, 4)])),
    ]);
    shogiesa()
        .args(["report", "--input", f.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("special bestmove"))
        .stdout(predicate::str::contains("50.0% of labeled"));
}

// PR9 golden baseline: `report`'s full-materialize-then-4-pass (main loop, sources loop,
// candidate_coverage_stats, requested_depth_stats) rewrite to a single streaming pass must
// produce byte-identical stdout. This fixture was generated once via the real
// extract -> label (multipv=2) -> label (2nd engine, forced resign) pipeline against
// tests/fixtures/sample.csa, then hand-augmented with a duplicate sfen, an invalid sfen, a
// side_to_move/SFEN tag mismatch, an unlabeled record, and one malformed JSON line -- covering
// every branch `report` prints, not just the common path. Captured against the pre-refactor
// binary; must still pass after.
const REPORT_GOLDEN_FIXTURE: &str = r#"{"schema_version":8,"sfen":"lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2","source":{"kind":"csa","path":"tests/fixtures/sample.csa","ply":1},"tags":{"phase":"opening","side_to_move":"white","in_check":false,"has_capture":false},"observations":[{"engine":"FakeUsiEngine","engine_version":null,"depth":4,"requested_depth":4,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"7g7f","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"],"policy_margin_cp":310,"candidates":[{"multipv":1,"bestmove":"7g7f","score":{"kind":"cp","value":100},"score_bound":"exact","pv":["7g7f","8h7g"]},{"multipv":2,"bestmove":"2g2f","score":{"kind":"cp","value":-210},"score_bound":"exact","pv":["2g2f","8h7g"]}]},{"engine":"FakeUsiEngine","engine_version":null,"depth":6,"requested_depth":6,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"7g7f","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"],"policy_margin_cp":310,"candidates":[{"multipv":1,"bestmove":"7g7f","score":{"kind":"cp","value":100},"score_bound":"exact","pv":["7g7f","8h7g"]},{"multipv":2,"bestmove":"2g2f","score":{"kind":"cp","value":-210},"score_bound":"exact","pv":["2g2f","8h7g"]}]},{"engine":"engineB","engine_version":null,"depth":4,"requested_depth":4,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"resign","bestmove_kind":"resign","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"]}]}
{"schema_version":8,"sfen":"lnsgkgsnl/1r5b1/pppppp1pp/6p2/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL b - 3","source":{"kind":"csa","path":"tests/fixtures/sample.csa","ply":2},"tags":{"phase":"opening","side_to_move":"black","in_check":false,"has_capture":false},"observations":[{"engine":"FakeUsiEngine","engine_version":null,"depth":4,"requested_depth":4,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"7g7f","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"],"policy_margin_cp":310,"candidates":[{"multipv":1,"bestmove":"7g7f","score":{"kind":"cp","value":100},"score_bound":"exact","pv":["7g7f","8h7g"]},{"multipv":2,"bestmove":"2g2f","score":{"kind":"cp","value":-210},"score_bound":"exact","pv":["2g2f","8h7g"]}]},{"engine":"FakeUsiEngine","engine_version":null,"depth":6,"requested_depth":6,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"7g7f","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"],"policy_margin_cp":310,"candidates":[{"multipv":1,"bestmove":"7g7f","score":{"kind":"cp","value":100},"score_bound":"exact","pv":["7g7f","8h7g"]},{"multipv":2,"bestmove":"2g2f","score":{"kind":"cp","value":-210},"score_bound":"exact","pv":["2g2f","8h7g"]}]},{"engine":"engineB","engine_version":null,"depth":4,"requested_depth":4,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"resign","bestmove_kind":"resign","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"]}]}
{"schema_version":8,"sfen":"lnsgkgsnl/1r5B1/pppppp1pp/6p2/9/2P6/PP1PPPPPP/7R1/LNSGKGSNL w B 4","source":{"kind":"csa","path":"tests/fixtures/sample.csa","ply":3},"tags":{"phase":"opening","side_to_move":"white","in_check":false,"has_capture":true},"observations":[{"engine":"FakeUsiEngine","engine_version":null,"depth":4,"requested_depth":4,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"7g7f","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"],"policy_margin_cp":310,"candidates":[{"multipv":1,"bestmove":"7g7f","score":{"kind":"cp","value":100},"score_bound":"exact","pv":["7g7f","8h7g"]},{"multipv":2,"bestmove":"2g2f","score":{"kind":"cp","value":-210},"score_bound":"exact","pv":["2g2f","8h7g"]}]},{"engine":"FakeUsiEngine","engine_version":null,"depth":6,"requested_depth":6,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"7g7f","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"],"policy_margin_cp":310,"candidates":[{"multipv":1,"bestmove":"7g7f","score":{"kind":"cp","value":100},"score_bound":"exact","pv":["7g7f","8h7g"]},{"multipv":2,"bestmove":"2g2f","score":{"kind":"cp","value":-210},"score_bound":"exact","pv":["2g2f","8h7g"]}]},{"engine":"engineB","engine_version":null,"depth":4,"requested_depth":4,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"resign","bestmove_kind":"resign","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"]}]}
{"schema_version":8,"sfen":"lnsgkg1nl/1r5s1/pppppp1pp/6p2/9/2P6/PP1PPPPPP/7R1/LNSGKGSNL b Bb 5","source":{"kind":"csa","path":"tests/fixtures/sample.csa","ply":4},"tags":{"phase":"opening","side_to_move":"black","in_check":false,"has_capture":true},"observations":[{"engine":"FakeUsiEngine","engine_version":null,"depth":4,"requested_depth":4,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"7g7f","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"],"policy_margin_cp":310,"candidates":[{"multipv":1,"bestmove":"7g7f","score":{"kind":"cp","value":100},"score_bound":"exact","pv":["7g7f","8h7g"]},{"multipv":2,"bestmove":"2g2f","score":{"kind":"cp","value":-210},"score_bound":"exact","pv":["2g2f","8h7g"]}]},{"engine":"FakeUsiEngine","engine_version":null,"depth":6,"requested_depth":6,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"7g7f","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"],"policy_margin_cp":310,"candidates":[{"multipv":1,"bestmove":"7g7f","score":{"kind":"cp","value":100},"score_bound":"exact","pv":["7g7f","8h7g"]},{"multipv":2,"bestmove":"2g2f","score":{"kind":"cp","value":-210},"score_bound":"exact","pv":["2g2f","8h7g"]}]},{"engine":"engineB","engine_version":null,"depth":4,"requested_depth":4,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"resign","bestmove_kind":"resign","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"]}]}
{"schema_version":8,"sfen":"lnsgkg1nl/1r5s1/pppppp1pp/6p2/5B3/2P6/PP1PPPPPP/7R1/LNSGKGSNL w b 6","source":{"kind":"csa","path":"tests/fixtures/sample.csa","ply":5},"tags":{"phase":"opening","side_to_move":"white","in_check":false,"has_capture":false},"observations":[{"engine":"FakeUsiEngine","engine_version":null,"depth":4,"requested_depth":4,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"7g7f","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"],"policy_margin_cp":310,"candidates":[{"multipv":1,"bestmove":"7g7f","score":{"kind":"cp","value":100},"score_bound":"exact","pv":["7g7f","8h7g"]},{"multipv":2,"bestmove":"2g2f","score":{"kind":"cp","value":-210},"score_bound":"exact","pv":["2g2f","8h7g"]}]},{"engine":"FakeUsiEngine","engine_version":null,"depth":6,"requested_depth":6,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"7g7f","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"],"policy_margin_cp":310,"candidates":[{"multipv":1,"bestmove":"7g7f","score":{"kind":"cp","value":100},"score_bound":"exact","pv":["7g7f","8h7g"]},{"multipv":2,"bestmove":"2g2f","score":{"kind":"cp","value":-210},"score_bound":"exact","pv":["2g2f","8h7g"]}]},{"engine":"engineB","engine_version":null,"depth":4,"requested_depth":4,"score":{"kind":"cp","value":100},"score_perspective":"side_to_move","score_bound":"exact","bestmove":"resign","bestmove_kind":"resign","nodes":1000,"time_ms":50,"pv":["7g7f","8h7g"]}]}
{"schema_version": 8, "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2", "source": {"kind": "csa", "path": "dup_source.csa", "ply": 1}, "tags": {"phase": "opening", "side_to_move": "white", "in_check": false, "has_capture": false}, "observations": [{"engine": "FakeUsiEngine", "engine_version": null, "depth": 4, "requested_depth": 4, "score": {"kind": "cp", "value": 100}, "score_perspective": "side_to_move", "score_bound": "exact", "bestmove": "7g7f", "nodes": 1000, "time_ms": 50, "pv": ["7g7f", "8h7g"], "policy_margin_cp": 310, "candidates": [{"multipv": 1, "bestmove": "7g7f", "score": {"kind": "cp", "value": 100}, "score_bound": "exact", "pv": ["7g7f", "8h7g"]}, {"multipv": 2, "bestmove": "2g2f", "score": {"kind": "cp", "value": -210}, "score_bound": "exact", "pv": ["2g2f", "8h7g"]}]}, {"engine": "FakeUsiEngine", "engine_version": null, "depth": 6, "requested_depth": 6, "score": {"kind": "cp", "value": 100}, "score_perspective": "side_to_move", "score_bound": "exact", "bestmove": "7g7f", "nodes": 1000, "time_ms": 50, "pv": ["7g7f", "8h7g"], "policy_margin_cp": 310, "candidates": [{"multipv": 1, "bestmove": "7g7f", "score": {"kind": "cp", "value": 100}, "score_bound": "exact", "pv": ["7g7f", "8h7g"]}, {"multipv": 2, "bestmove": "2g2f", "score": {"kind": "cp", "value": -210}, "score_bound": "exact", "pv": ["2g2f", "8h7g"]}]}, {"engine": "engineB", "engine_version": null, "depth": 4, "requested_depth": 4, "score": {"kind": "cp", "value": 100}, "score_perspective": "side_to_move", "score_bound": "exact", "bestmove": "resign", "bestmove_kind": "resign", "nodes": 1000, "time_ms": 50, "pv": ["7g7f", "8h7g"]}]}
{"schema_version": 8, "sfen": "not-a-valid-sfen", "source": {"kind": "csa", "path": "invalid.csa", "ply": 1}, "tags": {"phase": "endgame", "side_to_move": "black", "in_check": false, "has_capture": false}, "observations": []}
{"schema_version": 8, "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1", "source": {"kind": "csa", "path": "mismatch.csa", "ply": 1}, "tags": {"phase": "middlegame", "side_to_move": "white", "in_check": false, "has_capture": false}, "observations": []}
{this is not valid json
"#;

const REPORT_GOLDEN_STDOUT: &str = r#"=== shogiesa report ===
positions      : 8
broken lines   : 1
ply range      : 1–5 (avg 2.2)
invalid SFENs  : 1
duplicate SFENs: 1
tag mismatches : 1  (side_to_move vs SFEN)

schema versions: {8: 8}

phase distribution:
  endgame           1  (12.5%)
  middlegame        1  (12.5%)
  opening           6  (75.0%)

side to move:
  black             3  (37.5%)
  white             5  (62.5%)

tag ratios:
  in-check            0  (0.0%)
  capture             2  (25.0%)

source files: 4
  dup_source.csa: 1
  invalid.csa: 1
  mismatch.csa: 1
  tests/fixtures/sample.csa: 5

source dominance:
  top source     : 62.5%  WARN: too concentrated

balance warnings:
  opening ratio  : 75.0%  WARN: too high
  side imbalance : 37.5% / 62.5%  OK
  duplicate rate : 12.5%  WARN: too high

observations:
  labeled        :      6  (75.0%)
  unlabeled      :      2  (25.0%)
  depth disagree :      0  (0.0% of labeled)
  engine disagree:      0  (0.0% of 6 multi-engine positions)
  special bestmove:     6  (100.0% of labeled; resign/win/none)
  depth counts:
    depth  4     :     12
    depth  6     :      6
  cp/mate ratio  : 18 cp / 0 mate  (0.0% mate)
  score bound (observations):
    exact      :     18
  avg score swing: 0.0cp  (over 6 records with ≥2 cp observations)
  avg policy margin: 310.0cp  (over 12 observations)
  multipv coverage:     12  (66.7% of 18 observations)
  score bound (multipv candidates):
    exact      :     24
  requested-depth underreach:      0  (0.0% of 18 observations with a requested_depth)

eval distribution (200cp buckets, deepest observation):
  unlabeled  :     2  ██████████
   -200..   -1:     4  ████████████████████
     +0.. +199:     2  ██████████

score swing distribution (50cp buckets, per record):
     0..49  :     6  ████████████████████

eval bucket x phase:
                     endgame  middlegame     opening
  unlabeled                1           1           0
  -200..-1                 0           0           4
  +0..+199                 0           0           2

eval bucket x side:
                       black       white
  unlabeled                1           1
  -200..-1                 0           4
  +0..+199                 2           0
"#;

#[test]
fn report_bounded_streaming_matches_pre_refactor_golden_output() {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(REPORT_GOLDEN_FIXTURE.as_bytes()).unwrap();
    shogiesa()
        .args(["report", "--input", f.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(REPORT_GOLDEN_STDOUT);
}

#[test]
fn stability_excludes_resign_from_bestmove_agreement() {
    let f = make_labeled_jsonl(&[position(
        "middlegame",
        serde_json::json!([obs("resign", 50, 4), obs("2b3c", 300, 6)]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "stability",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let v: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
    assert_eq!(
        v["stability"]["bestmove_agreement"], true,
        "a resign isn't a disagreeing move -- only one ordinary-move observation remains"
    );
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
fn filter_require_engine_agreement_single_engine_passes() {
    // Only one engine represented — engine_bestmove_agreement is None, so the gate is a no-op
    // even though these two observations (from the same engine, different depths) disagree.
    let f = make_labeled_jsonl(&[position(
        "middlegame",
        serde_json::json!([
            obs_with_engine("engineA", "7g7f", 50, 4),
            obs_with_engine("engineA", "2g2f", 55, 6),
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
            "--require-engine-agreement",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 1);
}

#[test]
fn filter_require_engine_agreement_excludes_disagreement() {
    let f = make_labeled_jsonl(&[position(
        "middlegame",
        serde_json::json!([
            obs_with_engine("engineA", "7g7f", 50, 4),
            obs_with_engine("engineB", "2g2f", 55, 4), // different engine, different bestmove
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
            "--require-engine-agreement",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 0);
}

#[test]
fn filter_max_engine_score_swing_cp_excludes_large_cross_engine_swing() {
    let f = make_labeled_jsonl(&[position(
        "middlegame",
        serde_json::json!([
            obs_with_engine("engineA", "7g7f", 50, 4),
            obs_with_engine("engineB", "7g7f", 300, 4), // agrees on bestmove, swing = 250cp
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
            "--max-engine-score-swing-cp",
            "150",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 0);
}

#[test]
fn filter_min_policy_margin_cp() {
    let f = make_labeled_jsonl(&[
        position(
            "middlegame",
            serde_json::json!([obs_with_margin("7g7f", 100, 4, 20)]),
        ), // low margin, excluded
        position(
            "middlegame",
            serde_json::json!([obs_with_margin("8h2b+", 350, 4, 310)]),
        ), // high margin, kept
        position("middlegame", serde_json::json!([obs("7g7f", 100, 4)])), // no margin computed, kept
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--min-policy-margin-cp",
            "50",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 2);
}

#[test]
fn filter_require_exact_score_excludes_non_exact() {
    let mut non_exact = obs("7g7f", 100, 4);
    non_exact["score_bound"] = serde_json::json!("lowerbound");
    let f = make_labeled_jsonl(&[
        position("middlegame", serde_json::json!([non_exact])),
        position("middlegame", serde_json::json!([obs("8h2b+", 100, 4)])), // exact (default), kept
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--require-exact-score",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 1);
}

#[test]
fn filter_require_policy_margin_excludes_missing_margin() {
    let f = make_labeled_jsonl(&[
        position("middlegame", serde_json::json!([obs("7g7f", 100, 4)])), // no margin, excluded
        position(
            "middlegame",
            serde_json::json!([obs_with_margin("8h2b+", 100, 4, 50)]),
        ), // has margin, kept
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--require-policy-margin",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 1);
}

#[test]
fn filter_min_depth_reached_excludes_shallow_but_exempts_mate() {
    let f = make_labeled_jsonl(&[
        position("middlegame", serde_json::json!([obs("7g7f", 100, 6)])), // shallow, excluded
        position("middlegame", serde_json::json!([obs("8h2b+", 100, 10)])), // deep enough, kept
        position("middlegame", serde_json::json!([obs_mate("7g7f", 3, 6)])), // shallow mate, kept
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--min-depth-reached",
            "10",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 2);
}

#[test]
fn filter_explain_out_records_rejected_with_full_reasons() {
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("opening", serde_json::json!([obs_mate("7g7f", 3, 4)])),
    ]);
    let out = NamedTempFile::new().unwrap();
    let explain_out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--exclude-mate",
            "--explain-out",
            explain_out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let kept = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(kept.lines().filter(|l| !l.trim().is_empty()).count(), 1);

    let rejected: Vec<serde_json::Value> = std::fs::read_to_string(explain_out.path())
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0]["quality"]["keep"], false);
    assert_eq!(
        rejected[0]["quality"]["reasons"],
        serde_json::json!(["mate"])
    );
    assert!(rejected[0]["record"]["observations"].is_array());
}

#[test]
fn filter_explain_out_works_standalone_with_dry_run() {
    let f = make_labeled_jsonl(&[position(
        "opening",
        serde_json::json!([obs_mate("7g7f", 3, 4)]),
    )]);
    let explain_out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--exclude-mate",
            "--dry-run",
            "--explain-out",
            explain_out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let rejected = std::fs::read_to_string(explain_out.path()).unwrap();
    assert_eq!(rejected.lines().filter(|l| !l.trim().is_empty()).count(), 1);
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
fn filter_exclude_timeout_salvaged() {
    let f = make_labeled_jsonl(&[
        position("middlegame", serde_json::json!([obs("7g7f", 50, 4)])),
        position(
            "middlegame",
            serde_json::json!([obs_timeout_salvaged("7g7f", 50, 4)]),
        ),
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--exclude-timeout-salvaged",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 1);
}

#[test]
fn filter_require_requested_depth_reached_rejects_salvaged_mate_underreach_by_default() {
    let f = make_labeled_jsonl(&[position(
        "endgame",
        serde_json::json!([obs_mate_salvaged_underreach("7g7f", 3, 8, 12)]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--require-requested-depth-reached",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 0);
}

#[test]
fn filter_allow_timeout_salvaged_mate_keeps_the_record() {
    let f = make_labeled_jsonl(&[position(
        "endgame",
        serde_json::json!([obs_mate_salvaged_underreach("7g7f", 3, 8, 12)]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--require-requested-depth-reached",
            "--allow-timeout-salvaged-mate",
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

fn position_with_flags(in_check: bool, has_capture: bool) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "source": { "kind": "csa", "path": "test.csa", "ply": 1 },
        "tags": { "phase": "middlegame", "side_to_move": "black", "in_check": in_check, "has_capture": has_capture },
        "observations": [obs("7g7f", 50, 4)]
    })
}

#[test]
fn filter_exclude_in_check() {
    let f = make_labeled_jsonl(&[
        position_with_flags(false, false),
        position_with_flags(true, false),
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--exclude-in-check",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 1);
}

#[test]
fn filter_exclude_capture() {
    let f = make_labeled_jsonl(&[
        position_with_flags(false, false),
        position_with_flags(false, true),
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--exclude-capture",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 1);
}

#[test]
fn filter_preset_loads_config_from_tune_output() {
    // pareto_fixture()'s "broad" candidate (max coverage on the frontier) is threshold 0 -- see
    // tune_csv_grid_has_expected_coverage_and_mismatch_per_threshold's own hand-verified table.
    // A min_policy_margin_cp=0 gate keeps every record (all margins are >= 0), so `filter
    // --preset ...:broad` must produce byte-identical output to an explicit
    // `--min-policy-margin-cp 0` run on the same input.
    let f = pareto_fixture();
    let tune_out = NamedTempFile::new().unwrap();
    let preset = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "tune",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--sweep-policy-margin",
            "0,150,350,450",
            "--out",
            tune_out.path().to_str().unwrap(),
            "--preset-out",
            preset.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let via_preset = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            via_preset.path().to_str().unwrap(),
            "--preset",
            &format!("{}:broad", preset.path().to_str().unwrap()),
        ])
        .assert()
        .success();

    let via_flags = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            via_flags.path().to_str().unwrap(),
            "--min-policy-margin-cp",
            "0",
        ])
        .assert()
        .success();

    let preset_content = std::fs::read_to_string(via_preset.path()).unwrap();
    let flags_content = std::fs::read_to_string(via_flags.path()).unwrap();
    assert_eq!(preset_content, flags_content);
    assert_eq!(
        preset_content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count(),
        6
    );
}

#[test]
fn filter_preset_unknown_label_errors() {
    let f = pareto_fixture();
    let tune_out = NamedTempFile::new().unwrap();
    let preset = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "tune",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--sweep-policy-margin",
            "0,150,350",
            "--out",
            tune_out.path().to_str().unwrap(),
            "--preset-out",
            preset.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let filtered_out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            filtered_out.path().to_str().unwrap(),
            "--preset",
            &format!("{}:nonexistent", preset.path().to_str().unwrap()),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nonexistent"));
}

#[test]
fn filter_preset_conflicts_with_individual_flags() {
    let f = pareto_fixture();
    let preset = NamedTempFile::new().unwrap();
    std::fs::write(preset.path(), "{}").unwrap();
    let filtered_out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            filtered_out.path().to_str().unwrap(),
            "--preset",
            &format!("{}:broad", preset.path().to_str().unwrap()),
            "--exclude-mate",
        ])
        .assert()
        .failure();
}

// --- calibrate ---

#[test]
fn calibrate_sweep_policy_margin_produces_golden_csv_rows() {
    // margins 50 and 150; sweeping thresholds 0/100/200 crosses each record's margin exactly once.
    let f = make_labeled_jsonl(&[
        position(
            "opening",
            serde_json::json!([obs_with_margin("7g7f", 50, 4, 50)]),
        ),
        position(
            "opening",
            serde_json::json!([obs_with_margin("7g7f", 50, 4, 150)]),
        ),
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "calibrate",
            "--input",
            f.path().to_str().unwrap(),
            "--sweep-policy-margin",
            "0,100,200",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(
        lines[0],
        "sweep_param,sweep_value,total,kept,dropped,coverage_pct,drop_reasons"
    );
    assert_eq!(lines[1], "policy_margin,0,2,2,0,100.00,");
    assert_eq!(lines[2], "policy_margin,100,2,1,1,50.00,policy_margin=1");
    assert_eq!(lines[3], "policy_margin,200,2,0,2,0.00,policy_margin=2");
}

#[test]
fn calibrate_sweep_score_swing_produces_golden_csv_rows() {
    // swing = 250 (50->300); sweeping thresholds 100/300 crosses it exactly once.
    let f = make_labeled_jsonl(&[position(
        "opening",
        serde_json::json!([obs("7g7f", 50, 4), obs("7g7f", 300, 6)]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "calibrate",
            "--input",
            f.path().to_str().unwrap(),
            "--sweep-score-swing",
            "100,300",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines[1], "score_swing,100,1,0,1,0.00,score_swing=1");
    assert_eq!(lines[2], "score_swing,300,1,1,0,100.00,");
}

#[test]
fn calibrate_reports_dataset_wide_diagnostics_independent_of_sweep() {
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("resign", 50, 4)])),
        position(
            "opening",
            serde_json::json!([obs_with_margin("7g7f", 50, 4, 80)]),
        ),
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "calibrate",
            "--input",
            f.path().to_str().unwrap(),
            "--sweep-policy-margin",
            "0",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "special bestmove:      1  (50.0% of labeled)",
        ))
        .stderr(predicate::str::contains("exact      :      2"))
        .stderr(predicate::str::contains("50..99  :      1"));
}

#[test]
fn calibrate_requires_at_least_one_sweep_flag() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([obs("7g7f", 50, 4)]))]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "calibrate",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "requires at least one of --sweep-policy-margin/--sweep-score-swing",
        ));
}

#[test]
fn calibrate_sweep_and_hold_flag_on_the_same_field_are_mutually_exclusive() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([obs("7g7f", 50, 4)]))]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "calibrate",
            "--input",
            f.path().to_str().unwrap(),
            "--sweep-policy-margin",
            "0,100",
            "--min-policy-margin-cp",
            "50",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

// --- audit ---

/// Full-control observation builder for `audit` tests -- unlike `obs`/`obs_mate`/`obs_with_margin`
/// (which never set `requested_depth`, always exercising the `depth`-fallback match rule), this
/// lets a test set an explicit `requested_depth` distinct from `depth` (e.g. to simulate a
/// mate-early-stop, or to prove the primary requested_depth-based match rule specifically).
fn audit_obs(
    engine: &str,
    bestmove: &str,
    score: serde_json::Value,
    depth: u32,
    requested_depth: Option<u32>,
) -> serde_json::Value {
    serde_json::json!({
        "engine": engine,
        "engine_version": null,
        "depth": depth,
        "requested_depth": requested_depth,
        "score": score,
        "bestmove": bestmove,
        "nodes": null,
        "time_ms": null,
        "pv": null
    })
}

fn cp(value: i32) -> serde_json::Value {
    serde_json::json!({ "kind": "cp", "value": value })
}

fn mate(moves: i32) -> serde_json::Value {
    serde_json::json!({ "kind": "mate", "moves": moves })
}

#[test]
fn audit_groups_by_engine_not_across_engines() {
    let f = make_labeled_jsonl(&[position(
        "opening",
        serde_json::json!([
            audit_obs("engineA", "7g7f", cp(100), 14, Some(14)),
            audit_obs("engineA", "7g7f", cp(100), 6, Some(6)),
            audit_obs("engineB", "2g2f", cp(-500), 14, Some(14)),
            audit_obs("engineB", "7g7f", cp(100), 6, Some(6)),
        ]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "audit",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("2 pairs compared"));

    let content = std::fs::read_to_string(out.path()).unwrap();
    let lines: Vec<serde_json::Value> = content
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        lines.len(),
        2,
        "one pair per engine, not a cross-engine product"
    );
    let by_engine: std::collections::HashMap<&str, &serde_json::Value> = lines
        .iter()
        .map(|v| (v["engine"].as_str().unwrap(), v))
        .collect();
    assert_eq!(by_engine["engineA"]["bestmove_match"], true);
    assert_eq!(by_engine["engineA"]["score_error_cp"], 0);
    assert_eq!(by_engine["engineB"]["bestmove_match"], false);
    assert_eq!(by_engine["engineB"]["score_error_cp"], -600);
}

#[test]
fn audit_falls_back_to_achieved_depth_when_requested_depth_is_absent() {
    // Legacy pre-schema-v6 data: requested_depth is always null, so the match must fall back to
    // matching on the achieved `depth` directly.
    let f = make_labeled_jsonl(&[position(
        "opening",
        serde_json::json!([
            audit_obs("engineA", "7g7f", cp(100), 14, None),
            audit_obs("engineA", "7g7f", cp(80), 6, None),
        ]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "audit",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("1 pairs compared"));
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().count(), 1);
    let v: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
    assert_eq!(v["score_error_cp"], 20);
}

#[test]
fn audit_uses_a_short_mate_teacher_without_flagging_underreach() {
    // Teacher requested depth 14 but stopped at 9 on a forced mate -- still used as the teacher,
    // and NOT flagged as underreach (mate-exemption, same as evaluate_quality's own gate).
    let f = make_labeled_jsonl(&[position(
        "opening",
        serde_json::json!([
            audit_obs("engineA", "7g7f", mate(3), 9, Some(14)),
            audit_obs("engineA", "7g7f", cp(100), 6, Some(6)),
        ]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "audit",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    let v: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
    assert_eq!(v["teacher_depth"], 9);
    assert_eq!(v["teacher_underreach"], false);
    assert!(
        v["score_error_cp"].is_null(),
        "mate teacher has no cp to compare"
    );
}

#[test]
fn audit_bestmove_match_is_vacuous_when_student_resigns() {
    // Reuses bestmove_agreement's existing resign-exclusion semantics, not a new audit-specific
    // rule: only one ordinary-move observation remains once the resign is excluded, so it's a
    // vacuous match despite the literal bestmove strings differing.
    let f = make_labeled_jsonl(&[position(
        "opening",
        serde_json::json!([
            audit_obs("engineA", "7g7f", cp(100), 14, Some(14)),
            audit_obs("engineA", "resign", cp(100), 6, Some(6)),
        ]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "audit",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    let v: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
    assert_eq!(v["bestmove_match"], true);
}

#[test]
fn audit_score_error_cp_normalizes_through_black_perspective() {
    // White to move: teacher raw cp 100 (good for White) -> black-perspective -100; student raw
    // cp -50 (bad for White) -> black-perspective +50. score_error = -100 - 50 = -150, NOT the raw
    // difference (100 - (-50) = 150, wrong sign) -- proves the comparison goes through
    // cp_from_black_perspective rather than subtracting raw side-to-move-relative values.
    let f = make_labeled_jsonl(&[position_white_to_move(
        "opening",
        serde_json::json!([
            audit_obs("engineA", "7g7f", cp(100), 14, Some(14)),
            audit_obs("engineA", "7g7f", cp(-50), 6, Some(6)),
        ]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "audit",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    let v: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
    assert_eq!(v["score_error_cp"], -150);
}

// --- tune ---

/// Observation with both `requested_depth` (needed to match teacher/student depths, like
/// `audit_obs`) and `policy_margin_cp` (needed to drive `--sweep-policy-margin`, like
/// `obs_with_margin`) under caller control -- neither existing helper has both.
fn tune_obs(
    bestmove: &str,
    cp_value: i32,
    depth: u32,
    requested_depth: u32,
    policy_margin: i32,
) -> serde_json::Value {
    serde_json::json!({
        "engine": "e",
        "engine_version": null,
        "depth": depth,
        "requested_depth": requested_depth,
        "score": { "kind": "cp", "value": cp_value },
        "bestmove": bestmove,
        "nodes": null,
        "time_ms": null,
        "pv": null,
        "policy_margin_cp": policy_margin
    })
}

/// 6 records, each with a teacher (depth 14) and student (depth 6) observation sharing one
/// policy_margin_cp -- margins 500/400/300/200/100/50, with the student disagreeing with the
/// teacher's bestmove exactly on the 300 and 50 margin records. Sweeping `--sweep-policy-margin
/// 0,150,350,450` against this produces a genuine (not degenerate) 3-point Pareto frontier:
/// coverage strictly decreases as the threshold rises, but mismatch rate drops faster than
/// coverage does, so no single point dominates every other -- verified by hand (and against this
/// exact fixture's pre-refactor behavior) before being written down as a permanent test.
fn pareto_fixture() -> NamedTempFile {
    make_labeled_jsonl(&[
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 500),
                tune_obs("7g7f", 100, 6, 6, 500),
            ]),
        ),
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 400),
                tune_obs("7g7f", 100, 6, 6, 400),
            ]),
        ),
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 300),
                tune_obs("2g2f", 100, 6, 6, 300), // disagrees with teacher
            ]),
        ),
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 200),
                tune_obs("7g7f", 100, 6, 6, 200),
            ]),
        ),
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 100),
                tune_obs("2g2f", 100, 6, 6, 100), // disagrees with teacher
            ]),
        ),
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 50),
                tune_obs("2g2f", 100, 6, 6, 50), // disagrees with teacher
            ]),
        ),
    ])
}

#[test]
fn tune_csv_grid_has_expected_coverage_and_mismatch_per_threshold() {
    let f = pareto_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "tune",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--sweep-policy-margin",
            "0,150,350,450",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(
        lines[0],
        "policy_margin,score_swing,total,kept,dropped,coverage_pct,drop_reasons,audit_pairs,\
         teacher_bestmove_mismatch_pct,avg_abs_score_error_cp,max_abs_score_error_cp,\
         teacher_non_exact_pct,student_non_exact_pct,teacher_underreach_pct,student_underreach_pct,\
         teacher_special_bestmove_pct,student_special_bestmove_pct"
    );
    // threshold 0: all 6 kept, 3/6 disagree
    assert_eq!(
        lines[1],
        "0,,6,6,0,100.00,,6,50.00,0.00,0,0.00,0.00,0.00,0.00,0.00,0.00"
    );
    // threshold 150: margins {500,400,300,200} kept (4), of which {300} disagrees -> 1/4
    assert_eq!(
        lines[2],
        "150,,6,4,2,66.67,policy_margin=2,4,25.00,0.00,0,0.00,0.00,0.00,0.00,0.00,0.00"
    );
    // threshold 350: margins {500,400} kept (2), both agree -> 0/2
    assert_eq!(
        lines[3],
        "350,,6,2,4,33.33,policy_margin=4,2,0.00,0.00,0,0.00,0.00,0.00,0.00,0.00,0.00"
    );
    // threshold 450: margin {500} kept (1), agrees -> 0/1 -- dominated on the frontier by 350's
    // point (higher coverage, same 0% mismatch), but still a real, correctly computed CSV row.
    assert_eq!(
        lines[4],
        "450,,6,1,5,16.67,policy_margin=5,1,0.00,0.00,0,0.00,0.00,0.00,0.00,0.00,0.00"
    );
}

/// 4 records combining two independent axes -- policy_margin in {500,100} and score_swing in
/// {0,200} -- so a 2x2 sweep exercises `tune`'s defining behavior: one combined threshold gate
/// per cell (both axes ANDed together), not two independent 1D sweeps like `calibrate`'s. If the
/// grid degenerated into independent sweeps, no cell's kept-count could depend on both axes at
/// once; here all four combinations produce distinct, hand-verified counts.
fn cartesian_fixture() -> NamedTempFile {
    make_labeled_jsonl(&[
        // policy_margin=500, swing=0 (teacher/student cp agree)
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 500),
                tune_obs("7g7f", 100, 6, 6, 500),
            ]),
        ),
        // policy_margin=500, swing=200
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 500),
                tune_obs("7g7f", 300, 6, 6, 500),
            ]),
        ),
        // policy_margin=100, swing=0
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 100),
                tune_obs("7g7f", 100, 6, 6, 100),
            ]),
        ),
        // policy_margin=100, swing=200
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 100),
                tune_obs("7g7f", 300, 6, 6, 100),
            ]),
        ),
    ])
}

#[test]
fn tune_grid_is_a_full_cartesian_product_of_both_swept_axes() {
    let f = cartesian_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "tune",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--sweep-policy-margin",
            "0,300",
            "--sweep-score-swing",
            "50,250",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    let rows: Vec<&str> = content.lines().skip(1).collect();
    // 2 policy_margin values x 2 score_swing values must yield exactly 4 combined-threshold
    // rows -- the row-major loop nesting is the thing under test, not just the row count.
    assert_eq!(rows.len(), 4);

    let kept_for = |policy: &str, swing: &str| -> u32 {
        let prefix = format!("{policy},{swing},");
        let row = rows
            .iter()
            .find(|r| r.starts_with(&prefix))
            .unwrap_or_else(|| panic!("missing combined-grid row for {prefix}"));
        row.split(',').nth(3).unwrap().parse().unwrap() // kept column
    };

    // Every row has BOTH axes populated with a real (non-empty) value -- independent 1D sweeps
    // (calibrate's shape) could never produce a row with both columns set at once.
    assert_eq!(kept_for("0", "50"), 2); // margin>=0 & swing<=50: the two swing=0 records
    assert_eq!(kept_for("0", "250"), 4); // margin>=0 & swing<=250: all 4 records
    assert_eq!(kept_for("300", "50"), 1); // margin>=300 & swing<=50: only the margin=500,swing=0 record
    assert_eq!(kept_for("300", "250"), 2); // margin>=300 & swing<=250: both margin=500 records
}

#[test]
fn tune_report_produces_three_distinct_pareto_candidates() {
    let f = pareto_fixture();
    let out = NamedTempFile::new().unwrap();
    let report = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "tune",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--sweep-policy-margin",
            "0,150,350,450",
            "--out",
            out.path().to_str().unwrap(),
            "--report",
            report.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(report.path()).unwrap();
    assert!(content.contains("| broad | 0 |"));
    assert!(content.contains("| balanced | 150 |"));
    assert!(content.contains("| strict | 350 |"));
    // the dominated threshold=450 point (lower coverage, same 0% mismatch as 350) must not be
    // labeled as any candidate, even though it legitimately still appears in the Full grid
    // listing below -- extract just the "Recommended candidates" table to check.
    let candidates_section = content
        .split("## Recommended candidates")
        .nth(1)
        .unwrap()
        .split("## Pareto frontier")
        .next()
        .unwrap();
    assert!(!candidates_section.contains("450"));
    assert!(content.contains("Why three candidates, not one"));
}

#[test]
fn tune_report_collapses_candidates_when_frontier_has_one_point() {
    // Every record identical (no coverage/mismatch variation at all) -- the frontier has exactly
    // one point, so all three candidate roles coincide there instead of fabricating distinctness.
    let f = make_labeled_jsonl(&[
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 500),
                tune_obs("7g7f", 100, 6, 6, 500),
            ]),
        ),
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 500),
                tune_obs("7g7f", 100, 6, 6, 500),
            ]),
        ),
    ]);
    let out = NamedTempFile::new().unwrap();
    let report = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "tune",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--sweep-policy-margin",
            "0,100",
            "--out",
            out.path().to_str().unwrap(),
            "--report",
            report.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(report.path()).unwrap();
    assert!(content.contains("broad, balanced, strict"));
    assert!(content.contains("fewer than 3 distinct points"));
}

#[test]
fn tune_requires_at_least_one_sweep_flag() {
    let f = pareto_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "tune",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "requires at least one of --sweep-policy-margin/--sweep-score-swing",
        ));
}

#[test]
fn tune_no_report_flag_writes_only_csv() {
    let f = pareto_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "tune",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--sweep-policy-margin",
            "0,150",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("report").not());
}

#[test]
fn tune_preset_out_writes_three_candidates_with_full_configs() {
    let f = pareto_fixture();
    let out = NamedTempFile::new().unwrap();
    let preset = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "tune",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--sweep-policy-margin",
            "0,150,350,450",
            "--out",
            out.path().to_str().unwrap(),
            "--preset-out",
            preset.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(preset.path()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed["preset_format_version"], 1);
    assert_eq!(parsed["tool"], "shogiesa tune");
    // Same 0/150/350 margins as the equivalent --report golden test above.
    assert_eq!(
        parsed["presets"]["broad"]["config"]["min_policy_margin_cp"],
        0
    );
    assert_eq!(
        parsed["presets"]["balanced"]["config"]["min_policy_margin_cp"],
        150
    );
    assert_eq!(
        parsed["presets"]["strict"]["config"]["min_policy_margin_cp"],
        350
    );
    assert!(parsed["presets"]["broad"]["coverage_fraction"].is_number());
    assert!(parsed["presets"]["broad"]["mismatch_rate"].is_number());
}

#[test]
fn tune_preset_out_collapses_like_report_does() {
    // Same single-frontier-point fixture as the --report collapse test: all 3 candidate keys
    // must point at the identical config, not fabricate 3 distinct entries.
    let f = make_labeled_jsonl(&[
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 500),
                tune_obs("7g7f", 100, 6, 6, 500),
            ]),
        ),
        position(
            "opening",
            serde_json::json!([
                tune_obs("7g7f", 100, 14, 14, 500),
                tune_obs("7g7f", 100, 6, 6, 500),
            ]),
        ),
    ]);
    let out = NamedTempFile::new().unwrap();
    let preset = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "tune",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "14",
            "--student-depths",
            "6",
            "--sweep-policy-margin",
            "0,100",
            "--out",
            out.path().to_str().unwrap(),
            "--preset-out",
            preset.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(preset.path()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed["presets"]["broad"], parsed["presets"]["balanced"]);
    assert_eq!(parsed["presets"]["balanced"], parsed["presets"]["strict"]);
}

#[test]
fn tune_preset_out_empty_when_no_audit_pairs() {
    // teacher-depth doesn't exist in the data -> no audit pairs anywhere -> empty frontier ->
    // an empty (not missing, not erroring) presets map, mirroring --report's own "no audit data"
    // informative-but-Ok path.
    let f = pareto_fixture();
    let out = NamedTempFile::new().unwrap();
    let preset = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "tune",
            "--input",
            f.path().to_str().unwrap(),
            "--teacher-depth",
            "99",
            "--student-depths",
            "6",
            "--sweep-policy-margin",
            "0,150",
            "--out",
            out.path().to_str().unwrap(),
            "--preset-out",
            preset.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(preset.path()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed["presets"].as_object().unwrap().len(), 0);
}

// --- stability ---

#[test]
fn stability_populates_swing_and_agreement() {
    let f = make_labeled_jsonl(&[position(
        "middlegame",
        serde_json::json!([obs("7g7f", 50, 4), obs("7g7f", 300, 6)]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "stability",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let v: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
    assert_eq!(v["stability"]["score_swing_cp"], 250);
    assert_eq!(v["stability"]["bestmove_agreement"], true);
}

#[test]
fn stability_detects_disagreement() {
    let f = make_labeled_jsonl(&[position(
        "middlegame",
        serde_json::json!([obs("7g7f", 50, 4), obs("2b3c", 50, 6)]),
    )]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "stability",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let v: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
    assert_eq!(v["stability"]["score_swing_cp"], 0);
    assert_eq!(v["stability"]["bestmove_agreement"], false);
}

#[test]
fn stability_skips_unlabeled() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "stability",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let v: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
    assert!(
        v["stability"].is_null(),
        "no observations → stability absent"
    );
}

// --- mine ---

fn game_pos(ply: u32, score_cp: i32) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "source": { "kind": "csa", "path": "game.csa", "ply": ply },
        "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
        "observations": [{ "engine": "e", "engine_version": null, "depth": 8,
            "score": { "kind": "cp", "value": score_cp },
            "bestmove": "7g7f", "nodes": null, "time_ms": null, "pv": null }]
    })
}

#[test]
fn mine_detects_blunder_and_window() {
    // ply1=+50, ply2=+300 → swing=250 > threshold 150
    // window=1 → ply1, ply2, ply3 all included
    let f = make_labeled_jsonl(&[game_pos(1, 50), game_pos(2, 300), game_pos(3, 310)]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "mine",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--blunder-threshold",
            "150",
            "--blunder-window",
            "1",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 3);
}

#[test]
fn mine_no_blunder_empty_output() {
    let f = make_labeled_jsonl(&[game_pos(1, 50), game_pos(2, 60)]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "mine",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--blunder-threshold",
            "150",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 0);
}

#[test]
fn mine_losing_threshold() {
    // ply2 eval=-600 for black → included with --losing-threshold=500
    let f = make_labeled_jsonl(&[game_pos(1, 100), game_pos(2, -600), game_pos(3, -580)]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "mine",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--blunder-threshold",
            "9999",
            "--losing-threshold",
            "500",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 2);
}

// --- balance ---

#[test]
fn balance_by_phase_defaults_to_min_bucket() {
    // 2 opening, 3 middlegame, 1 endgame → min=1 → 1 per phase = 3 total
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("opening", serde_json::json!([obs("7g7f", 60, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", 100, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", 110, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", 120, 4)])),
        position("endgame", serde_json::json!([obs("7g7f", 200, 4)])),
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "balance",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--by",
            "phase",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 3);
}

#[test]
fn balance_target_override() {
    // --target 2: opening→2, middlegame→2, endgame→1(capped) = 5 total
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("opening", serde_json::json!([obs("7g7f", 60, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", 100, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", 110, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", 120, 4)])),
        position("endgame", serde_json::json!([obs("7g7f", 200, 4)])),
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "balance",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--by",
            "phase",
            "--target",
            "2",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 5);
}

// --- PR8 golden baseline: balance's two-pass per-bucket bounded heap rewrite must produce
// byte-identical selection and order to the full-materialize-sort-truncate code it replaces.
// This fixture ties 4 records on the same sfen ("posA") within one bucket, well over any
// plausible target, so a real eviction contest happens and the earliest-index tiebreak (matching
// `sort_by`'s stability) is the only thing that decides the outcome. Captured against the
// pre-refactor binary; must still pass after.
fn pr8_heap_fixture() -> NamedTempFile {
    make_labeled_jsonl(&[
        position_with_path("posA", "b1.csa", serde_json::json!([])),
        position_with_path("posA", "b2.csa", serde_json::json!([])),
        position_with_path("posA", "b3.csa", serde_json::json!([])),
        position_with_path("posA", "b4.csa", serde_json::json!([])),
        position_with_path("posB", "b5.csa", serde_json::json!([])),
        position_with_path("posC", "b6.csa", serde_json::json!([])),
    ])
}

#[test]
fn balance_bounded_heap_resolves_tie_contest_by_earliest_index() {
    let f = pr8_heap_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "balance",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--by",
            "phase",
            "--target",
            "2",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(source_paths_in_order(&content), vec!["b1.csa", "b2.csa"]);
}

#[test]
fn balance_bounded_heap_auto_target_matches_pre_refactor_golden_output() {
    let f = make_labeled_jsonl(&[
        position_with_path_and_phase("posA", "b1.csa", "opening", serde_json::json!([])),
        position_with_path_and_phase("posA", "b2.csa", "opening", serde_json::json!([])),
        position_with_path_and_phase("posA", "b3.csa", "opening", serde_json::json!([])),
        position_with_path_and_phase("posA", "b4.csa", "opening", serde_json::json!([])),
        position_with_path_and_phase("posB", "b5.csa", "opening", serde_json::json!([])),
        position_with_path_and_phase("posM1", "b6.csa", "middlegame", serde_json::json!([])),
        position_with_path_and_phase("posM2", "b7.csa", "middlegame", serde_json::json!([])),
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "balance",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--by",
            "phase",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(
        source_paths_in_order(&content),
        vec!["b1.csa", "b2.csa", "b6.csa", "b7.csa"]
    );
}

#[test]
fn balance_by_eval_bucket_normalizes_to_black_perspective() {
    // Two Black-to-move records (raw cp -250, black-perspective also -250) and two
    // White-to-move records (raw cp +250, black-perspective -250 -- White winning by 250 means
    // Black losing by 250) all belong in the SAME 200cp eval bucket once normalized. With
    // --target 1, that single shared bucket should keep exactly 1 record; if bucketing used the
    // raw side-to-move-relative value instead, the White-to-move pair would form a second,
    // distinct bucket (around +200) and --target 1 would keep 2 records total.
    let f = make_labeled_jsonl(&[
        position("middlegame", serde_json::json!([obs("7g7f", -250, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", -260, 4)])),
        position_white_to_move("middlegame", serde_json::json!([obs("3c3d", 250, 4)])),
        position_white_to_move("middlegame", serde_json::json!([obs("3c3d", 260, 4)])),
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "balance",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--by",
            "eval-bucket",
            "--target",
            "1",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(
        content.lines().filter(|l| !l.trim().is_empty()).count(),
        1,
        "all 4 records should collapse into one black-perspective eval bucket"
    );
}

// --- stratify ---

/// Writes a hand-built quota-file fixture directly (not via `stratify --write-template`), so
/// tests can construct arbitrary or deliberately malformed quota content.
fn stratify_quota_file(by: &[&str], quotas: &[(&str, usize)]) -> NamedTempFile {
    let quotas: serde_json::Map<String, serde_json::Value> = quotas
        .iter()
        .map(|(k, v)| (k.to_string(), serde_json::json!(v)))
        .collect();
    let mut f = NamedTempFile::new().unwrap();
    write!(
        f,
        "{}",
        serde_json::json!({
            "stratify_format_version": 1,
            "input": "unused.jsonl",
            "by": by,
            "quotas": quotas
        })
    )
    .unwrap();
    f.flush().unwrap();
    f
}

#[test]
fn stratify_write_template_writes_observed_bucket_counts() {
    let input = make_labeled_jsonl(&[
        position("opening", serde_json::json!([])),
        position("opening", serde_json::json!([])),
        position("middlegame", serde_json::json!([])),
    ]);
    let template = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--write-template",
            template.path().to_str().unwrap(),
            "--by",
            "phase",
        ])
        .assert()
        .success();

    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(template.path()).unwrap()).unwrap();
    assert_eq!(v["stratify_format_version"], 1);
    assert_eq!(v["by"], serde_json::json!(["phase"]));
    // The trailing colon from `bucket_key`'s own string output must survive verbatim -- trimming
    // it here would silently defeat the whole point of reusing that string directly.
    assert_eq!(v["quotas"]["opening:"], 2);
    assert_eq!(v["quotas"]["middlegame:"], 1);
}

#[test]
fn stratify_write_template_then_quota_unedited_keeps_everything() {
    // The write-template and --quota code paths independently derive bucket keys via
    // `bucket_key`, from the same `--by`/`by` dims -- if they ever drifted, an *unedited* template
    // (quotas == observed counts) would start dropping records under `bucket_not_in_quota`.
    // Piping one into the other, unedited, is the one test that actually exercises the seam.
    let input = make_labeled_jsonl(&[
        position("opening", serde_json::json!([])),
        position("opening", serde_json::json!([])),
        position("middlegame", serde_json::json!([])),
    ]);
    let template = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--write-template",
            template.path().to_str().unwrap(),
            "--by",
            "phase",
        ])
        .assert()
        .success();

    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--quota",
            template.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 3);
}

#[test]
fn stratify_write_template_requires_by() {
    let input = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let template = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--write-template",
            template.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires --by"));
}

#[test]
fn stratify_quota_keeps_up_to_each_bucket_quota() {
    let mut records = vec![];
    for _ in 0..4 {
        records.push(position("opening", serde_json::json!([])));
    }
    for _ in 0..3 {
        records.push(position("middlegame", serde_json::json!([])));
    }
    let input = make_labeled_jsonl(&records);
    let quota = stratify_quota_file(&["phase"], &[("opening:", 2), ("middlegame:", 1)]);
    let out = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--quota",
            quota.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 3);
}

#[test]
fn stratify_drops_bucket_not_in_quota_distinctly_from_over_quota() {
    let input = make_labeled_jsonl(&[
        position("opening", serde_json::json!([])),
        position("opening", serde_json::json!([])),
        position("endgame", serde_json::json!([])),
        position("endgame", serde_json::json!([])),
    ]);
    let quota = stratify_quota_file(&["phase"], &[("opening:", 1)]);
    let out = NamedTempFile::new().unwrap();
    let manifest = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--quota",
            quota.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("bucket_not_in_quota      2"))
        .stderr(predicate::str::contains("over_quota               1"));

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    assert_eq!(manifest["records_kept"], 1);
    assert_eq!(manifest["records_dropped"], 3);
    assert_eq!(manifest["drop_reasons"]["bucket_not_in_quota"], 2);
    assert_eq!(manifest["drop_reasons"]["over_quota"], 1);
}

#[test]
fn stratify_preserves_original_file_order() {
    // 3 distinct roots, each contributing exactly 1 candidate (so every one is rank 0 -- tied,
    // broken by root hash), quota 2. Regardless of *which* 2 survive the hash tie-break, whichever
    // survive must come out in their original file order, not heap-internal order.
    let input = make_labeled_jsonl(&[
        position_with_path_and_phase("sfen-a", "root_a.csa", "opening", serde_json::json!([])),
        position_with_path_and_phase("sfen-b", "root_b.csa", "opening", serde_json::json!([])),
        position_with_path_and_phase("sfen-c", "root_c.csa", "opening", serde_json::json!([])),
    ]);
    let quota = stratify_quota_file(&["phase"], &[("opening:", 2)]);
    let out = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--quota",
            quota.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let original_index = |path: &str| match path {
        "root_a.csa" => 0,
        "root_b.csa" => 1,
        "root_c.csa" => 2,
        other => panic!("unexpected path {other}"),
    };
    let indices: Vec<i32> = source_paths_in_order(&content)
        .iter()
        .map(|p| original_index(p))
        .collect();
    assert_eq!(indices.len(), 2);
    assert!(
        indices.is_sorted(),
        "kept records must come out in original file order, got indices {indices:?}"
    );
}

#[test]
fn stratify_group_aware_quota_fill_diversifies_across_roots() {
    // gameA contributes 5 candidates (sfens sorting lexicographically before gameB's), gameB
    // contributes 1. Quota 2. Rank-0 tier is exactly {gameA's first record, gameB's only record}
    // -- 2 entries, exactly filling the quota -- so both are *always* kept regardless of seed.
    // If group-awareness regressed to a plain-sfen key (like `balance`'s), the 2 lexicographically
    // smallest sfens are both gameA's, giving zero representation for gameB.
    for seed in ["0", "12345"] {
        let mut records = vec![];
        for i in 1..=5 {
            records.push(position_with_path_and_phase(
                &format!("a{i}"),
                "gameA.csa",
                "opening",
                serde_json::json!([]),
            ));
        }
        records.push(position_with_path_and_phase(
            "b1",
            "gameB.csa",
            "opening",
            serde_json::json!([]),
        ));
        let input = make_labeled_jsonl(&records);
        let quota = stratify_quota_file(&["phase"], &[("opening:", 2)]);
        let out = NamedTempFile::new().unwrap();

        shogiesa()
            .args([
                "stratify",
                "--input",
                input.path().to_str().unwrap(),
                "--quota",
                quota.path().to_str().unwrap(),
                "--out",
                out.path().to_str().unwrap(),
                "--seed",
                seed,
            ])
            .assert()
            .success();

        let content = std::fs::read_to_string(out.path()).unwrap();
        assert_eq!(
            source_paths_in_order(&content),
            vec!["gameA.csa".to_string(), "gameB.csa".to_string()],
            "seed {seed}: both roots must be represented, in original file order"
        );
    }
}

#[test]
fn stratify_single_root_bucket_keeps_first_n_in_file_order() {
    // Single root: sfens are in descending lexicographic order as encountered in the file, so
    // "keep the 2 lexicographically smallest" (posW, posX) and "keep the first 2 in file order"
    // (posZ, posY) would disagree -- this locks in the documented single-root behavior (file
    // order wins, since there's no root diversity to protect).
    let input = make_labeled_jsonl(&[
        position_with_path_and_phase("posZ", "game.csa", "opening", serde_json::json!([])),
        position_with_path_and_phase("posY", "game.csa", "opening", serde_json::json!([])),
        position_with_path_and_phase("posX", "game.csa", "opening", serde_json::json!([])),
        position_with_path_and_phase("posW", "game.csa", "opening", serde_json::json!([])),
    ]);
    let quota = stratify_quota_file(&["phase"], &[("opening:", 2)]);
    let out = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--quota",
            quota.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let sfens: Vec<String> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["sfen"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(sfens, vec!["posZ".to_string(), "posY".to_string()]);
}

#[test]
fn stratify_manifest_reports_root_diversity_stats() {
    let input = make_labeled_jsonl(&[
        position_with_path_and_phase("s1", "g1.csa", "opening", serde_json::json!([])),
        position_with_path_and_phase("s2", "g2.csa", "opening", serde_json::json!([])),
        position_with_path_and_phase("s3", "g3.csa", "middlegame", serde_json::json!([])),
    ]);
    let quota = stratify_quota_file(&["phase"], &[("opening:", 2), ("middlegame:", 1)]);
    let out = NamedTempFile::new().unwrap();
    let manifest = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--quota",
            quota.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    // A naive "max share across all buckets" would read 1.0 (the middlegame singleton bucket) --
    // scoping to buckets with >=2 kept records must exclude it, leaving the opening bucket's 0.5.
    assert_eq!(manifest["max_root_share_in_any_bucket"], 0.5);
    assert_eq!(manifest["distinct_roots_kept"], 3);
}

#[test]
fn stratify_requires_exactly_one_of_write_template_or_quota() {
    let input = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    shogiesa()
        .args(["stratify", "--input", input.path().to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "requires exactly one of --write-template or --quota",
        ));
}

#[test]
fn stratify_write_template_and_quota_conflict() {
    let input = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let template = NamedTempFile::new().unwrap();
    let quota = stratify_quota_file(&["phase"], &[("opening:", 1)]);
    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--write-template",
            template.path().to_str().unwrap(),
            "--quota",
            quota.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn stratify_quota_rejects_by_flag() {
    let input = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let quota = stratify_quota_file(&["phase"], &[("opening:", 1)]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--quota",
            quota.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--by",
            "phase",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn stratify_quota_loader_error_paths() {
    let input = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = NamedTempFile::new().unwrap();

    let missing = std::env::temp_dir().join("shogiesa_test_quota_does_not_exist.json");
    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--quota",
            missing.to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot read quota file"));

    let mut malformed = NamedTempFile::new().unwrap();
    write!(malformed, "not json").unwrap();
    malformed.flush().unwrap();
    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--quota",
            malformed.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot parse quota file"));

    let empty_quotas = stratify_quota_file(&["phase"], &[]);
    shogiesa()
        .args([
            "stratify",
            "--input",
            input.path().to_str().unwrap(),
            "--quota",
            empty_quotas.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to keep"));
}

// --- split ---

#[test]
fn split_by_source_creates_one_file_per_game() {
    use std::io::Write;
    // Two records from different sources
    let rec_a = serde_json::json!({
        "schema_version": 1, "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "source": { "kind": "csa", "path": "game_a.csa", "ply": 1 },
        "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
        "observations": []
    });
    let rec_b = serde_json::json!({
        "schema_version": 1, "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "source": { "kind": "csa", "path": "game_b.csa", "ply": 1 },
        "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
        "observations": []
    });
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "{rec_a}").unwrap();
    writeln!(f, "{rec_b}").unwrap();
    f.flush().unwrap();

    let out_dir = tempfile::tempdir().unwrap();
    shogiesa()
        .args([
            "split",
            "--input",
            f.path().to_str().unwrap(),
            "--by-source",
            "--out-dir",
            out_dir.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let files: Vec<_> = std::fs::read_dir(out_dir.path()).unwrap().collect();
    assert_eq!(
        files.len(),
        3,
        "one file per source game, plus manifest.json"
    );

    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out_dir.path().join("manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["total_positions"], 2);
    assert_eq!(manifest["files"]["game_a.csa.jsonl"], 1);
    assert_eq!(manifest["files"]["game_b.csa.jsonl"], 1);
}

fn source_record(path: &str, ply: u32) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "source": { "kind": "csa", "path": path, "ply": ply },
        "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
        "observations": []
    })
}

#[test]
fn split_by_source_survives_forced_eviction_with_more_sources_than_max_open_writers() {
    use std::io::Write;
    // 5 sources, interleaved (a1,b1,c1,d1,e1,a2,b2,c2,d2,e2), --max-open-writers 2 -- by the
    // time each source's 2nd record arrives, at least 2 other sources have opened in between,
    // forcing every source's writer to be evicted and later reopened in append mode.
    let sources = [
        "game_a.csa",
        "game_b.csa",
        "game_c.csa",
        "game_d.csa",
        "game_e.csa",
    ];
    let mut f = NamedTempFile::new().unwrap();
    for ply in [1u32, 2] {
        for src in sources {
            writeln!(f, "{}", source_record(src, ply)).unwrap();
        }
    }
    f.flush().unwrap();

    let out_dir = tempfile::tempdir().unwrap();
    shogiesa()
        .args([
            "split",
            "--input",
            f.path().to_str().unwrap(),
            "--by-source",
            "--out-dir",
            out_dir.path().to_str().unwrap(),
            "--max-open-writers",
            "2",
        ])
        .assert()
        .success();

    for src in sources {
        let content = std::fs::read_to_string(out_dir.path().join(format!("{src}.jsonl"))).unwrap();
        let plies: Vec<serde_json::Value> = content
            .lines()
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["source"]["ply"].clone())
            .collect();
        assert_eq!(
            plies,
            vec![serde_json::json!(1), serde_json::json!(2)],
            "source {src} lost or reordered a record across an eviction+reopen cycle"
        );
    }

    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out_dir.path().join("manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["total_positions"], 10);
}

#[test]
fn split_by_source_truncates_stale_file_on_first_touch_this_run() {
    use std::io::Write;
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "{}", source_record("game_a.csa", 1)).unwrap();
    f.flush().unwrap();

    let out_dir = tempfile::tempdir().unwrap();
    // Simulate a stale file left over from an unrelated previous run in the same --out-dir
    std::fs::write(
        out_dir.path().join("game_a.csa.jsonl"),
        "STALE LEFTOVER LINE\n",
    )
    .unwrap();

    shogiesa()
        .args([
            "split",
            "--input",
            f.path().to_str().unwrap(),
            "--by-source",
            "--out-dir",
            out_dir.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out_dir.path().join("game_a.csa.jsonl")).unwrap();
    assert!(
        !content.contains("STALE LEFTOVER LINE"),
        "first touch this run must truncate, not append to, a pre-existing file"
    );
    assert_eq!(content.lines().count(), 1);
}

#[test]
fn split_train_valid_test_no_leakage() {
    use std::io::Write;
    let mut f = NamedTempFile::new().unwrap();
    for src in ["game_a.csa", "game_b.csa", "game_c.csa"] {
        for ply in [1u32, 2] {
            writeln!(f, "{}", source_record(src, ply)).unwrap();
        }
    }
    f.flush().unwrap();

    let dir = tempfile::tempdir().unwrap();
    let train = dir.path().join("train.jsonl");
    let valid = dir.path().join("valid.jsonl");
    let test = dir.path().join("test.jsonl");
    shogiesa()
        .args([
            "split",
            "--input",
            f.path().to_str().unwrap(),
            "--train",
            train.to_str().unwrap(),
            "--valid",
            valid.to_str().unwrap(),
            "--test",
            test.to_str().unwrap(),
            "--valid-frac",
            "0.34",
            "--test-frac",
            "0.34",
            "--seed",
            "7",
        ])
        .assert()
        .success();

    let mut source_to_files: HashMap<String, HashSet<&str>> = HashMap::new();
    let mut total_lines = 0usize;
    for (name, path) in [("train", &train), ("valid", &valid), ("test", &test)] {
        let content = std::fs::read_to_string(path).unwrap();
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            let src = v["source"]["path"].as_str().unwrap().to_string();
            source_to_files.entry(src).or_default().insert(name);
            total_lines += 1;
        }
    }
    assert_eq!(total_lines, 6, "no positions dropped");
    assert_eq!(
        source_to_files.len(),
        3,
        "all 3 sources should appear somewhere"
    );
    for (src, files) in &source_to_files {
        assert_eq!(
            files.len(),
            1,
            "source {src} leaked across splits: {files:?}"
        );
    }

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["total_positions"], 6);
    let split_total: u64 = ["train", "valid", "test"]
        .iter()
        .map(|s| manifest["splits"][s]["positions"].as_u64().unwrap())
        .sum();
    assert_eq!(split_total, 6);
}

#[test]
fn split_train_valid_test_keeps_variation_with_mainline() {
    use std::io::Write;
    let mut f = NamedTempFile::new().unwrap();
    // Each "game" has a mainline position and a KIF-variation-suffixed sibling, mirroring
    // shogiesa-kif's `source.path` convention for 変化 branches (`path#varN@ply`).
    for src in ["game_a.kif", "game_b.kif", "game_c.kif", "game_d.kif"] {
        writeln!(f, "{}", source_record(src, 1)).unwrap();
        writeln!(f, "{}", source_record(&format!("{src}#var1@2"), 2)).unwrap();
    }
    f.flush().unwrap();

    let dir = tempfile::tempdir().unwrap();
    let train = dir.path().join("train.jsonl");
    let valid = dir.path().join("valid.jsonl");
    let test = dir.path().join("test.jsonl");
    shogiesa()
        .args([
            "split",
            "--input",
            f.path().to_str().unwrap(),
            "--train",
            train.to_str().unwrap(),
            "--valid",
            valid.to_str().unwrap(),
            "--test",
            test.to_str().unwrap(),
            "--valid-frac",
            "0.34",
            "--test-frac",
            "0.34",
            "--seed",
            "7",
        ])
        .assert()
        .success();

    let mut path_to_file: HashMap<String, &str> = HashMap::new();
    for (name, path) in [("train", &train), ("valid", &valid), ("test", &test)] {
        let content = std::fs::read_to_string(path).unwrap();
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            let src = v["source"]["path"].as_str().unwrap().to_string();
            path_to_file.insert(src, name);
        }
    }
    for src in ["game_a.kif", "game_b.kif", "game_c.kif", "game_d.kif"] {
        let mainline_file = path_to_file[src];
        let variation_file = path_to_file[&format!("{src}#var1@2")];
        assert_eq!(
            mainline_file, variation_file,
            "{src}'s mainline and variation landed in different splits"
        );
    }
}

#[test]
fn split_train_valid_test_root_id_overrides_unrelated_paths() {
    use std::io::Write;

    fn source_record_with_root(path: &str, ply: u32, root_id: &str) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 7,
            "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "source": { "kind": "kif", "path": path, "ply": ply, "root_id": root_id },
            "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
            "observations": []
        })
    }

    let mut f = NamedTempFile::new().unwrap();
    // Each pair's two `path` values share nothing in common (so path-suffix grouping would treat
    // them as unrelated games), but share a root_id -- proving root_id, not path, decides the
    // bucket when both are present.
    for i in 0..4 {
        writeln!(
            f,
            "{}",
            source_record_with_root(
                &format!("totally_unrelated_path_{i}a"),
                1,
                &format!("root_{i}")
            )
        )
        .unwrap();
        writeln!(
            f,
            "{}",
            source_record_with_root(
                &format!("totally_unrelated_path_{i}b"),
                2,
                &format!("root_{i}")
            )
        )
        .unwrap();
    }
    f.flush().unwrap();

    let dir = tempfile::tempdir().unwrap();
    let train = dir.path().join("train.jsonl");
    let valid = dir.path().join("valid.jsonl");
    let test = dir.path().join("test.jsonl");
    shogiesa()
        .args([
            "split",
            "--input",
            f.path().to_str().unwrap(),
            "--train",
            train.to_str().unwrap(),
            "--valid",
            valid.to_str().unwrap(),
            "--test",
            test.to_str().unwrap(),
            "--valid-frac",
            "0.34",
            "--test-frac",
            "0.34",
            "--seed",
            "7",
        ])
        .assert()
        .success();

    let mut path_to_file: HashMap<String, &str> = HashMap::new();
    for (name, path) in [("train", &train), ("valid", &valid), ("test", &test)] {
        let content = std::fs::read_to_string(path).unwrap();
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            let src = v["source"]["path"].as_str().unwrap().to_string();
            path_to_file.insert(src, name);
        }
    }
    for i in 0..4 {
        let a_file = path_to_file[&format!("totally_unrelated_path_{i}a")];
        let b_file = path_to_file[&format!("totally_unrelated_path_{i}b")];
        assert_eq!(
            a_file, b_file,
            "root_{i}'s two unrelated-path records landed in different splits"
        );
    }
}

#[test]
fn split_train_valid_test_deterministic_with_seed() {
    use std::io::Write;
    let mut f = NamedTempFile::new().unwrap();
    for src in ["game_a.csa", "game_b.csa", "game_c.csa", "game_d.csa"] {
        writeln!(f, "{}", source_record(src, 1)).unwrap();
    }
    f.flush().unwrap();

    let run = || {
        let dir = tempfile::tempdir().unwrap();
        let train = dir.path().join("train.jsonl");
        let valid = dir.path().join("valid.jsonl");
        let test = dir.path().join("test.jsonl");
        shogiesa()
            .args([
                "split",
                "--input",
                f.path().to_str().unwrap(),
                "--train",
                train.to_str().unwrap(),
                "--valid",
                valid.to_str().unwrap(),
                "--test",
                test.to_str().unwrap(),
                "--valid-frac",
                "0.25",
                "--test-frac",
                "0.25",
                "--seed",
                "42",
            ])
            .assert()
            .success();
        (
            std::fs::read_to_string(&train).unwrap(),
            std::fs::read_to_string(&valid).unwrap(),
            std::fs::read_to_string(&test).unwrap(),
        )
    };
    assert_eq!(run(), run());
}

#[test]
fn split_train_valid_test_requires_all_three_paths() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let dir = tempfile::tempdir().unwrap();
    shogiesa()
        .args([
            "split",
            "--input",
            f.path().to_str().unwrap(),
            "--train",
            dir.path().join("train.jsonl").to_str().unwrap(),
        ])
        .assert()
        .failure();
}

// --- sample ---

#[test]
fn sample_returns_exact_count() {
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", 100, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", 110, 4)])),
        position("endgame", serde_json::json!([obs("7g7f", 200, 4)])),
        position("endgame", serde_json::json!([obs("7g7f", 210, 4)])),
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "sample",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "3",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 3);
}

#[test]
fn sample_deterministic_with_seed() {
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", 100, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", 110, 4)])),
        position("endgame", serde_json::json!([obs("7g7f", 200, 4)])),
    ]);
    let out1 = NamedTempFile::new().unwrap();
    let out2 = NamedTempFile::new().unwrap();
    for out in [&out1, &out2] {
        shogiesa()
            .args([
                "sample",
                "--input",
                f.path().to_str().unwrap(),
                "--out",
                out.path().to_str().unwrap(),
                "--count",
                "2",
                "--seed",
                "42",
            ])
            .assert()
            .success();
    }
    assert_eq!(
        std::fs::read_to_string(out1.path()).unwrap(),
        std::fs::read_to_string(out2.path()).unwrap(),
        "same seed → same output"
    );
}

// --- dedup-zobrist ---

#[test]
fn extract_dedup_zobrist_removes_duplicates() {
    // Extract from the same file twice (concat of two identical paths) would double-count,
    // but --dedup-zobrist removes them. We use --dedup-zobrist on a single file as a smoke test.
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "extract",
            "--input",
            fixture("sample.csa").to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--dedup-zobrist",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    // sample.csa has 5 distinct positions; all should be kept
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 5);
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
            fake_usi_engine_bin().to_str().unwrap(),
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

#[test]
fn filter_manifest_records_drop_reasons_and_config() {
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("opening", serde_json::json!([obs_mate("7g7f", 3, 4)])),
    ]);
    let out = NamedTempFile::new().unwrap();
    let manifest_path = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--exclude-mate",
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert!(!manifest["git_sha"].as_str().unwrap().is_empty());
    assert_eq!(manifest["schema_version"], shogiesa_core::SCHEMA_VERSION);
    assert_eq!(manifest["command"], "filter");
    assert_eq!(manifest["records_read"], 2);
    assert_eq!(manifest["records_kept"], 1);
    assert_eq!(manifest["records_dropped"], 1);
    assert_eq!(manifest["drop_reasons"]["mate"], 1);
    assert_eq!(manifest["filter_config"]["exclude_mate"], true);
}

#[test]
fn filter_dry_run_reports_without_writing_output() {
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("opening", serde_json::json!([obs_mate("7g7f", 3, 4)])),
    ]);
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--exclude-mate",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "done (dry run): 2 read, 1 passed, 1 filtered",
        ))
        .stderr(predicate::str::contains("mate"));
}

#[test]
fn filter_requires_out_unless_dry_run() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([obs("7g7f", 50, 4)]))]);
    shogiesa()
        .args(["filter", "--input", f.path().to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn filter_dry_run_with_manifest_writes_manifest_no_output_file() {
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("opening", serde_json::json!([obs_mate("7g7f", 3, 4)])),
    ]);
    let manifest_path = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--exclude-mate",
            "--dry-run",
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["records_read"], 2);
    assert_eq!(manifest["records_kept"], 1);
    assert_eq!(manifest["records_dropped"], 1);
}

#[test]
fn balance_manifest_records_counts() {
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("opening", serde_json::json!([obs("7g7f", 60, 4)])),
        position("middlegame", serde_json::json!([obs("7g7f", 100, 4)])),
    ]);
    let out = NamedTempFile::new().unwrap();
    let manifest_path = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "balance",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--by",
            "phase",
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["command"], "balance");
    assert_eq!(manifest["records_read"], 3);
    assert_eq!(manifest["records_kept"], 2); // min bucket size (1) per phase * 2 phases
    assert_eq!(manifest["records_dropped"], 1);
    assert_eq!(manifest["labeled_records"], 2);
}

// --- select ---

#[test]
fn select_uncertain_ranks_worst_quality_first() {
    fn obs_full(
        bound: &str,
        requested_depth: u32,
        depth: u32,
        margin: Option<i32>,
    ) -> serde_json::Value {
        serde_json::json!({
            "engine": "e", "engine_version": null, "depth": depth, "requested_depth": requested_depth,
            "score": { "kind": "cp", "value": 50 }, "score_bound": bound, "bestmove": "7g7f",
            "nodes": null, "time_ms": null, "pv": null, "policy_margin_cp": margin
        })
    }
    // clean: exact score, margin present, requested depth met -> every gate passes
    let clean = position(
        "opening",
        serde_json::json!([obs_full("exact", 8, 8, Some(50))]),
    );
    // partial: missing policy margin only -> 1 of 4 gates fails
    let partial = position(
        "opening",
        serde_json::json!([obs_full("exact", 8, 8, None)]),
    );
    // worst: non-exact score, missing margin, and depth underreach -> 3 of 4 gates fail
    let worst = position(
        "opening",
        serde_json::json!([obs_full("lowerbound", 12, 8, None)]),
    );

    let f = make_labeled_jsonl(&[clean, partial.clone(), worst.clone()]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "select",
            "--input",
            f.path().to_str().unwrap(),
            "--strategy",
            "uncertain",
            "--count",
            "2",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let lines: Vec<serde_json::Value> = content
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["observations"][0]["score_bound"], "lowerbound");
    assert_eq!(
        lines[1]["observations"][0]["policy_margin_cp"],
        serde_json::Value::Null
    );
}

#[test]
fn select_hard_prioritizes_blunder_adjacent_position() {
    let f = make_labeled_jsonl(&[
        game_pos(1, 0),
        game_pos(2, 300), // swing 300 from ply1 >= default threshold 200
        game_pos(3, 305), // swing 5 from ply2, not a blunder
    ]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "select",
            "--input",
            f.path().to_str().unwrap(),
            "--strategy",
            "hard",
            "--count",
            "1",
            "--blunder-window",
            "0", // only the blunder ply itself, not its neighbors -- keeps this test unambiguous
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let lines: Vec<serde_json::Value> = content
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["source"]["ply"], 2);
}

#[test]
fn select_hard_breaks_ties_by_per_record_score_swing() {
    fn multi_obs_position(ply: u32, scores: &[i32]) -> serde_json::Value {
        let observations: Vec<serde_json::Value> = scores
            .iter()
            .enumerate()
            .map(|(depth, &cp)| obs("7g7f", cp, depth as u32 + 1))
            .collect();
        let mut rec = game_pos(ply, scores[0]);
        rec["observations"] = serde_json::json!(observations);
        rec
    }
    // --blunder-threshold 9999 disables blunder detection so only per-record swing matters.
    let low_swing = multi_obs_position(1, &[100, 110]);
    let high_swing = multi_obs_position(2, &[120, 600]);

    let f = make_labeled_jsonl(&[low_swing, high_swing]);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "select",
            "--input",
            f.path().to_str().unwrap(),
            "--strategy",
            "hard",
            "--count",
            "1",
            "--blunder-threshold",
            "9999",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let lines: Vec<serde_json::Value> = content
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(
        lines[0]["source"]["ply"], 2,
        "bigger per-record swing ranks first"
    );
}

#[test]
fn select_coverage_prioritizes_thin_bucket() {
    let mut records: Vec<serde_json::Value> = (0..5)
        .map(|_| position("opening", serde_json::json!([])))
        .collect();
    records.push(position("endgame", serde_json::json!([])));

    let f = make_labeled_jsonl(&records);
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "select",
            "--input",
            f.path().to_str().unwrap(),
            "--strategy",
            "coverage",
            "--count",
            "1",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let lines: Vec<serde_json::Value> = content
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["tags"]["phase"], "endgame");
}

#[test]
fn sample_manifest_records_counts() {
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("opening", serde_json::json!([obs("7g7f", 60, 4)])),
        position("opening", serde_json::json!([obs("7g7f", 70, 4)])),
    ]);
    let out = NamedTempFile::new().unwrap();
    let manifest_path = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "sample",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "2",
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["command"], "sample");
    assert_eq!(manifest["records_read"], 3);
    assert_eq!(manifest["records_kept"], 2);
    assert_eq!(manifest["records_dropped"], 1);
}

// --- PR7 golden baseline: sample/select's bounded top-K heap streaming rewrite must produce
// byte-identical selection and order to the full-materialize-sort-truncate code it replaces. This
// fixture ties every record on the primary ranking key (all identical observations, so
// `uncertain`'s evaluate_quality score and `coverage`'s bucket count are the same for all 6) with
// one duplicated sfen ("posA" on g1 and g4) -- so ranking is decided entirely by the
// seeded_hash/original-index tie-break chain, not the primary key, forcing every layer of that
// chain to actually run. Captured against the pre-refactor binary; must still pass after.
fn pr7_heap_fixture() -> NamedTempFile {
    make_labeled_jsonl(&[
        position_with_path("posA", "g1.csa", serde_json::json!([obs("7g7f", 50, 4)])),
        position_with_path("posB", "g2.csa", serde_json::json!([obs("7g7f", 50, 4)])),
        position_with_path("posC", "g3.csa", serde_json::json!([obs("7g7f", 50, 4)])),
        position_with_path("posA", "g4.csa", serde_json::json!([obs("7g7f", 50, 4)])),
        position_with_path("posD", "g5.csa", serde_json::json!([obs("7g7f", 50, 4)])),
        position_with_path("posE", "g6.csa", serde_json::json!([obs("7g7f", 50, 4)])),
    ])
}

fn source_paths_in_order(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["source"]["path"].as_str().unwrap().to_string()
        })
        .collect()
}

#[test]
fn sample_bounded_heap_matches_pre_refactor_golden_output() {
    let f = pr7_heap_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "sample",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "4",
            "--seed",
            "7",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    // restored to input order; both duplicate-sfen records (g1, g4) survive the hash tie
    assert_eq!(
        source_paths_in_order(&content),
        vec!["g1.csa", "g3.csa", "g4.csa", "g6.csa"]
    );
}

#[test]
fn select_uncertain_bounded_heap_matches_pre_refactor_golden_output() {
    let f = pr7_heap_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "select",
            "--input",
            f.path().to_str().unwrap(),
            "--strategy",
            "uncertain",
            "--count",
            "4",
            "--seed",
            "7",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    // ranked order, not file order; g1 (earlier index) sorts before its hash-tied twin g4
    assert_eq!(
        source_paths_in_order(&content),
        vec!["g6.csa", "g1.csa", "g4.csa", "g3.csa"]
    );
}

#[test]
fn select_coverage_bounded_heap_matches_pre_refactor_golden_output() {
    let f = pr7_heap_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "select",
            "--input",
            f.path().to_str().unwrap(),
            "--strategy",
            "coverage",
            "--count",
            "4",
            "--seed",
            "7",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(
        source_paths_in_order(&content),
        vec!["g6.csa", "g1.csa", "g4.csa", "g3.csa"]
    );
}

#[test]
fn sample_count_larger_than_dataset_keeps_everything() {
    let f = pr7_heap_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "sample",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "100",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 6);
}

#[test]
fn sample_count_zero_selects_nothing() {
    let f = pr7_heap_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "sample",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "0",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 0);
}

#[test]
fn select_uncertain_count_larger_than_dataset_keeps_everything() {
    let f = pr7_heap_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "select",
            "--input",
            f.path().to_str().unwrap(),
            "--strategy",
            "uncertain",
            "--count",
            "100",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(content.lines().filter(|l| !l.trim().is_empty()).count(), 6);
}

/// Stronger tie-contest fixture than `pr7_heap_fixture`: 4 records share one sfen ("posA") so
/// their score/bucket-count AND seeded_hash are all identical. With `--count 2` and one unrelated
/// survivor (posE), only one of the four tied "posA" records can fit -- forcing a genuine eviction
/// contest among them (unlike `pr7_heap_fixture`'s single duplicate pair, which happened to both
/// fit within capacity without ever competing for a slot). Whichever one wins must be resolved by
/// the final index tiebreak, exactly reproducing `sort_by`'s stability.
fn pr7_tie_contest_fixture() -> NamedTempFile {
    make_labeled_jsonl(&[
        position_with_path("posA", "t1.csa", serde_json::json!([obs("7g7f", 50, 4)])),
        position_with_path("posA", "t2.csa", serde_json::json!([obs("7g7f", 50, 4)])),
        position_with_path("posA", "t3.csa", serde_json::json!([obs("7g7f", 50, 4)])),
        position_with_path("posA", "t4.csa", serde_json::json!([obs("7g7f", 50, 4)])),
        position_with_path("posD", "t5.csa", serde_json::json!([obs("7g7f", 50, 4)])),
        position_with_path("posE", "t6.csa", serde_json::json!([obs("7g7f", 50, 4)])),
    ])
}

#[test]
fn sample_bounded_heap_resolves_full_tie_contest_by_earliest_index() {
    let f = pr7_tie_contest_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "sample",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "2",
            "--seed",
            "7",
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(source_paths_in_order(&content), vec!["t1.csa", "t6.csa"]);
}

#[test]
fn select_uncertain_bounded_heap_resolves_full_tie_contest_by_earliest_index() {
    let f = pr7_tie_contest_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "select",
            "--input",
            f.path().to_str().unwrap(),
            "--strategy",
            "uncertain",
            "--count",
            "2",
            "--seed",
            "7",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(source_paths_in_order(&content), vec!["t6.csa", "t1.csa"]);
}

#[test]
fn select_coverage_bounded_heap_resolves_full_tie_contest_by_earliest_index() {
    let f = pr7_tie_contest_fixture();
    let out = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "select",
            "--input",
            f.path().to_str().unwrap(),
            "--strategy",
            "coverage",
            "--count",
            "2",
            "--seed",
            "7",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(source_paths_in_order(&content), vec!["t6.csa", "t1.csa"]);
}

#[test]
fn pack_manifest_records_counts() {
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("opening", serde_json::json!([obs("7g7f", 60, 4)])),
    ]);
    let out = NamedTempFile::new().unwrap();
    let manifest_path = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "pack",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["command"], "pack");
    assert_eq!(
        manifest["pack_format_version"],
        shogiesa_pack::FORMAT_VERSION
    );
    assert_eq!(manifest["records_read"], 2);
    assert_eq!(manifest["records_kept"], 2);
    assert_eq!(manifest["records_dropped"], 0);
}

#[test]
fn same_input_file_produces_same_manifest_input_hash_across_commands() {
    // filter/pack hash incrementally while streaming; sample/balance hash via a separate
    // whole-file pass; label accumulates its hash inside its own reader thread instead of
    // calling that separate pass (to avoid re-reading the whole input a second time) -- all
    // three must agree, or comparing manifests across commands for the same file would show a
    // spurious "changed input" for no reason.
    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("opening", serde_json::json!([obs("7g7f", 60, 4)])),
    ]);
    let filter_out = NamedTempFile::new().unwrap();
    let filter_manifest = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            filter_out.path().to_str().unwrap(),
            "--manifest",
            filter_manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let sample_out = NamedTempFile::new().unwrap();
    let sample_manifest = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "sample",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            sample_out.path().to_str().unwrap(),
            "--count",
            "2",
            "--manifest",
            sample_manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let label_out = NamedTempFile::new().unwrap();
    let label_manifest = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--out",
            label_out.path().to_str().unwrap(),
            "--manifest",
            label_manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let filter_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(filter_manifest.path()).unwrap()).unwrap();
    let sample_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(sample_manifest.path()).unwrap()).unwrap();
    let label_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(label_manifest.path()).unwrap()).unwrap();
    assert_eq!(filter_json["input_hash"], sample_json["input_hash"]);
    assert_eq!(filter_json["input_hash"], label_json["input_hash"]);
}

#[test]
fn manifest_common_keys_present_across_commands() {
    // Presence, not an exact/closed key set -- filter/label legitimately carry extra fields
    // (filter_config, engine_name, ...), and score_bound_distribution/drop_reasons are omitted
    // entirely when empty (no MultiPV candidates / no drops), so neither belongs here.
    let common_keys = [
        "shogiesa_version",
        "git_sha",
        "schema_version",
        "pack_format_version",
        "command",
        "args",
        "input_path",
        "input_hash",
        "records_read",
        "records_kept",
        "records_dropped",
        "observations_total",
        "observations_with_candidates",
    ];

    let f = make_labeled_jsonl(&[
        position("opening", serde_json::json!([obs("7g7f", 50, 4)])),
        position("middlegame", serde_json::json!([obs("2g2f", 30, 4)])),
    ]);

    let mut manifests = Vec::new();

    let label_out = NamedTempFile::new().unwrap();
    let label_manifest = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--out",
            label_out.path().to_str().unwrap(),
            "--manifest",
            label_manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    manifests.push(("label", label_manifest));

    let filter_out = NamedTempFile::new().unwrap();
    let filter_manifest = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "filter",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            filter_out.path().to_str().unwrap(),
            "--manifest",
            filter_manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    manifests.push(("filter", filter_manifest));

    let sample_out = NamedTempFile::new().unwrap();
    let sample_manifest = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "sample",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            sample_out.path().to_str().unwrap(),
            "--count",
            "1",
            "--manifest",
            sample_manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    manifests.push(("sample", sample_manifest));

    let balance_out = NamedTempFile::new().unwrap();
    let balance_manifest = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "balance",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            balance_out.path().to_str().unwrap(),
            "--by",
            "phase",
            "--manifest",
            balance_manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    manifests.push(("balance", balance_manifest));

    let pack_out = NamedTempFile::new().unwrap();
    let pack_manifest = NamedTempFile::new().unwrap();
    shogiesa()
        .args([
            "pack",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            pack_out.path().to_str().unwrap(),
            "--manifest",
            pack_manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    manifests.push(("pack", pack_manifest));

    for (command, manifest_file) in manifests {
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(manifest_file.path()).unwrap()).unwrap();
        for key in common_keys {
            assert!(
                manifest.get(key).is_some(),
                "{command} manifest missing common key {key:?}: {manifest}"
            );
        }
    }
}

// --- from-match ---

#[test]
fn from_match_extracts_positions_from_startpos_kifu() {
    let out = NamedTempFile::new().unwrap();
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
    let out = NamedTempFile::new().unwrap();
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
    let out = NamedTempFile::new().unwrap();
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
    let out = NamedTempFile::new().unwrap();
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
    let out = NamedTempFile::new().unwrap();
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
    let out = NamedTempFile::new().unwrap();
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
    let out = NamedTempFile::new().unwrap();
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
    let out = NamedTempFile::new().unwrap();
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
    let out = NamedTempFile::new().unwrap();
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
    let out = NamedTempFile::new().unwrap();
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
    let extracted = NamedTempFile::new().unwrap();
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

    let labeled = NamedTempFile::new().unwrap();
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

    let filtered = NamedTempFile::new().unwrap();
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
fn position_at_ply(ply: u32, observations: serde_json::Value) -> serde_json::Value {
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
    let out = NamedTempFile::new().unwrap();
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
    let out = NamedTempFile::new().unwrap();
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
    let out = NamedTempFile::new().unwrap();
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

// --- distribution ---

/// Mirrors `distribution`'s `print_coverage_row` format (`"{label:<32}{count:>6}  {flag}"`,
/// minus the 2-space println indent, which `contains` doesn't need) so assertions don't have to
/// hand-compute column padding.
fn coverage_row_contains(label: &str, count: usize, flag: &str) -> String {
    format!("{label:<32}{count:>6}  {flag}")
}

/// Mirrors `distribution`'s `print_ply_histogram` row format
/// (`"{b:>4}..{:<4}{count:>6}  {flag}"`).
fn ply_row_contains(b: u32, bucket_size: u32, count: usize, flag: &str) -> String {
    format!("{b:>4}..{:<4}{count:>6}  {flag}", b + bucket_size - 1)
}

#[test]
fn distribution_shows_basic_sections() {
    let pos = NamedTempFile::new().unwrap();
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
        .args(["distribution", "--input", pos.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("=== shogiesa distribution ==="))
        .stdout(predicate::str::contains(
            "phase x side x eval-bucket coverage",
        ))
        .stdout(predicate::str::contains("ply distribution"))
        .stdout(predicate::str::contains("source-root distribution"))
        .stdout(predicate::str::contains("distinct roots"))
        // Sentinel cells (mate/unlabeled x phase/side) are a separate code path from the cp grid
        // -- these positions are unlabeled (extract only, no `label` step), so every sentinel row
        // for phases/sides that don't occur in this 5-ply game must still print, flagged MISSING.
        .stdout(predicate::str::contains(coverage_row_contains(
            "opening:black:mate",
            0,
            "MISSING",
        )))
        .stdout(predicate::str::contains(coverage_row_contains(
            "middlegame:black:unlabeled",
            0,
            "MISSING",
        )))
        .stdout(predicate::str::contains(
            "(sentinel cells (mate/unlabeled x phase/side): 12 cells, 10 empty)",
        ));
}

#[test]
fn distribution_flags_missing_eval_bucket_as_missing() {
    // -250cp -> bucket -400 ("-400..-201"); 250cp -> bucket 200 ("+200..+399"). The span between
    // them (-400, -200, 0, 200) means -200 and 0 are enumerated but never observed for
    // middlegame:black -- exactly the gap this command exists to surface.
    let input = make_labeled_jsonl(&[
        position("middlegame", serde_json::json!([obs("7g7f", -250, 4)])),
        position("middlegame", serde_json::json!([obs("3c3d", 250, 4)])),
    ]);

    shogiesa()
        .args(["distribution", "--input", input.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(coverage_row_contains(
            "middlegame:black:-400..-201",
            1,
            "OK",
        )))
        .stdout(predicate::str::contains(coverage_row_contains(
            "middlegame:black:-200..-1",
            0,
            "MISSING",
        )))
        .stdout(predicate::str::contains(coverage_row_contains(
            "middlegame:black:+0..+199",
            0,
            "MISSING",
        )))
        .stdout(predicate::str::contains(coverage_row_contains(
            "middlegame:black:+200..+399",
            1,
            "OK",
        )));
}

#[test]
fn distribution_ply_histogram_flags_missing_bucket() {
    // Plies 1 and 5 fall in bucket 0..19; ply 45 falls in bucket 40..59. Bucket 20..39 is
    // enumerated (it's within the observed span) but has zero records.
    let input = make_labeled_jsonl(&[
        position_at_ply(1, serde_json::json!([obs("7g7f", 50, 4)])),
        position_at_ply(5, serde_json::json!([obs("7g7f", 50, 4)])),
        position_at_ply(45, serde_json::json!([obs("7g7f", 50, 4)])),
    ]);

    shogiesa()
        .args([
            "distribution",
            "--input",
            input.path().to_str().unwrap(),
            "--ply-bucket-size",
            "20",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(ply_row_contains(0, 20, 2, "OK")))
        .stdout(predicate::str::contains(ply_row_contains(
            20, 20, 0, "MISSING",
        )))
        .stdout(predicate::str::contains(ply_row_contains(40, 20, 1, "OK")));
}

#[test]
fn distribution_source_root_dominance_uses_root_id_not_path() {
    fn record_with_root(path: &str, root_id: &str) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 8,
            "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "source": { "kind": "kif", "path": path, "ply": 1, "root_id": root_id },
            "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
            "observations": []
        })
    }
    // 3 records share root_a (distinct paths -- proving root_id, not path, decides "distinct
    // roots"); 1 record has its own root_b. 3/4 = 75.0% dominance, over the 50% WARN threshold.
    let input = make_labeled_jsonl(&[
        record_with_root("game1.kif", "root_a"),
        record_with_root("game1.kif#var1@2", "root_a"),
        record_with_root("game1.kif#var2@3", "root_a"),
        record_with_root("game2.kif", "root_b"),
    ]);

    shogiesa()
        .args(["distribution", "--input", input.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("distinct roots : 2"))
        .stdout(predicate::str::contains("75.0%"))
        .stdout(predicate::str::contains("WARN"));
}

#[test]
fn distribution_flags_under_and_over_represented_buckets() {
    // All records share side=black, cp=100 (bucket 0, "+0..+199"), differing only by phase --
    // counts 1/1/20 skew the mean (7.33) so opening/middlegame read UNDER and endgame reads OVER.
    let mut records: Vec<serde_json::Value> = vec![position(
        "opening",
        serde_json::json!([obs("7g7f", 100, 4)]),
    )];
    records.push(position(
        "middlegame",
        serde_json::json!([obs("7g7f", 100, 4)]),
    ));
    for _ in 0..20 {
        records.push(position(
            "endgame",
            serde_json::json!([obs("7g7f", 100, 4)]),
        ));
    }
    let input = make_labeled_jsonl(&records);

    shogiesa()
        .args([
            "distribution",
            "--input",
            input.path().to_str().unwrap(),
            "--under-ratio",
            "0.5",
            "--over-ratio",
            "2.0",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(coverage_row_contains(
            "opening:black:+0..+199",
            1,
            "UNDER",
        )))
        .stdout(predicate::str::contains(coverage_row_contains(
            "endgame:black:+0..+199",
            20,
            "OVER",
        )));
}

#[test]
fn distribution_rejects_zero_ply_bucket_size() {
    let input = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    shogiesa()
        .args([
            "distribution",
            "--input",
            input.path().to_str().unwrap(),
            "--ply-bucket-size",
            "0",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--ply-bucket-size must be > 0"));
}

// --- make-gate-openings ---

/// A syntactically valid SFEN (required -- unlike `position_with_path_and_phase`'s fake `sfen`
/// strings, `make-gate-openings` calls `Sfen::parse` and skips anything that fails it) with a
/// caller-chosen `hand` field, which is all that's varied across these fixtures: two records
/// sharing a `hand` value collide on `make-gate-openings`'s board+side+hand dedup key; two with
/// different `hand` values don't. `Sfen::parse` never cross-validates hand-vs-board piece counts,
/// so any piece letter here is safe regardless of the (always-standard) board.
fn gate_record(hand: &str, path: &str, ply: u32) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "sfen": format!("lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b {hand} 1"),
        "source": { "kind": "csa", "path": path, "ply": ply },
        "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
        "observations": []
    })
}

fn gate_sfens_in_order(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect()
}

#[test]
fn make_gate_openings_diversifies_across_roots() {
    // gameA contributes 5 candidates, gameB contributes 1. Quota 2. Rank-0 tier is exactly
    // {gameA's first record, gameB's only record} -- 2 entries, exactly filling the quota -- so
    // both are *always* kept regardless of seed. Mirrors stratify's flagship diversity test,
    // degenerated to make-gate-openings' single universal bucket.
    for seed in ["0", "12345"] {
        let mut records = vec![];
        for (i, hand) in ["P", "L", "N", "S", "G"].iter().enumerate() {
            records.push(gate_record(hand, "gameA.csa", (i + 1) as u32 * 10));
        }
        records.push(gate_record("R", "gameB.csa", 10));
        let input = make_labeled_jsonl(&records);
        let out = NamedTempFile::new().unwrap();

        shogiesa()
            .args([
                "make-gate-openings",
                "--input",
                input.path().to_str().unwrap(),
                "--out",
                out.path().to_str().unwrap(),
                "--count",
                "2",
                "--min-ply",
                "1",
                "--seed",
                seed,
            ])
            .assert()
            .success();

        let sfens = gate_sfens_in_order(&std::fs::read_to_string(out.path()).unwrap());
        assert_eq!(sfens.len(), 2, "seed {seed}");
        assert!(
            sfens.iter().any(|s| s.contains(" P 1")),
            "seed {seed}: gameA's first record must be kept, got {sfens:?}"
        );
        assert!(
            sfens.iter().any(|s| s.contains(" R 1")),
            "seed {seed}: gameB's only record must be represented, got {sfens:?}"
        );
    }
}

#[test]
fn make_gate_openings_dedups_identical_starting_position() {
    // Same board+side+hand (dedup key), different path/ply/trailing move-count, and -- crucially
    // -- different roots, so quota alone can't be what caps the output: each of the 3 records has
    // its own root and would get rank 0 in its own right, so a generous quota (3) that doesn't
    // bind would keep all 3 if dedup weren't actually collapsing the duplicate pair.
    let input = make_labeled_jsonl(&[
        gate_record("P", "gameA.csa", 10),
        gate_record("P", "gameB.csa", 20), // duplicate of the above (same board+side+hand)
        gate_record("L", "gameC.csa", 10),
    ]);
    let out = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "make-gate-openings",
            "--input",
            input.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "3",
            "--min-ply",
            "1",
        ])
        .assert()
        .success();

    let sfens = gate_sfens_in_order(&std::fs::read_to_string(out.path()).unwrap());
    assert_eq!(
        sfens.len(),
        2,
        "the duplicate P-hand record must collapse to one, even though quota (3) doesn't bind: {sfens:?}"
    );
}

#[test]
fn make_gate_openings_dedup_does_not_inflate_rank_for_other_records() {
    // Root A: one real record (hand P), then 2 exact duplicates of it (dropped), then 2 more
    // genuinely distinct records (hand L, hand N). Root B: 3 distinct records (hand S, R, B), no
    // duplicates. Quota 4.
    //
    // If dedup happens BEFORE rank assignment (correct): root A's rank sequence for its 3
    // surviving records is 0, 1, 2 (the 2 dropped duplicates never touch the rank counter) -- same
    // shape as root B's 0, 1, 2. Rank-tier fill: {A@0, B@0} then {A@1, B@1} exactly fills quota 4,
    // giving 2 from each root.
    //
    // If dedup happened AFTER rank assignment (bug): the 2 dropped duplicates would still consume
    // rank slots, inflating root A's surviving records to ranks 0, 3, 4 -- rank-tier fill would
    // then take {A@0, B@0}, {B@1}, {B@2}, giving 1 from root A and 3 from root B instead.
    let input = make_labeled_jsonl(&[
        gate_record("P", "gameA.csa", 10),
        gate_record("P", "gameA.csa", 20), // duplicate of the above (same hand)
        gate_record("P", "gameA.csa", 30), // duplicate of the above (same hand)
        gate_record("L", "gameA.csa", 40),
        gate_record("N", "gameA.csa", 50),
        gate_record("S", "gameB.csa", 10),
        gate_record("R", "gameB.csa", 20),
        gate_record("B", "gameB.csa", 30),
    ]);
    let out = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "make-gate-openings",
            "--input",
            input.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "4",
            "--min-ply",
            "1",
        ])
        .assert()
        .success();

    let sfens = gate_sfens_in_order(&std::fs::read_to_string(out.path()).unwrap());
    let from_a = sfens
        .iter()
        .filter(|s| s.contains(" P 1") || s.contains(" L 1") || s.contains(" N 1"))
        .count();
    let from_b = sfens
        .iter()
        .filter(|s| s.contains(" S 1") || s.contains(" R 1") || s.contains(" B 1"))
        .count();
    assert_eq!(
        (from_a, from_b),
        (2, 2),
        "root A's duplicates must not starve its own later records of quota share: {sfens:?}"
    );
}

#[test]
fn make_gate_openings_min_ply_filters_early_positions() {
    let input = make_labeled_jsonl(&[
        gate_record("P", "game.csa", 1),
        gate_record("L", "game.csa", 7),
        gate_record("N", "game.csa", 8),
        gate_record("S", "game.csa", 20),
    ]);
    let out = NamedTempFile::new().unwrap();
    let manifest = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "make-gate-openings",
            "--input",
            input.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "10",
            "--min-ply",
            "8",
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let sfens = gate_sfens_in_order(&std::fs::read_to_string(out.path()).unwrap());
    assert_eq!(sfens.len(), 2, "only ply>=8 records survive: {sfens:?}");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    assert_eq!(manifest["drop_reasons"]["below_min_ply"], 2);
}

#[test]
fn make_gate_openings_max_ply_filters_late_positions() {
    let input = make_labeled_jsonl(&[
        gate_record("P", "game.csa", 10),
        gate_record("L", "game.csa", 50),
        gate_record("N", "game.csa", 100),
    ]);
    let out = NamedTempFile::new().unwrap();
    let manifest = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "make-gate-openings",
            "--input",
            input.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "10",
            "--min-ply",
            "1",
            "--max-ply",
            "50",
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let sfens = gate_sfens_in_order(&std::fs::read_to_string(out.path()).unwrap());
    assert_eq!(sfens.len(), 2, "only ply<=50 records survive: {sfens:?}");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    assert_eq!(manifest["drop_reasons"]["above_max_ply"], 1);
}

#[test]
fn make_gate_openings_skips_invalid_sfen_without_crashing() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(f, "{}", gate_record("P", "game.csa", 10)).unwrap();
    // A malformed sfen (missing fields) -- must be warned-and-skipped, not crash the run.
    writeln!(
        f,
        r#"{{"schema_version":1,"sfen":"not a valid sfen","source":{{"kind":"csa","path":"bad.csa","ply":10}},"tags":{{"phase":"opening","side_to_move":"black","in_check":false,"has_capture":false}},"observations":[]}}"#
    )
    .unwrap();
    f.flush().unwrap();
    let out = NamedTempFile::new().unwrap();
    let manifest = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "make-gate-openings",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "10",
            "--min-ply",
            "1",
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let sfens = gate_sfens_in_order(&std::fs::read_to_string(out.path()).unwrap());
    assert_eq!(sfens.len(), 1);
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    assert_eq!(manifest["drop_reasons"]["invalid_sfen"], 1);
}

#[test]
fn make_gate_openings_writes_plain_sfen_lines_not_jsonl() {
    let input = make_labeled_jsonl(&[gate_record("P", "game.csa", 10)]);
    let out = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "make-gate-openings",
            "--input",
            input.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "10",
            "--min-ply",
            "1",
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(
        content,
        "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b P 1\n"
    );
    assert!(
        serde_json::from_str::<serde_json::Value>(content.trim()).is_err(),
        "output must be a plain sfen line, not JSON"
    );
}

#[test]
fn make_gate_openings_count_larger_than_dataset_keeps_everything() {
    let input = make_labeled_jsonl(&[
        gate_record("P", "gameA.csa", 10),
        gate_record("L", "gameB.csa", 10),
    ]);
    let out = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "make-gate-openings",
            "--input",
            input.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "100",
            "--min-ply",
            "1",
        ])
        .assert()
        .success();

    let sfens = gate_sfens_in_order(&std::fs::read_to_string(out.path()).unwrap());
    assert_eq!(sfens.len(), 2);
}

#[test]
fn make_gate_openings_count_zero_selects_nothing() {
    let input = make_labeled_jsonl(&[gate_record("P", "gameA.csa", 10)]);
    let out = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "make-gate-openings",
            "--input",
            input.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "0",
            "--min-ply",
            "1",
        ])
        .assert()
        .success();

    assert_eq!(std::fs::read_to_string(out.path()).unwrap(), "");
}

#[test]
fn make_gate_openings_manifest_reports_root_diversity_stats() {
    let input = make_labeled_jsonl(&[
        gate_record("P", "gameA.csa", 10),
        gate_record("L", "gameB.csa", 10),
        gate_record("N", "gameC.csa", 10),
    ]);
    let out = NamedTempFile::new().unwrap();
    let manifest = NamedTempFile::new().unwrap();

    shogiesa()
        .args([
            "make-gate-openings",
            "--input",
            input.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "3",
            "--min-ply",
            "1",
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    assert_eq!(manifest["command"], "make-gate-openings");
    assert_eq!(manifest["records_read"], 3);
    assert_eq!(manifest["records_kept"], 3);
    assert_eq!(manifest["distinct_roots_kept"], 3);
    // 3 roots, 1 record each, in a 3-record suite -- no root has more than its even 1/3 share.
    assert!(
        (manifest["max_root_share_in_any_bucket"].as_f64().unwrap() - (1.0 / 3.0)).abs() < 1e-9
    );
}

// --- lineprior export ---

fn lineprior_export(input: &Path, out: &Path, extra: &[&str]) -> assert_cmd::assert::Assert {
    let mut args = vec![
        "lineprior",
        "export",
        "--input",
        input.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--source",
        "test_v1",
    ];
    args.extend_from_slice(extra);
    shogiesa().args(args).assert()
}

#[test]
fn lineprior_export_creates_jsonl_from_csa() {
    let out = NamedTempFile::new().unwrap();
    lineprior_export(&fixture("sample.csa"), out.path(), &[]).success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 5, "sample.csa has 5 moves");

    let records: Vec<serde_json::Value> = lines
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        records[0]["state"], "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "first move's state must be the initial position, not post-move"
    );
    let expected_outcome = ["success", "failure", "success", "failure", "success"];
    for (rec, exp) in records.iter().zip(expected_outcome) {
        assert_eq!(rec["outcome"], exp);
        assert_eq!(rec["source"], "test_v1");
        assert!(rec["tags"].as_array().unwrap().iter().any(|t| t == "shogi"));
    }
}

#[test]
fn lineprior_export_creates_jsonl_from_kif() {
    let out = NamedTempFile::new().unwrap();
    lineprior_export(&fixture("sample.kif"), out.path(), &[]).success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 5);
    let records: Vec<serde_json::Value> = lines
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // Ply 3 ("２二角成(88)") is an explicit promotion in this fixture.
    assert_eq!(records[2]["action"], "8h2b+");
}

#[test]
fn lineprior_export_max_ply_truncates_but_outcome_still_correct() {
    let out = NamedTempFile::new().unwrap();
    lineprior_export(&fixture("sample.csa"), out.path(), &["--max-ply", "3"]).success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 3);
    let last: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    // The full game (including the truncated tail) must still be walked to resolve outcome --
    // ply 3 is Black's move in a game Black wins, so it must still read "success", not
    // "unknown"/wrong from a walk that stopped early.
    assert_eq!(last["outcome"], "success");
}

#[test]
fn lineprior_export_max_ply_does_not_drop_variation_moves() {
    // `raw_moves` for a KIF game is mainline-first (plies 1..N) then variation branches, which
    // restart at a low ply -- a naive `break` on the mainline's first over-limit move would also
    // skip every later variation row, silently dropping all variation data whenever a corpus's
    // mainlines run past --max-ply (exactly the case the README's own `--max-ply 80` example
    // hits on annotated games with long mainlines).
    let dir = TempDir::new().unwrap();
    let kif_path = dir.path().join("game.kif");
    std::fs::write(
        &kif_path,
        "手合割：平手\n先手：A\n後手：B\n手数----指手\n\
   1 ７六歩(77)   (0:01/0)\n   2 ３四歩(33)   (0:01/0)\n   3 ２六歩(27)   (0:01/0)\n\
\n変化：2手\n   2 ８四歩(83)   (0:01/0)\n   3 ７八金(69)   (0:01/0)\n",
    )
    .unwrap();

    let out = NamedTempFile::new().unwrap();
    lineprior_export(&kif_path, out.path(), &["--max-ply", "2"]).success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let records: Vec<serde_json::Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // Mainline and variation share a sequence_id (root_id-grouped by design), so distinguish
    // them by `action`: mainline ply 2 is "3c3d" (３四歩), the variation's own ply 2 is "8c8d"
    // (８四歩). Both must survive even though the mainline's ply-3 (over-limit) move comes
    // between them in raw emission order.
    let step2_actions: Vec<&str> = records
        .iter()
        .filter(|r| r["step"] == 2)
        .map(|r| r["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        step2_actions.len(),
        2,
        "expected mainline ply2 + variation ply2, got {step2_actions:?}"
    );
    assert!(
        step2_actions.contains(&"8c8d"),
        "variation's own ply-2 move must survive max-ply truncation of the mainline's later move: {step2_actions:?}"
    );
}

#[test]
fn lineprior_export_rejects_invalid_state_format() {
    let out = NamedTempFile::new().unwrap();
    lineprior_export(
        &fixture("sample.csa"),
        out.path(),
        &["--state-format", "foo"],
    )
    .failure();
}

#[test]
fn lineprior_export_sequence_id_stable_across_repeated_runs() {
    let out1 = NamedTempFile::new().unwrap();
    let out2 = NamedTempFile::new().unwrap();
    lineprior_export(&fixture("sample.kif"), out1.path(), &[]).success();
    lineprior_export(&fixture("sample.kif"), out2.path(), &[]).success();

    let ids = |path: &Path| -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).unwrap();
                v["sequence_id"].as_str().unwrap().to_string()
            })
            .collect()
    };
    assert_eq!(ids(out1.path()), ids(out2.path()));
}

#[test]
fn lineprior_export_manifest_shape_and_counts() {
    let dir = TempDir::new().unwrap();
    std::fs::copy(fixture("sample.csa"), dir.path().join("a.csa")).unwrap();
    std::fs::copy(fixture("sample.kif"), dir.path().join("b.kif")).unwrap();

    let out = NamedTempFile::new().unwrap();
    let manifest = NamedTempFile::new().unwrap();
    lineprior_export(
        dir.path(),
        out.path(),
        &["--manifest", manifest.path().to_str().unwrap()],
    )
    .success();

    let m: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    assert_eq!(m["records_exported"], 10);
    assert_eq!(m["games_read"], 2);
    assert_eq!(m["games_skipped"], 0);
    assert_eq!(m["sequence_count"], 2);
    assert_eq!(m["unknown_outcome_count"], 0);
    assert_eq!(
        m["outcome_distribution"]["success"].as_u64().unwrap()
            + m["outcome_distribution"]["failure"].as_u64().unwrap(),
        10
    );
    assert_eq!(m["tag_distribution"]["opening"], 10);
    assert_eq!(m["source"], "test_v1");
}

#[test]
fn lineprior_export_manifest_input_hash_stable_across_repeated_runs() {
    let dir = TempDir::new().unwrap();
    std::fs::copy(fixture("sample.kif"), dir.path().join("a.kif")).unwrap();

    let out1 = NamedTempFile::new().unwrap();
    let out2 = NamedTempFile::new().unwrap();
    let manifest1 = NamedTempFile::new().unwrap();
    let manifest2 = NamedTempFile::new().unwrap();
    lineprior_export(
        dir.path(),
        out1.path(),
        &["--manifest", manifest1.path().to_str().unwrap()],
    )
    .success();
    lineprior_export(
        dir.path(),
        out2.path(),
        &["--manifest", manifest2.path().to_str().unwrap()],
    )
    .success();

    let hash = |p: &Path| -> String {
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        v["input_hash"].as_str().unwrap().to_string()
    };
    assert_eq!(hash(manifest1.path()), hash(manifest2.path()));
}

// --- help smoke test ---

#[test]
fn help_lists_manifest_and_dry_run_flags() {
    shogiesa()
        .args(["label", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--manifest"))
        .stdout(predicate::str::contains("--resume-from"));

    shogiesa()
        .args(["filter", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--dry-run"));
}

#[test]
fn version_flag_reports_crate_version() {
    shogiesa()
        .args(["--version"])
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));

    shogiesa()
        .args(["-V"])
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}
