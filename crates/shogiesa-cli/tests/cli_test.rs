use std::io::Write;
use std::path::Path;

use assert_cmd::Command;
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
