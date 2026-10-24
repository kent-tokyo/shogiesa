use super::*;

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
        let out = closed_temp_file();

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
    let out = closed_temp_file();

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
    let out = closed_temp_file();

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
    let out = closed_temp_file();
    let manifest = closed_temp_file();

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
    let out = closed_temp_file();
    let manifest = closed_temp_file();

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
    let out = closed_temp_file();
    let manifest = closed_temp_file();

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
    let out = closed_temp_file();

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
    let out = closed_temp_file();

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
    let out = closed_temp_file();

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
    let out = closed_temp_file();
    let manifest = closed_temp_file();

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

#[test]
fn make_gate_openings_manifest_records_selection_seed_and_algorithm_version() {
    let input = make_labeled_jsonl(&[gate_record("-", "gameA.csa", 10)]);
    let out = closed_temp_file();
    let manifest = closed_temp_file();

    shogiesa()
        .args([
            "make-gate-openings",
            "--input",
            input.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--count",
            "1",
            "--min-ply",
            "1",
            "--seed",
            "7",
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    assert_eq!(manifest["selection_seed"], 7);
    assert_eq!(
        manifest["selection_algorithm_version"],
        "gate-openings-group-aware-fill-v1"
    );
}

#[test]
fn make_gate_openings_manifest_canonical_valid_count_distinct_from_records_kept_when_count_trims() {
    // 5 distinct starting positions (varied hand -- board+side+hand is the dedup key), same
    // source root, --count 2: all 5 pass every filter (canonical_valid_count), but the quota
    // trims the kept suite down to 2 (records_kept) -- the two must differ to prove
    // canonical_valid_count isn't just an alias for records_kept.
    let input = make_labeled_jsonl(&[
        gate_record("-", "gameA.csa", 10),
        gate_record("P", "gameA.csa", 10),
        gate_record("PP", "gameA.csa", 10),
        gate_record("PPP", "gameA.csa", 10),
        gate_record("PPPP", "gameA.csa", 10),
    ]);
    let out = closed_temp_file();
    let manifest = closed_temp_file();

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
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    assert_eq!(manifest["canonical_valid_count"], 5);
    assert_eq!(manifest["records_kept"], 2);
}

#[test]
fn make_gate_openings_manifest_output_sha256_is_deterministic_and_changes_with_count() {
    let input = make_labeled_jsonl(&[
        gate_record("-", "gameA.csa", 10),
        gate_record("P", "gameA.csa", 10),
        gate_record("PP", "gameA.csa", 10),
    ]);

    let run = |count: &str| -> String {
        let out = closed_temp_file();
        let manifest = closed_temp_file();
        shogiesa()
            .args([
                "make-gate-openings",
                "--input",
                input.path().to_str().unwrap(),
                "--out",
                out.path().to_str().unwrap(),
                "--count",
                count,
                "--min-ply",
                "1",
                "--manifest",
                manifest.path().to_str().unwrap(),
            ])
            .assert()
            .success();
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
        manifest["output_sha256"].as_str().unwrap().to_string()
    };

    let hash_count_2_first = run("2");
    let hash_count_2_second = run("2");
    assert_eq!(
        hash_count_2_first, hash_count_2_second,
        "identical input/seed/count must produce the same output_sha256 across runs"
    );

    let hash_count_3 = run("3");
    assert_ne!(
        hash_count_2_first, hash_count_3,
        "a different --count (different --out contents) must change output_sha256"
    );
}

#[test]
fn make_gate_openings_manifest_distributions_sum_to_records_kept() {
    let input = make_labeled_jsonl(&[
        gate_record("-", "gameA.csa", 10),
        gate_record("P", "gameB.csa", 10),
        gate_record("PP", "gameC.csa", 10),
    ]);
    let out = closed_temp_file();
    let manifest = closed_temp_file();

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
    let records_kept = manifest["records_kept"].as_u64().unwrap();

    for field in [
        "source_distribution",
        "phase_distribution",
        "material_distribution",
    ] {
        let dist = manifest[field].as_object().unwrap();
        let sum: u64 = dist.values().map(|v| v.as_u64().unwrap()).sum();
        assert_eq!(sum, records_kept, "{field} must sum to records_kept");
    }

    // 3 distinct source roots, 1 kept record each.
    assert_eq!(
        manifest["source_distribution"].as_object().unwrap().len(),
        3
    );
    // material differs per record: 122 (board only), 123 (+1 pawn in hand), 124 (+2 pawns).
    let material_dist = manifest["material_distribution"].as_object().unwrap();
    assert_eq!(material_dist.get("122").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(material_dist.get("123").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(material_dist.get("124").and_then(|v| v.as_u64()), Some(1));
}

/// Like `gate_record`, but with a caller-chosen `sfen` instead of the fixed standard-position
/// board -- needed for the unplayable-position tests below, which must inject a real
/// zero-legal-move SFEN rather than vary the standard board's `hand`.
fn gate_record_with_sfen(sfen: &str, path: &str, ply: u32) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "sfen": sfen,
        "source": { "kind": "csa", "path": path, "ply": ply },
        "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
        "observations": []
    })
}

/// One of the shortest mate sequences from startpos, verified against
/// `shogi_legality_lite`'s own `status_partial_works` test (which asserts the side to move here
/// -- White -- is checkmated, i.e. has zero legal moves).
const UNPLAYABLE_MATE_SFEN: &str =
    "lnsg1gsnl/5rkb1/ppppppp+Pp/9/9/9/PPPPPPP1P/1B5R1/LNSGKGSNL w P 8";

#[test]
fn make_gate_openings_keeps_playable_position() {
    let input = make_labeled_jsonl(&[gate_record("P", "game.csa", 10)]);
    let out = closed_temp_file();
    let manifest = closed_temp_file();

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
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let sfens = gate_sfens_in_order(&std::fs::read_to_string(out.path()).unwrap());
    assert_eq!(sfens.len(), 1);
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    assert_eq!(manifest["drop_reasons"]["unplayable"], 0);
}

#[test]
fn make_gate_openings_drops_unplayable_position_by_default() {
    let input = make_labeled_jsonl(&[
        gate_record("P", "gameA.csa", 10),
        gate_record_with_sfen(UNPLAYABLE_MATE_SFEN, "gameB.csa", 8),
    ]);
    let out = closed_temp_file();

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

    let sfens = gate_sfens_in_order(&std::fs::read_to_string(out.path()).unwrap());
    assert!(
        !sfens.iter().any(|s| s == UNPLAYABLE_MATE_SFEN),
        "the zero-legal-move SFEN must be dropped by default: {sfens:?}"
    );
    assert_eq!(
        sfens.len(),
        1,
        "only the playable record survives: {sfens:?}"
    );
}

#[test]
fn make_gate_openings_manifest_reports_unplayable_count() {
    let input = make_labeled_jsonl(&[
        gate_record("P", "gameA.csa", 10),
        gate_record_with_sfen(UNPLAYABLE_MATE_SFEN, "gameB.csa", 8),
    ]);
    let out = closed_temp_file();
    let manifest = closed_temp_file();

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
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    assert_eq!(manifest["drop_reasons"]["unplayable"], 1);
}

#[test]
fn make_gate_openings_allow_unplayable_keeps_zero_legal_move_position() {
    let input = make_labeled_jsonl(&[
        gate_record("P", "gameA.csa", 10),
        gate_record_with_sfen(UNPLAYABLE_MATE_SFEN, "gameB.csa", 8),
    ]);
    let out = closed_temp_file();
    let manifest = closed_temp_file();

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
            "--allow-unplayable",
            "--manifest",
            manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let sfens = gate_sfens_in_order(&std::fs::read_to_string(out.path()).unwrap());
    assert_eq!(
        sfens.len(),
        2,
        "both records kept with --allow-unplayable: {sfens:?}"
    );
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(manifest.path()).unwrap()).unwrap();
    assert_eq!(manifest["drop_reasons"]["unplayable"], 0);
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
    let out = closed_temp_file();
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
    let out = closed_temp_file();
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
    let out = closed_temp_file();
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

    let out = closed_temp_file();
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
    let out = closed_temp_file();
    lineprior_export(
        &fixture("sample.csa"),
        out.path(),
        &["--state-format", "foo"],
    )
    .failure();
}

