use super::*;

// --- report ---

#[test]
fn report_shows_stats() {
    let out = closed_temp_file();
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
fn dataset_diff_fixture_reports_semantic_changes_independent_of_input_order() {
    let json_out = closed_temp_file();
    let output = shogiesa()
        .args([
            "dataset-diff",
            "--baseline",
            fixture("dataset_diff_baseline.jsonl").to_str().unwrap(),
            "--candidate",
            fixture("dataset_diff_candidate.jsonl").to_str().unwrap(),
            "--json-out",
            json_out.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());

    let normalized = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| {
            if line.starts_with("baseline           : ") {
                "baseline           : <baseline>"
            } else if line.starts_with("candidate          : ") {
                "candidate          : <candidate>"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_eq!(normalized, golden("dataset_diff.golden"));

    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(json_out.path()).unwrap()).unwrap();
    assert_eq!(report["report_version"], 1);
    assert_eq!(report["summary"]["unchanged_records"], 1);
    assert_eq!(report["summary"]["changed_records"], 1);
    assert_eq!(report["summary"]["added_records"], 1);
    assert_eq!(report["summary"]["removed_records"], 1);
    assert_eq!(report["field_changes"]["observations"], 1);
    assert_eq!(report["deltas"]["observations"], 1);
    assert_eq!(report["deltas"]["source_roots"]["game-c.kif"], -1);
    assert_eq!(report["deltas"]["source_roots"]["game-d.csa"], 1);
}

#[test]
fn dataset_diff_position_mode_treats_source_path_change_as_a_field_change() {
    let baseline = closed_temp_file();
    let candidate = closed_temp_file();
    let record = std::fs::read_to_string(fixture("dataset_diff_baseline.jsonl"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string()
        + "\n";
    std::fs::write(baseline.path(), &record).unwrap();
    std::fs::write(
        candidate.path(),
        record.replace("game-a.csa", "moved/game-a.csa"),
    )
    .unwrap();

    shogiesa()
        .args([
            "dataset-diff",
            "--baseline",
            baseline.path().to_str().unwrap(),
            "--candidate",
            candidate.path().to_str().unwrap(),
            "--match-by",
            "position",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("matched records    : 1"))
        .stdout(predicate::str::contains("changed records    : 1"))
        .stdout(predicate::str::contains("added records      : 0"))
        .stdout(predicate::str::contains("removed records    : 0"))
        .stdout(predicate::str::contains("  source: 1"));
}

#[test]
fn dataset_diff_reports_physical_line_after_blank_lines() {
    let mut baseline = NamedTempFile::new().unwrap();
    let candidate = NamedTempFile::new().unwrap();
    let record = std::fs::read_to_string(fixture("dataset_diff_baseline.jsonl"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    writeln!(baseline).unwrap();
    writeln!(baseline, "not json").unwrap();
    writeln!(baseline, "{record}").unwrap();
    baseline.flush().unwrap();
    std::fs::write(candidate.path(), format!("{record}\n")).unwrap();

    shogiesa()
        .env("RUST_LOG", "warn")
        .args([
            "dataset-diff",
            "--baseline",
            baseline.path().to_str().unwrap(),
            "--candidate",
            candidate.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::is_match("line.*=.*2").unwrap());
}

#[test]
fn recipe_plan_fixture_is_typed_deterministic_and_dry_run_only() {
    let json_out = closed_temp_file();
    let output = shogiesa()
        .args([
            "recipe",
            "plan",
            "--recipe",
            fixture("recipe_plan.json").to_str().unwrap(),
            "--json-out",
            json_out.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let normalized = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| {
            if line.starts_with("recipe             : ") {
                "recipe             : <recipe>"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_eq!(normalized, golden("recipe_plan.golden"));

    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(json_out.path()).unwrap()).unwrap();
    assert_eq!(report["plan_version"], 1);
    assert_eq!(report["recipe_version"], 1);
    assert_eq!(report["summary"]["ready"], 2);
    assert_eq!(report["summary"]["waiting_for_dependencies"], 1);
    assert_eq!(report["summary"]["blocked_missing_input"], 0);
    assert_eq!(report["executed"], false);
    assert_eq!(report["stages"][2]["dependencies"][0], "pack-candidate");
    assert_eq!(report["stages"][2]["status"], "waiting-for-dependencies");
}

#[test]
fn recipe_plan_rejects_forward_dependencies() {
    shogiesa()
        .args([
            "recipe",
            "plan",
            "--recipe",
            fixture("recipe_forward_dependency.json").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("produced by later stage"));
}

#[test]
fn recipe_plan_rejects_output_outside_recipe_directory() {
    shogiesa()
        .args([
            "recipe",
            "plan",
            "--recipe",
            fixture("recipe_output_escape.json").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("escapes the recipe directory"));
}

#[test]
fn recipe_plan_rejects_json_output_aliasing_recipe_without_overwrite() {
    let dir = TempDir::new().unwrap();
    let recipe = dir.path().join("recipe.json");
    std::fs::copy(fixture("recipe_plan.json"), &recipe).unwrap();
    let before = std::fs::read(&recipe).unwrap();

    shogiesa()
        .args([
            "recipe",
            "plan",
            "--recipe",
            recipe.to_str().unwrap(),
            "--json-out",
            recipe.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--recipe must not be the same path as --json-out",
        ));

    assert_eq!(std::fs::read(&recipe).unwrap(), before);
}

#[test]
fn recipe_plan_rejects_stage_output_overwriting_recipe() {
    let dir = TempDir::new().unwrap();
    let recipe = dir.path().join("recipe.json");
    let spec = serde_json::json!({
        "recipe_version": 1,
        "stages": [{
            "id": "overwrite-recipe",
            "command": "report",
            "args": [],
            "inputs": [],
            "outputs": ["recipe.json"]
        }]
    });
    std::fs::write(&recipe, serde_json::to_vec_pretty(&spec).unwrap()).unwrap();

    shogiesa()
        .args(["recipe", "plan", "--recipe", recipe.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "must not overwrite the recipe file",
        ));
}

#[test]
fn recipe_run_verify_and_reuse_stage_outputs() {
    let dir = TempDir::new().unwrap();
    let baseline = fixture("dataset_diff_baseline.jsonl");
    let candidate = fixture("dataset_diff_candidate.jsonl");
    let recipe = dir.path().join("recipe.json");
    let spec = serde_json::json!({
        "recipe_version": 1,
        "stages": [
            {
                "id": "compare",
                "command": "dataset-diff",
                "args": ["--baseline", baseline, "--candidate", candidate, "--json-out", "diff.json"],
                "inputs": [baseline, candidate],
                "outputs": ["diff.json"]
            },
            {
                "id": "pack",
                "command": "pack",
                "args": ["--input", candidate, "--out", "candidate.shgpk"],
                "inputs": [candidate],
                "outputs": ["candidate.shgpk"]
            },
            {
                "id": "unpack",
                "command": "unpack",
                "args": ["--input", "candidate.shgpk", "--out", "roundtrip.jsonl"],
                "inputs": ["candidate.shgpk"],
                "outputs": ["roundtrip.jsonl"]
            }
        ]
    });
    std::fs::write(&recipe, serde_json::to_vec_pretty(&spec).unwrap()).unwrap();
    let run_dir = dir.path().join("run");
    let first = shogiesa()
        .args([
            "recipe",
            "run",
            "--recipe",
            recipe.to_str().unwrap(),
            "--run-dir",
            run_dir.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(String::from_utf8(first).unwrap().contains("succeeded"));
    let manifest_path = run_dir.join("run.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    let normalized = serde_json::json!({
        "run_version": manifest["run_version"],
        "status": manifest["status"],
        "stages": manifest["stages"].as_array().unwrap().iter().map(|stage| {
            serde_json::json!({
                "id": stage["id"],
                "status": stage["status"],
                "outputs": stage["outputs"].as_array().unwrap().len(),
            })
        }).collect::<Vec<_>>(),
    });
    assert_eq!(
        serde_json::to_string_pretty(&normalized).unwrap() + "\n",
        golden("recipe_run_manifest.golden")
    );
    let mut interrupted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    interrupted["status"] = serde_json::json!("running");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&interrupted).unwrap(),
    )
    .unwrap();
    shogiesa()
        .args([
            "recipe",
            "run",
            "--recipe",
            recipe.to_str().unwrap(),
            "--run-dir",
            run_dir.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--resume"));
    shogiesa()
        .args([
            "recipe",
            "run",
            "--recipe",
            recipe.to_str().unwrap(),
            "--run-dir",
            run_dir.to_str().unwrap(),
            "--resume",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("compare — reused"));
    shogiesa()
        .args([
            "recipe",
            "verify",
            "--recipe",
            recipe.to_str().unwrap(),
            "--run-dir",
            run_dir.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("status             : verified"));
    shogiesa()
        .args([
            "recipe",
            "run",
            "--recipe",
            recipe.to_str().unwrap(),
            "--run-dir",
            run_dir.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("compare — reused"))
        .stdout(predicate::str::contains("unpack — reused"));

    let mut partial: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    partial["status"] = serde_json::json!("running");
    partial["stages"] = serde_json::json!([partial["stages"][0].clone()]);
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&partial).unwrap()).unwrap();
    shogiesa()
        .args([
            "recipe",
            "run",
            "--recipe",
            recipe.to_str().unwrap(),
            "--run-dir",
            run_dir.to_str().unwrap(),
            "--resume",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("compare — reused"))
        .stdout(predicate::str::contains("pack — succeeded"));

    let mut output = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.path().join("roundtrip.jsonl"))
        .unwrap();
    writeln!(output, "tampered").unwrap();
    shogiesa()
        .args([
            "recipe",
            "verify",
            "--recipe",
            recipe.to_str().unwrap(),
            "--run-dir",
            run_dir.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("output hash mismatch"));

    let mut malformed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    malformed["stages"][0]["outputs"] = serde_json::json!([]);
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&malformed).unwrap(),
    )
    .unwrap();
    shogiesa()
        .args([
            "recipe",
            "verify",
            "--recipe",
            recipe.to_str().unwrap(),
            "--run-dir",
            run_dir.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "output count does not match recipe",
        ));
}

#[test]
fn conflict_report_excludes_unknown_draw_and_mate_and_counts_cp_sign_conflicts() {
    let output = shogiesa()
        .args([
            "conflict-report",
            "--input",
            fixture("conflict_report_input.jsonl").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let normalized = stdout
        .lines()
        .map(|line| {
            if line.starts_with("input             : ") {
                "input             : <fixture>"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_eq!(normalized, golden("conflict_report.golden"));
}

#[test]
fn conflict_report_includes_mainline_and_variation_records_in_same_fixture_matrix() {
    let mut mainline = position("middlegame", serde_json::json!([obs("7g7f", 300, 4)]));
    mainline["source"]["path"] = serde_json::json!("game.kif");
    mainline["game_result"] = serde_json::json!({
        "outcome": "black_wins", "result_source": "kif_marker"
    });

    let mut variation = position("middlegame", serde_json::json!([obs("2g2f", 300, 4)]));
    variation["source"] = serde_json::json!({
        "kind": "kif",
        "path": "game.kif#var1@2",
        "ply": 3,
        "root_id": "game.kif",
        "variation_id": "var1",
        "branch_from_ply": 2
    });
    variation["game_result"] = serde_json::json!({
        "outcome": "white_wins", "result_source": "kif_variation"
    });

    let input = make_labeled_jsonl(&[mainline, variation]);
    shogiesa()
        .args(["conflict-report", "--input", input.path().to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("records read      : 2"))
        .stdout(predicate::str::contains("decisive records  : 2"))
        .stdout(predicate::str::contains("evaluated pairs   : 2"))
        .stdout(predicate::str::contains("conflicts         : 1  (50.0%)"))
        .stdout(predicate::str::contains("test / weight=none"))
        .stdout(predicate::str::contains(
            "evaluated=     2 conflicts=     1 (50.0%)",
        ));
}

#[test]
fn conflict_report_deadband_fixture_goldenizes_excluded_cp_count() {
    let output = shogiesa()
        .args([
            "conflict-report",
            "--input",
            fixture("conflict_report_input.jsonl").to_str().unwrap(),
            "--min-abs-cp",
            "300",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let normalized = stdout
        .lines()
        .map(|line| {
            if line.starts_with("input             : ") {
                "input             : <fixture>"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_eq!(normalized, golden("conflict_report_deadband.golden"));
}

#[test]
fn block_report_keeps_roots_separate_and_reports_cp_summary() {
    for (block_size, golden_name) in [
        ("2", "block_report_size2.golden"),
        ("1", "block_report_size1.golden"),
    ] {
        let output = shogiesa()
            .args([
                "block-report",
                "--input",
                fixture("block_report_input.jsonl").to_str().unwrap(),
                "--block-size",
                block_size,
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let normalized = stdout
            .lines()
            .map(|line| {
                if line.starts_with("input             : ") {
                    "input             : <fixture>"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert_eq!(normalized, golden(golden_name), "block size {block_size}");
    }
}

#[test]
fn report_shows_labeled_diagnostics() {
    let pos = closed_temp_file();
    let obs = closed_temp_file();

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
    let pos = closed_temp_file();
    let obs = closed_temp_file();

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
    let pos = closed_temp_file();
    let obs1 = closed_temp_file();
    let obs2 = closed_temp_file();

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
    let out = closed_temp_file();
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

    let packed = closed_temp_file();
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

    let unpacked = closed_temp_file();
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

const VALIDATE_BROKEN_GOLDEN_STDOUT: &str = r#"=== shogiesa validate ===
total lines    : 1
valid JSON     : 0
valid records  : 0
broken lines   : 1
invalid records: 0
invalid SFENs  : 0
duplicate SFENs: 0
tag mismatches : 0  (side_to_move vs SFEN)
schema versions: {}
unsupported schema versions: {}

WARN: 1 broken lines
"#;

#[test]
fn validate_broken_fixture_has_stable_normal_and_strict_outcomes() {
    shogiesa()
        .args([
            "validate",
            "--input",
            fixture("broken.jsonl").to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(VALIDATE_BROKEN_GOLDEN_STDOUT);

    shogiesa()
        .args([
            "validate",
            "--strict",
            "--input",
            fixture("broken.jsonl").to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stdout(VALIDATE_BROKEN_GOLDEN_STDOUT);
}

// --- validate --strict ---

#[test]
fn validate_strict_clean_data_exits_0() {
    let out = closed_temp_file();
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
fn validate_strict_rejects_valid_json_with_invalid_record_shape() {
    let mut f = NamedTempFile::new().unwrap();
    writeln!(
        f,
        r#"{{"schema_version":11,"sfen":"startpos","observations":[]}}"#
    )
    .unwrap();
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
        .stdout(predicate::str::contains("valid JSON     : 1"))
        .stdout(predicate::str::contains("valid records  : 0"))
        .stdout(predicate::str::contains("broken lines   : 0"))
        .stdout(predicate::str::contains("invalid records: 1"))
        .stdout(predicate::str::contains("WARN: 1 invalid position records"));
}

#[test]
fn validate_strict_rejects_future_schema_and_reports_version() {
    let f = make_labeled_jsonl(&[position_with_version(
        shogiesa_core::SCHEMA_VERSION + 1,
        serde_json::json!([]),
        None,
    )]);

    shogiesa()
        .args([
            "validate",
            "--input",
            f.path().to_str().unwrap(),
            "--strict",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("valid JSON     : 1"))
        .stdout(predicate::str::contains("invalid records: 1"))
        .stdout(predicate::str::contains(
            "unsupported schema versions: {12: 1}",
        ));
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
