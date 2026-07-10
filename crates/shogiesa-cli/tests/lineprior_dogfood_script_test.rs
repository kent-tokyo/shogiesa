use std::path::Path;

use assert_cmd::cargo::cargo_bin;
use tempfile::TempDir;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn fixtures_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// Runs `scripts/lineprior_dogfood.sh` end-to-end against the real, already-built `shogiesa`
/// binary and `tests/fixtures/fake_lineprior.sh` standing in for the external `lineprior` tool --
/// exercises the script's own plumbing (arg parsing, file wiring, jq extraction into report.md)
/// without requiring the real external tool anywhere, including in CI.
#[test]
fn lineprior_dogfood_script_produces_report() {
    let out_dir = TempDir::new().unwrap();
    let status = std::process::Command::new("bash")
        .arg(repo_root().join("scripts/lineprior_dogfood.sh"))
        .args([
            "--games",
            fixtures_dir().to_str().unwrap(),
            "--lineprior",
            fixture("fake_lineprior.sh").to_str().unwrap(),
            "--out",
            out_dir.path().to_str().unwrap(),
            "--source",
            "test_dogfood",
            "--shogiesa",
            cargo_bin("shogiesa").to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "lineprior_dogfood.sh exited non-zero");

    let report = std::fs::read_to_string(out_dir.path().join("report.md")).unwrap();
    assert!(report.contains("# lineprior dogfood report"));
    assert!(report.contains("## Export"));
    assert!(report.contains("## Eval metrics"));
    assert!(report.contains("top5_hit_rate | 0.67"));
    assert!(report.contains("## Best config"));
    assert!(report.contains("## Commands run"));

    let export_manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out_dir.path().join("export_manifest.json")).unwrap(),
    )
    .unwrap();
    assert!(export_manifest["records_exported"].as_u64().unwrap() > 0);
}