#[test]
fn lineprior_export_sequence_id_stable_across_repeated_runs() {
    let out1 = closed_temp_file();
    let out2 = closed_temp_file();
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

    let out = closed_temp_file();
    let manifest = closed_temp_file();
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

    let out1 = closed_temp_file();
    let out2 = closed_temp_file();
    let manifest1 = closed_temp_file();
    let manifest2 = closed_temp_file();
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

// --- shuffle ---

fn shuffle_record(path: &str, ply: u32, observations: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "source": { "kind": "csa", "path": path, "ply": ply },
        "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
        "observations": observations
    })
}

fn shuffle_manifest_json(p: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn shuffle_order_lines(p: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(p)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn shuffle_same_seed_same_order_and_hash() {
    let f = make_labeled_jsonl(&[
        source_record("a.csa", 1),
        source_record("b.csa", 1),
        source_record("c.csa", 1),
        source_record("d.csa", 1),
        source_record("e.csa", 1),
    ]);

    let run = || {
        let out = closed_temp_file();
        let manifest = closed_temp_file();
        shogiesa()
            .args([
                "shuffle",
                "--input",
                f.path().to_str().unwrap(),
                "--out",
                out.path().to_str().unwrap(),
                "--seed",
                "7",
                "--manifest",
                manifest.path().to_str().unwrap(),
            ])
            .assert()
            .success();
        (
            std::fs::read_to_string(out.path()).unwrap(),
            shuffle_manifest_json(manifest.path())["order_hash"].clone(),
        )
    };
    assert_eq!(run(), run());
}

#[test]
fn shuffle_different_seed_different_order() {
    let f = make_labeled_jsonl(&[
        source_record("a.csa", 1),
        source_record("b.csa", 1),
        source_record("c.csa", 1),
        source_record("d.csa", 1),
        source_record("e.csa", 1),
    ]);

    let run_with_seed = |seed: &str| {
        let out = closed_temp_file();
        shogiesa()
            .args([
                "shuffle",
                "--input",
                f.path().to_str().unwrap(),
                "--out",
                out.path().to_str().unwrap(),
                "--seed",
                seed,
            ])
            .assert()
            .success();
        std::fs::read_to_string(out.path()).unwrap()
    };
    assert_ne!(run_with_seed("1"), run_with_seed("2"));
}

#[test]
fn shuffle_order_hash_changes_if_any_sample_id_changes() {
    let run_with_ply = |ply: u32| {
        let f = make_labeled_jsonl(&[
            source_record("a.csa", 1),
            source_record("b.csa", ply),
            source_record("c.csa", 1),
        ]);
        let out = closed_temp_file();
        let manifest = closed_temp_file();
        shogiesa()
            .args([
                "shuffle",
                "--input",
                f.path().to_str().unwrap(),
                "--out",
                out.path().to_str().unwrap(),
                "--manifest",
                manifest.path().to_str().unwrap(),
            ])
            .assert()
            .success();
        shuffle_manifest_json(manifest.path())["order_hash"].clone()
    };
    assert_ne!(run_with_ply(1), run_with_ply(2));
}

#[test]
fn shuffle_block_id_arithmetic_at_boundaries() {
    let records: Vec<serde_json::Value> = (0..33)
        .map(|i| source_record(&format!("g{i}.csa"), 1))
        .collect();
    let f = make_labeled_jsonl(&records);
    let out = closed_temp_file();
    let order_manifest = closed_temp_file();
    shogiesa()
        .args([
            "shuffle",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--block-size",
            "32",
            "--order-manifest",
            order_manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let lines = shuffle_order_lines(order_manifest.path());
    assert_eq!(lines.len(), 33);
    for line in &lines {
        let pos = line["training_position"].as_u64().unwrap();
        let expected_block = (pos - 1) / 32;
        assert_eq!(
            line["block_id"].as_u64().unwrap(),
            expected_block,
            "training_position {pos} has wrong block_id"
        );
    }
    assert!(
        lines
            .iter()
            .any(|l| l["training_position"] == 32 && l["block_id"] == 0)
    );
    assert!(
        lines
            .iter()
            .any(|l| l["training_position"] == 33 && l["block_id"] == 1)
    );
}

#[test]
fn shuffle_block_size_zero_rejected() {
    let f = make_labeled_jsonl(&[source_record("a.csa", 1)]);
    let out = closed_temp_file();
    shogiesa()
        .args([
            "shuffle",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--block-size",
            "0",
        ])
        .assert()
        .failure();
}

#[test]
fn shuffle_teacher_flags_omitted_produces_null_columns() {
    let f = make_labeled_jsonl(&[shuffle_record(
        "a.csa",
        1,
        serde_json::json!([obs_with_engine("strong", "7g7f", 50, 10)]),
    )]);
    let out = closed_temp_file();
    let order_manifest = closed_temp_file();
    shogiesa()
        .args([
            "shuffle",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--order-manifest",
            order_manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let lines = shuffle_order_lines(order_manifest.path());
    assert_eq!(lines[0]["teacher_cp"], serde_json::Value::Null);
    assert_eq!(lines[0]["teacher_depth"], serde_json::Value::Null);
}

#[test]
fn shuffle_teacher_flags_select_correct_observation() {
    let f = make_labeled_jsonl(&[shuffle_record(
        "a.csa",
        1,
        serde_json::json!([
            obs_with_engine("weak", "7g7f", 999, 10),
            obs_with_engine("strong", "2g2f", 42, 12),
        ]),
    )]);
    let out = closed_temp_file();
    let order_manifest = closed_temp_file();
    shogiesa()
        .args([
            "shuffle",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--order-manifest",
            order_manifest.path().to_str().unwrap(),
            "--teacher-engine",
            "strong",
            "--teacher-depth",
            "12",
        ])
        .assert()
        .success();

    let lines = shuffle_order_lines(order_manifest.path());
    assert_eq!(lines[0]["teacher_cp"], serde_json::json!(42));
    assert_eq!(lines[0]["teacher_depth"], serde_json::json!(12));
}

#[test]
fn shuffle_teacher_flags_partial_is_rejected() {
    let f = make_labeled_jsonl(&[source_record("a.csa", 1)]);
    let out = closed_temp_file();
    shogiesa()
        .args([
            "shuffle",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--teacher-engine",
            "strong",
        ])
        .assert()
        .failure();
}

#[test]
fn shuffle_split_id_passthrough() {
    let f = make_labeled_jsonl(&[source_record("a.csa", 1), source_record("b.csa", 1)]);

    let out = closed_temp_file();
    let order_manifest = closed_temp_file();
    shogiesa()
        .args([
            "shuffle",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--order-manifest",
            order_manifest.path().to_str().unwrap(),
            "--split-id",
            "train",
        ])
        .assert()
        .success();
    for line in shuffle_order_lines(order_manifest.path()) {
        assert_eq!(line["split_id"], serde_json::json!("train"));
    }

    let out2 = closed_temp_file();
    let order_manifest2 = closed_temp_file();
    shogiesa()
        .args([
            "shuffle",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out2.path().to_str().unwrap(),
            "--order-manifest",
            order_manifest2.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    for line in shuffle_order_lines(order_manifest2.path()) {
        assert_eq!(line["split_id"], serde_json::Value::Null);
    }
}

#[test]
fn shuffle_outcome_column_reflects_game_result() {
    let f = make_labeled_jsonl(&[serde_json::json!({
        "schema_version": 1,
        "sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "source": { "kind": "csa", "path": "a.csa", "ply": 1 },
        "tags": { "phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false },
        "observations": [],
        "game_result": { "outcome": "black_wins", "result_source": "test" }
    })]);
    let out = closed_temp_file();
    let order_manifest = closed_temp_file();
    shogiesa()
        .args([
            "shuffle",
            "--input",
            f.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--order-manifest",
            order_manifest.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let lines = shuffle_order_lines(order_manifest.path());
    assert_eq!(lines[0]["outcome"], serde_json::json!("black_wins"));
}

#[test]
fn shuffle_empty_input_deterministic() {
    let f = closed_temp_file();

    let run = || {
        let out = closed_temp_file();
        let manifest = closed_temp_file();
        shogiesa()
            .args([
                "shuffle",
                "--input",
                f.path().to_str().unwrap(),
                "--out",
                out.path().to_str().unwrap(),
                "--manifest",
                manifest.path().to_str().unwrap(),
            ])
            .assert()
            .success();
        (
            std::fs::read_to_string(out.path()).unwrap(),
            shuffle_manifest_json(manifest.path())["order_hash"].clone(),
        )
    };
    let (content, hash1) = run();
    let (content2, hash2) = run();
    assert_eq!(content, "");
    assert_eq!(content2, "");
    assert_eq!(hash1, hash2);
}

// --- output path safety ---

#[test]
fn transform_rejects_input_as_output_without_truncating_it() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("positions.jsonl");
    std::fs::copy(fixture("pack_input.jsonl"), &input).unwrap();
    let before = std::fs::read(&input).unwrap();

    shogiesa()
        .args([
            "stability",
            "--input",
            input.to_str().unwrap(),
            "--out",
            input.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--input must not be the same path as --out",
        ));

    assert_eq!(std::fs::read(&input).unwrap(), before);
}

#[test]
fn transform_rejects_hard_link_output_alias_without_truncating_input() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("positions.jsonl");
    let alias = dir.path().join("positions.shgpk");
    std::fs::copy(fixture("pack_input.jsonl"), &input).unwrap();
    std::fs::hard_link(&input, &alias).unwrap();
    let before = std::fs::read(&input).unwrap();

    shogiesa()
        .args([
            "pack",
            "--input",
            input.to_str().unwrap(),
            "--out",
            alias.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--input must not be the same path as --out",
        ));

    assert_eq!(std::fs::read(&input).unwrap(), before);
}

#[test]
fn transform_rejects_colliding_output_and_sidecar_without_overwrite() {
    let dir = TempDir::new().unwrap();
    let output = dir.path().join("output.jsonl");
    std::fs::write(&output, b"keep me\n").unwrap();

    shogiesa()
        .args([
            "shuffle",
            "--input",
            fixture("pack_input.jsonl").to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
            "--manifest",
            output.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--out must not be the same path as --manifest",
        ));

    assert_eq!(std::fs::read(&output).unwrap(), b"keep me\n");
}

#[test]
fn filter_rejects_output_aliasing_preset_without_overwrite() {
    let preset = closed_temp_file();
    std::fs::write(preset.path(), b"keep preset\n").unwrap();
    let preset_spec = format!("{}:balanced", preset.path().display());

    shogiesa()
        .args([
            "filter",
            "--input",
            fixture("pack_input.jsonl").to_str().unwrap(),
            "--preset",
            &preset_spec,
            "--out",
            preset.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--preset must not be the same path as --out",
        ));

    assert_eq!(std::fs::read(preset.path()).unwrap(), b"keep preset\n");
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
