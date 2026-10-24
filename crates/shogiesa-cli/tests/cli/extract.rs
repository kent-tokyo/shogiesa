use super::*;

// --- extract ---

#[test]
fn extract_creates_jsonl() {
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
    let out = closed_temp_file();
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

#[test]
fn extract_directory_remains_shallow_by_default() {
    let out = closed_temp_file();
    shogiesa()
        .args([
            "extract",
            "--input",
            fixture("recursive_extract").to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "no .csa, .kif, or .ki2 files found",
        ));
}

#[test]
fn extract_recursive_is_byte_stable_and_uses_sorted_relative_provenance() {
    let out1 = closed_temp_file();
    let out2 = closed_temp_file();
    for out in [&out1, &out2] {
        shogiesa()
            .args([
                "extract",
                "--input",
                fixture("recursive_extract").to_str().unwrap(),
                "--out",
                out.path().to_str().unwrap(),
                "--recursive",
            ])
            .assert()
            .success()
            .stderr(predicate::str::contains(
                "discovery: 4 game files, 1 non-game files skipped, 0 symlinks skipped",
            ));
    }

    let bytes1 = std::fs::read(out1.path()).unwrap();
    let bytes2 = std::fs::read(out2.path()).unwrap();
    assert_eq!(bytes1, bytes2);

    let content = String::from_utf8(bytes1).unwrap();
    let paths = source_paths_in_order(&content);
    assert_eq!(paths.len(), 20);
    assert_eq!(&paths[0..5], &["a/game.csa"; 5]);
    assert_eq!(&paths[5..10], &["b/game.csa"; 5]);
    assert_eq!(&paths[10..15], &["b/nested/game.kif"; 5]);
    assert_eq!(&paths[15..20], &["c/game.ki2"; 5]);
}

#[test]
fn extract_recursive_deduplicates_across_the_whole_tree() {
    let out = closed_temp_file();
    shogiesa()
        .args([
            "extract",
            "--input",
            fixture("recursive_extract").to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--recursive",
            "--dedup",
        ])
        .assert()
        .success();

    let content = std::fs::read_to_string(out.path()).unwrap();
    let paths = source_paths_in_order(&content);
    assert_eq!(&paths[0..5], &["a/game.csa"; 5]);
    assert!(
        !paths.iter().any(|path| path == "b/game.csa"),
        "the duplicate-basename CSA file must be deduplicated against the first subtree"
    );
}

#[cfg(unix)]
#[test]
fn extract_recursive_does_not_follow_symlink_cycles() {
    use std::os::unix::fs::symlink;

    let input = TempDir::new().unwrap();
    std::fs::copy(fixture("sample.csa"), input.path().join("game.csa")).unwrap();
    symlink(input.path(), input.path().join("loop")).unwrap();
    let out = closed_temp_file();

    shogiesa()
        .args([
            "extract",
            "--input",
            input.path().to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--recursive",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("1 symlinks skipped"));

    let content = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(source_paths_in_order(&content), vec!["game.csa"; 5]);
}

#[cfg(unix)]
#[test]
fn extract_recursive_rejects_a_symlink_input_root() {
    use std::os::unix::fs::symlink;

    let parent = TempDir::new().unwrap();
    let real_input = parent.path().join("real");
    std::fs::create_dir(&real_input).unwrap();
    std::fs::copy(fixture("sample.csa"), real_input.join("game.csa")).unwrap();
    let linked_input = parent.path().join("linked");
    symlink(&real_input, &linked_input).unwrap();
    let out = closed_temp_file();

    shogiesa()
        .args([
            "extract",
            "--input",
            linked_input.to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
            "--recursive",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "recursive input root must not be a symlink",
        ));
}

#[test]
fn extract_fixture_error_cases_preserve_valid_data_and_variation_provenance() {
    for (name, expected_positions) in [
        ("malformed.csa", 1usize),
        ("malformed.kif", 1usize),
        ("no-terminal.kif", 1usize),
    ] {
        let out = closed_temp_file();
        shogiesa()
            .args([
                "extract",
                "--input",
                fixture(name).to_str().unwrap(),
                "--out",
                out.path().to_str().unwrap(),
            ])
            .assert()
            .success();

        let lines = std::fs::read_to_string(out.path()).unwrap();
        let records: Vec<serde_json::Value> = lines
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), expected_positions, "fixture {name}");
        assert_eq!(records[0]["source"]["ply"], 1, "fixture {name}");
    }

    let out = closed_temp_file();
    shogiesa()
        .args([
            "extract",
            "--input",
            fixture("variation.kif").to_str().unwrap(),
            "--out",
            out.path().to_str().unwrap(),
        ])
        .assert()
        .success();
    let records: Vec<serde_json::Value> = std::fs::read_to_string(out.path())
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 4);
    assert!(records.iter().any(|record| {
        record["source"]["variation_id"] == serde_json::Value::String("var1".into())
            && record["source"]["path"]
                .as_str()
                .is_some_and(|path| path.contains("#var1@2"))
    }));
}
