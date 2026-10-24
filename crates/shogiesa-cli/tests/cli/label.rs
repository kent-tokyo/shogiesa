use super::*;

// --- label ---

#[test]
fn label_rejects_malformed_or_ambiguous_search_limits() {
    let input = fixture("teacher_calibration_positions.jsonl");
    for (flag, value, message) in [
        ("--depths", "4,bad,8", "invalid value \"bad\" in --depths"),
        ("--depths", "4,", "--depths contains an empty value"),
        ("--depths", "4,4", "--depths values must be distinct"),
        ("--nodes", "0", "--nodes values must be greater than zero"),
    ] {
        let out = closed_temp_file();
        shogiesa()
            .args([
                "label",
                "--input",
                input.to_str().unwrap(),
                "--engine",
                "unused-engine",
                flag,
                value,
                "--out",
                out.path().to_str().unwrap(),
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
}

#[test]
fn label_rejects_zero_runtime_controls() {
    let input = fixture("teacher_calibration_positions.jsonl");
    for (flag, message) in [
        ("--timeout-ms", "--timeout-ms must be greater than zero"),
        ("--jobs", "--jobs must be greater than zero"),
        ("--multipv", "--multipv must be greater than zero"),
    ] {
        let out = closed_temp_file();
        shogiesa()
            .args([
                "label",
                "--input",
                input.to_str().unwrap(),
                "--engine",
                "unused-engine",
                "--depths",
                "4",
                flag,
                "0",
                "--out",
                out.path().to_str().unwrap(),
            ])
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
}

#[test]
fn label_rejects_malformed_duplicate_and_multipv_engine_options() {
    let input = fixture("teacher_calibration_positions.jsonl");
    let cases: &[(&[&str], &str)] = &[
        (&["--engine-option", "Hash"], "expected NAME=VALUE"),
        (&["--engine-option", "=64"], "NAME must not be empty"),
        (
            &["--engine-option", "Hash=64", "--engine-option", "hash=128"],
            "duplicate --engine-option name",
        ),
        (
            &["--engine-option", "MultiPV=2"],
            "MultiPV must be configured with --multipv",
        ),
    ];
    for (options, message) in cases {
        let out = closed_temp_file();
        let mut command = shogiesa();
        command.args([
            "label",
            "--input",
            input.to_str().unwrap(),
            "--engine",
            "unused-engine",
            "--depths",
            "4",
        ]);
        command.args(*options);
        command.args(["--out", out.path().to_str().unwrap()]);
        command
            .assert()
            .failure()
            .stderr(predicate::str::contains(*message));
    }
}

#[test]
fn label_adds_observations() {
    let pos = closed_temp_file();
    let obs = closed_temp_file();

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
fn label_existing_policy_distinguishes_multipv_setting() {
    let pos = closed_temp_file();
    let single = closed_temp_file();
    let multipv = closed_temp_file();
    let replaced = closed_temp_file();

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
            single.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    // A single-PV observation must not satisfy a MultiPV=2 request at the same depth.
    shogiesa()
        .args([
            "label",
            "--input",
            single.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--multipv",
            "2",
            "--skip-existing",
            "--out",
            multipv.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let value: serde_json::Value = serde_json::from_str(
        std::fs::read_to_string(multipv.path())
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    let observations = value["observations"].as_array().unwrap();
    assert_eq!(observations.len(), 2);
    assert_eq!(
        observations[0]["candidates"].as_array().map_or(0, Vec::len),
        0
    );
    assert_eq!(observations[1]["candidates"].as_array().unwrap().len(), 2);

    // Re-labeling single-PV must not replace the existing MultiPV observation: the setting is
    // part of the request identity just as the search limit is.
    shogiesa()
        .args([
            "label",
            "--input",
            multipv.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--replace-existing",
            "--out",
            replaced.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let value: serde_json::Value = serde_json::from_str(
        std::fs::read_to_string(replaced.path())
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(value["observations"].as_array().unwrap().len(), 2);
}

#[test]
fn label_skip_existing_no_duplicate_on_early_stop_divergence() {
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
    let unordered_out = closed_temp_file();
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
    let ordered_out = closed_temp_file();
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
    let out = closed_temp_file();
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
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();
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
    assert_eq!(manifest["manifest_schema_version"], 1);
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
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();
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
fn label_nodes_sends_go_nodes_and_populates_requested_nodes() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--nodes",
            "50000",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(out.path()).unwrap().trim()).unwrap();
    let obs = &output["observations"][0];
    assert_eq!(obs["search_limit_kind"], "nodes");
    assert_eq!(obs["requested_nodes"], 50000);
    // fake-usi-engine echoes the requested node count back as its reported "nodes" value.
    assert_eq!(obs["nodes"], 50000);
    assert!(obs["requested_depth"].is_null());
}

#[test]
fn label_depths_and_nodes_conflict() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--nodes",
            "50000",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .failure();
}

#[test]
fn label_requires_depths_or_nodes() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .failure();
}

#[test]
fn label_records_seldepth_nps_hashfull() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
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
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let output: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(out.path()).unwrap().trim()).unwrap();
    let obs = &output["observations"][0];
    assert_eq!(obs["seldepth"], 6); // fake engine reports depth + 2
    assert_eq!(obs["nps"], 500000);
    assert_eq!(obs["hashfull"], 321);
}

#[test]
fn label_records_engine_options_hash() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let run = |option: &str| -> String {
        let out = closed_temp_file();
        shogiesa()
            .args([
                "label",
                "--input",
                f.path().to_str().unwrap(),
                "--engine",
                fake_usi_engine_bin().to_str().unwrap(),
                "--depths",
                "4",
                "--engine-option",
                option,
                "--out",
                out.path().to_str().unwrap(),
            ])
            .assert()
            .success();
        let output: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(out.path()).unwrap().trim()).unwrap();
        output["observations"][0]["engine_options_hash"]
            .as_str()
            .unwrap()
            .to_string()
    };

    let hash_a = run("Hash=64");
    assert_eq!(
        hash_a.len(),
        64,
        "full blake3 hex digest, not the truncated cache-key u64"
    );
    assert!(hash_a.chars().all(|c| c.is_ascii_hexdigit()));

    let hash_b = run("Hash=128");
    assert_ne!(
        hash_a, hash_b,
        "different --engine-option values must hash differently"
    );
}

#[test]
fn label_weight_file_populates_weight_sha256_on_observations_and_manifest() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();
    let mut weight_file = NamedTempFile::new().unwrap();
    weight_file.write_all(b"fake nnue weight bytes").unwrap();
    weight_file.flush().unwrap();

    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--weight-file",
            weight_file.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    use sha2::{Digest, Sha256};
    let expected: String = Sha256::digest(b"fake nnue weight bytes")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let output: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(out.path()).unwrap().trim()).unwrap();
    assert_eq!(output["observations"][0]["weight_sha256"], expected);

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["weight_sha256"], expected);
}

#[test]
fn label_weight_file_directory_errors_clearly() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
    let weight_dir = TempDir::new().unwrap();

    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--weight-file",
            weight_dir.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "weight_sha256 requires a single file, got a directory",
        ));
}

