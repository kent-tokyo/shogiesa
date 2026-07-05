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
fn label_jobs_2_preserves_order_by_default_and_unordered_output_has_same_set() {
    fn tagged_position(ply: u32) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
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
            "--out",
            ordered_out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    assert_eq!(
        plies_of(ordered_out.path()),
        (0..8u64).collect::<Vec<_>>(),
        "default output order must match input order"
    );

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
            "--unordered-output",
            "--out",
            unordered_out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let mut plies = plies_of(unordered_out.path());
    plies.sort_unstable();
    assert_eq!(
        plies,
        (0..8u64).collect::<Vec<_>>(),
        "--unordered-output must still produce the same set of records"
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

// --- help smoke test ---

#[test]
fn help_lists_manifest_and_dry_run_flags() {
    shogiesa()
        .args(["label", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--manifest"));

    shogiesa()
        .args(["filter", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--dry-run"));
}
