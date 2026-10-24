use super::*;

#[test]
fn split_rejects_nonfinite_and_negative_ratios_without_touching_bundle() {
    for (ratio_args, message) in [
        (vec!["--valid-frac=-0.1"], "--valid-frac must be finite"),
        (vec!["--test-frac", "NaN"], "--test-frac must be finite"),
    ] {
        let dir = TempDir::new().unwrap();
        let train = dir.path().join("train.jsonl");
        let valid = dir.path().join("valid.jsonl");
        let test = dir.path().join("test.jsonl");
        let manifest = dir.path().join("manifest.json");
        for path in [&train, &valid, &test, &manifest] {
            std::fs::write(path, b"previous").unwrap();
        }

        let mut command = shogiesa();
        command.args([
            "split",
            "--input",
            fixture("dataset_diff_candidate.jsonl").to_str().unwrap(),
            "--train",
            train.to_str().unwrap(),
            "--valid",
            valid.to_str().unwrap(),
            "--test",
            test.to_str().unwrap(),
        ]);
        command
            .args(ratio_args)
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));

        for path in [&train, &valid, &test, &manifest] {
            assert_eq!(std::fs::read(path).unwrap(), b"previous");
        }
    }
}

#[test]
fn recipe_outputs_with_the_same_basename_commit_independently() {
    let dir = TempDir::new().unwrap();
    let input = fixture("dataset_diff_candidate.jsonl");
    let recipe = dir.path().join("recipe.json");
    let spec = serde_json::json!({
        "recipe_version": 1,
        "stages": [{
            "id": "filter",
            "command": "filter",
            "args": [
                "--input", input,
                "--out", "accepted/result.jsonl",
                "--explain-out", "rejected/result.jsonl",
                "--exclude-mate"
            ],
            "inputs": [input],
            "outputs": ["accepted/result.jsonl", "rejected/result.jsonl"]
        }]
    });
    std::fs::write(&recipe, serde_json::to_vec_pretty(&spec).unwrap()).unwrap();
    let run_dir = dir.path().join("run");

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
        .success();

    let accepted = dir.path().join("accepted/result.jsonl");
    let rejected = dir.path().join("rejected/result.jsonl");
    assert!(accepted.is_file());
    assert!(rejected.is_file());
    assert_ne!(accepted, rejected);
}

#[test]
fn label_rejects_unreasonably_large_worker_count_before_launching_engine() {
    let out = closed_temp_file();
    shogiesa()
        .args([
            "label",
            "--input",
            fixture("teacher_calibration_positions.jsonl")
                .to_str()
                .unwrap(),
            "--engine",
            "unused-engine",
            "--depths",
            "4",
            "--jobs",
            "1025",
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--jobs must be at most 1024"));
}

#[test]
fn cache_prune_rejects_day_to_second_overflow() {
    let cache = TempDir::new().unwrap();
    let excessive_days = u64::MAX.to_string();
    shogiesa()
        .args([
            "cache",
            "prune",
            "--cache-dir",
            cache.path().to_str().unwrap(),
            "--older-than-days",
            &excessive_days,
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--older-than-days is too large"));
}

#[test]
fn distribution_rejects_invalid_ratio_controls() {
    let input = fixture("dataset_diff_candidate.jsonl");
    for (args, message) in [
        (
            ["--under-ratio", "NaN", "--over-ratio", "2"],
            "--under-ratio must be finite and non-negative",
        ),
        (
            ["--under-ratio", "3", "--over-ratio", "2"],
            "--under-ratio must be <= --over-ratio",
        ),
    ] {
        let mut command = shogiesa();
        command.args(["distribution", "--input", input.to_str().unwrap()]);
        command.args(args);
        command
            .assert()
            .failure()
            .stderr(predicate::str::contains(message));
    }
}