#[test]
fn label_manifest_records_owned_provenance_fields() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--experiment-id",
            "exp-1",
            "--candidate-id",
            "cand-1",
            "--baseline-id",
            "base-1",
            "--lineage-id",
            "lineage-1",
            "--init-seed",
            "7",
            "--split-seed",
            "9",
            "--split-sha256",
            "deadbeef",
            "--teacher-manifest-sha256",
            "feedface",
            "--validity",
            "valid",
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["experiment_id"], "exp-1");
    assert_eq!(manifest["candidate_id"], "cand-1");
    assert_eq!(manifest["baseline_id"], "base-1");
    assert_eq!(manifest["lineage_id"], "lineage-1");
    assert_eq!(manifest["init_seed"], 7);
    assert_eq!(manifest["split_seed"], 9);
    assert_eq!(manifest["split_sha256"], "deadbeef");
    assert_eq!(manifest["teacher_manifest_sha256"], "feedface");
    assert_eq!(manifest["validity"], "valid");
}

#[test]
fn label_manifest_binds_input_and_output_bytes_and_passes_typed_verifier() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();
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
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    use sha2::{Digest, Sha256};
    let input_bytes = std::fs::read(f.path()).unwrap();
    let expected_input: String = Sha256::digest(&input_bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let output_bytes = std::fs::read(out.path()).unwrap();
    let expected_output: String = Sha256::digest(&output_bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["dataset_sha256"], expected_input);
    assert_eq!(manifest["output_sha256"], expected_output);

    let typed = shogiesa_core::manifest::parse_run_manifest_v1(
        std::fs::File::open(manifest_path.path()).unwrap(),
    )
    .unwrap();
    let verified = shogiesa_core::manifest::verify_label_run(
        &typed,
        std::fs::File::open(f.path()).unwrap(),
        std::fs::File::open(out.path()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        verified.provenance,
        shogiesa_core::manifest::ProvenanceStatus::Complete
    );
}

#[test]
fn label_manifest_counts_skipped_input_in_records_read() {
    let mut input = NamedTempFile::new().unwrap();
    writeln!(input, "{}", position("opening", serde_json::json!([]))).unwrap();
    writeln!(input, "{{not-json}}").unwrap();
    input.flush().unwrap();
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();

    shogiesa()
        .args([
            "label",
            "--input",
            input.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("2 in, 1 labeled, 1 skipped"));

    let typed = shogiesa_core::manifest::parse_run_manifest_v1(
        std::fs::File::open(manifest_path.path()).unwrap(),
    )
    .unwrap();
    assert_eq!(typed.records_read, 2);
    assert_eq!(typed.records_kept, 1);
    assert_eq!(typed.records_dropped, 1);
    shogiesa_core::manifest::verify_label_run(
        &typed,
        std::fs::File::open(input.path()).unwrap(),
        std::fs::File::open(out.path()).unwrap(),
    )
    .unwrap();
}

#[test]
fn label_manifest_records_binary_sha256() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();
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
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    use sha2::{Digest, Sha256};
    let engine_bytes = std::fs::read(fake_usi_engine_bin()).unwrap();
    let expected: String = Sha256::digest(&engine_bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["binary_sha256"], expected);
}

#[test]
fn label_manifest_records_engine_threads_and_hash_mb() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--engine-option",
            "threads=4",
            "--engine-option",
            "Hash=256",
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    // "threads" (lowercase) must still match "Threads" -- engine_option_u32 is case-insensitive.
    assert_eq!(manifest["engine_threads"], 4);
    assert_eq!(manifest["engine_hash_mb"], 256);
}

