use super::*;

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
    let output = shogiesa()
        .args([
            "distribution",
            "--input",
            fixture("block_report_input.jsonl").to_str().unwrap(),
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
    assert_eq!(normalized, golden("distribution.golden"));
}

#[test]
fn distribution_flags_missing_eval_bucket_as_missing() {
    let output = shogiesa()
        .args([
            "distribution",
            "--input",
            fixture("distribution_missing_bucket_input.jsonl")
                .to_str()
                .unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        golden("distribution_missing_bucket.golden")
    );
}

#[test]
fn distribution_malformed_fixture_keeps_valid_records_and_goldenizes_warning_count() {
    let output = shogiesa()
        .args([
            "distribution",
            "--input",
            fixture("distribution_malformed_input.jsonl")
                .to_str()
                .unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        golden("distribution_malformed.golden")
    );
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