#[test]
fn label_usi_strict_off_does_not_flag_illegal_bestmove() {
    // 5i5g is a king "move" two ranks in one step -- not legal for any piece, regardless of
    // board state (see is_legal_bestmove_lite_accepts_drop_and_promotion's own use of it).
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--engine-option",
            "Bestmove=5i5g",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let written = std::fs::read_to_string(out.path()).unwrap();
    let record: serde_json::Value = serde_json::from_str(written.lines().next().unwrap()).unwrap();
    assert_eq!(
        record["observations"].as_array().unwrap().len(),
        1,
        "without --usi-strict, an illegal bestmove is trusted same as before this feature existed"
    );
}

#[test]
fn label_usi_strict_detects_illegal_bestmove_and_discards_observation() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--engine-option",
            "Bestmove=5i5g",
            "--usi-strict",
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["protocol_violations_count"], 1);

    let written = std::fs::read_to_string(out.path()).unwrap();
    let record: serde_json::Value = serde_json::from_str(written.lines().next().unwrap()).unwrap();
    assert_eq!(
        record["observations"].as_array().unwrap().len(),
        0,
        "an illegal bestmove must not be recorded as a trusted observation"
    );
}

#[test]
fn label_without_restart_on_protocol_error_keeps_reusing_engine_after_violation() {
    let f = make_labeled_jsonl(&[
        position_with_path(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "g1.csa",
            serde_json::json!([]),
        ),
        position_with_path(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "g2.csa",
            serde_json::json!([]),
        ),
    ]);
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--engine-option",
            "Bestmove=5i5g",
            "--usi-strict",
            "--jobs",
            "1",
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["protocol_violations_count"], 2);
    assert_eq!(
        manifest["engine_restarts"], 0,
        "--usi-strict alone (no --restart-on-protocol-error) must not relaunch the engine"
    );
}

#[test]
fn label_restart_on_protocol_error_relaunches_engine_after_violation() {
    let f = make_labeled_jsonl(&[
        position_with_path(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "g1.csa",
            serde_json::json!([]),
        ),
        position_with_path(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "g2.csa",
            serde_json::json!([]),
        ),
    ]);
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--engine-option",
            "Bestmove=5i5g",
            "--usi-strict",
            "--restart-on-protocol-error",
            "--jobs",
            "1",
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["protocol_violations_count"], 2);
    assert!(
        manifest["engine_restarts"].as_u64().unwrap() >= 1,
        "--restart-on-protocol-error must relaunch the engine after a detected violation, got {manifest:?}"
    );
}

#[test]
fn label_transcript_on_error_writes_a_file() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
    let transcript_dir = TempDir::new().unwrap();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--engine-option",
            "Bestmove=5i5g",
            "--usi-strict",
            "--transcript-on-error",
            transcript_dir.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let files: Vec<_> = std::fs::read_dir(transcript_dir.path()).unwrap().collect();
    assert_eq!(
        files.len(),
        1,
        "--transcript-on-error must write one file per detected violation"
    );
}

#[test]
fn label_restart_engine_every_cycles_without_losing_data() {
    // 2 limits per record (--depths 4,6) with --restart-engine-every 2: the threshold is only
    // hit *after* both of a record's searches complete (the check runs once, after the `for
    // &limit` loop, not per-iteration) -- proving a periodic restart can't cut a record off
    // mid-way and drop its second limit's observation.
    let f = make_labeled_jsonl(&[
        position_with_path(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "g1.csa",
            serde_json::json!([]),
        ),
        position_with_path(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "g2.csa",
            serde_json::json!([]),
        ),
    ]);
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4,6",
            "--restart-engine-every",
            "2",
            "--jobs",
            "1",
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert!(
        manifest["engine_restarts"].as_u64().unwrap() >= 1,
        "--restart-engine-every 2 must relaunch after every 2 searches, got {manifest:?}"
    );
    assert_eq!(manifest["records_kept"], 2);

    let written = std::fs::read_to_string(out.path()).unwrap();
    for line in written.lines() {
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(
            record["observations"].as_array().unwrap().len(),
            2,
            "a periodic restart landing after a record's last limit must not lose the earlier \
             limit's observation from that same record"
        );
    }
}

#[test]
fn label_dead_engine_unconditionally_restarts_and_recovers_next_position() {
    // Zero Phase 2 flags set -- proves the root-cause fix (a dead engine is now always
    // relaunched) independent of --usi-strict/--restart-on-protocol-error/--restart-engine-every.
    // With --jobs 1, ExitAfterGo=2 kills the child on its own 2nd `go`; the stdout-forwarding
    // channel disconnecting maps straight to UsiError::Io (see recv_until's doc comment), so
    // restart is immediate and deterministic, not a race against a later call's write() tripping
    // EPIPE. Since ExitAfterGo=2 is a static --engine-option reapplied to every relaunch too,
    // each freshly launched process repeats the same pattern: its own go 1 succeeds, its own go 2
    // kills it and triggers another restart. With 5 positions that's an exact
    // succeed/fail/succeed/fail/succeed alternation (1-indexed) -- positions 1, 3, 5 (process
    // #1/#2/#3's own first `go` each) must all be labeled, proving the engine recovers
    // *repeatedly*, not just once.
    let f = make_labeled_jsonl(&[
        position_with_path(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "g1.csa",
            serde_json::json!([]),
        ),
        position_with_path(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "g2.csa",
            serde_json::json!([]),
        ),
        position_with_path(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "g3.csa",
            serde_json::json!([]),
        ),
        position_with_path(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "g4.csa",
            serde_json::json!([]),
        ),
        position_with_path(
            "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            "g5.csa",
            serde_json::json!([]),
        ),
    ]);
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "4",
            "--engine-option",
            "ExitAfterGo=2",
            "--jobs",
            "1",
            "--out",
            out.path().to_str().unwrap(),
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert!(
        manifest["engine_restarts"].as_u64().unwrap() >= 2,
        "a dead engine (Io) must be relaunched every time it dies, even with zero Phase 2 flags \
         set, got {manifest:?}"
    );

    let written = std::fs::read_to_string(out.path()).unwrap();
    let records: Vec<serde_json::Value> = written
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 5);
    for i in [0, 2, 4] {
        assert_eq!(
            records[i]["observations"].as_array().unwrap().len(),
            1,
            "position {} (a freshly (re)launched process's own first `go`) must be labeled, got {:?}",
            i + 1,
            records[i]
        );
    }
}

#[test]
fn label_cache_dir_distinguishes_depth_and_nodes_at_same_numeric_value() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let cache_dir = TempDir::new().unwrap();

    let out1 = closed_temp_file();
    let manifest1 = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--depths",
            "8",
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
    assert_eq!(manifest1["cache_hits"], 0);

    // Same numeric value, but --nodes instead of --depths -- must be a full cache miss, not a
    // false hit against the --depths 8 entry cached above.
    let out2 = closed_temp_file();
    let manifest2 = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            f.path().to_str().unwrap(),
            "--engine",
            fake_usi_engine_bin().to_str().unwrap(),
            "--nodes",
            "8",
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
        manifest2["cache_hits"], 0,
        "--nodes 8 must not hit the --depths 8 cache entry"
    );
}

#[test]
fn shuffle_manifest_records_shuffle_seed() {
    let f = make_labeled_jsonl(&[position("opening", serde_json::json!([]))]);
    let out = closed_temp_file();
    let manifest_path = closed_temp_file();
    shogiesa()
        .args([
            "shuffle",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--seed",
            "42",
            "--manifest",
            manifest_path.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest_path.path()).unwrap()).unwrap();
    assert_eq!(manifest["shuffle_seed"], 42);
}

#[test]
fn label_cache_dir_hits_on_second_run_and_output_matches() {
    let pos = closed_temp_file();
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
    let out1 = closed_temp_file();
    let manifest1 = closed_temp_file();
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
    let out2 = closed_temp_file();
    let manifest2 = closed_temp_file();
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
    let pos = closed_temp_file();
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

    let out = closed_temp_file();
    let manifest = closed_temp_file();
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
    let pos = closed_temp_file();
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

    let out = closed_temp_file();
    let manifest = closed_temp_file();
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
    let pos = closed_temp_file();
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
    let out1 = closed_temp_file();
    let manifest1 = closed_temp_file();
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

    let out2 = closed_temp_file();
    let manifest2 = closed_temp_file();
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

    let out = closed_temp_file();
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

    let out = closed_temp_file();
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
    let out = closed_temp_file();
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
    let same = closed_temp_file();
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
    let out = closed_temp_file();
    let manifest = closed_temp_file();
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
