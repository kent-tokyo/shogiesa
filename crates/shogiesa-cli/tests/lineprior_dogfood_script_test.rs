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

/// `bash` on PATH resolves to the `C:\Windows\System32\bash.exe` WSL launcher stub on GitHub's
/// `windows-latest` runners (present even without WSL installed), not Git for Windows' real bash
/// -- invoking it fails immediately with "Windows Subsystem for Linux has no installed
/// distributions." Git for Windows (pre-installed on that image) ships its own bash at this fixed
/// path, so use it explicitly on Windows rather than relying on PATH order.
fn bash_command() -> std::process::Command {
    if cfg!(windows) {
        std::process::Command::new(r"C:\Program Files\Git\bin\bash.exe")
    } else {
        std::process::Command::new("bash")
    }
}

/// A Windows `\`-separated path is not safe to pass as a bash argument verbatim: bash's own
/// argument tokenizing treats `\` as an escape character, silently consuming it and the
/// following letter (observed in CI: `D:\a\shogiesa\...` arrived at bash as
/// `D:ashogiesa...`, since `\a`/`\s`/etc. were each swallowed as escaped-letter sequences) --
/// this made every `--games`/`--lineprior`/`--out`/`--shogiesa` argument unusable on
/// `windows-latest`, and even the script path itself (passed as bash's own argv). Git Bash (MSYS)
/// accepts `/`-separated Windows paths (`D:/a/shogiesa/...`) natively, so converting is enough --
/// no `/d/a/...`-style MSYS path translation needed. A no-op on non-Windows, where `\` isn't a
/// path separator to begin with.
fn to_bash_path(p: &Path) -> String {
    let s = p.to_str().unwrap().to_string();
    if cfg!(windows) {
        s.replace('\\', "/")
    } else {
        s
    }
}

/// Runs `scripts/lineprior_dogfood.sh` end-to-end against the real, already-built `shogiesa`
/// binary and `tests/fixtures/fake_lineprior.sh` standing in for the external `lineprior` tool --
/// exercises the script's own plumbing (arg parsing, file wiring, jq extraction into report.md)
/// without requiring the real external tool anywhere, including in CI.
#[test]
fn lineprior_dogfood_script_produces_report() {
    let out_dir = TempDir::new().unwrap();
    let status = bash_command()
        .arg(to_bash_path(
            &repo_root().join("scripts/lineprior_dogfood.sh"),
        ))
        .args([
            "--games",
            &to_bash_path(&fixtures_dir()),
            "--lineprior",
            &to_bash_path(&fixture("fake_lineprior.sh")),
            "--out",
            &to_bash_path(out_dir.path()),
            "--source",
            "test_dogfood",
            "--shogiesa",
            &to_bash_path(&cargo_bin("shogiesa")),
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

fn run_dogfood(lineprior_stub: &str, out_dir: &Path, extra: &[&str]) -> std::process::ExitStatus {
    let mut args = vec![
        "--games".to_string(),
        to_bash_path(&fixtures_dir()),
        "--lineprior".to_string(),
        to_bash_path(&fixture(lineprior_stub)),
        "--out".to_string(),
        to_bash_path(out_dir),
        "--source".to_string(),
        "test_dogfood".to_string(),
        "--shogiesa".to_string(),
        to_bash_path(&cargo_bin("shogiesa")),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    bash_command()
        .arg(to_bash_path(
            &repo_root().join("scripts/lineprior_dogfood.sh"),
        ))
        .args(args)
        .status()
        .unwrap()
}

#[test]
fn lineprior_dogfood_script_strict_report_fields_passes_with_complete_metrics() {
    let out_dir = TempDir::new().unwrap();
    let status = run_dogfood(
        "fake_lineprior.sh",
        out_dir.path(),
        &["--strict-report-fields"],
    );
    assert!(status.success());
    assert!(out_dir.path().join("report.md").exists());
}

#[test]
fn lineprior_dogfood_script_strict_report_fields_fails_on_missing_metrics() {
    let out_dir = TempDir::new().unwrap();
    let status = run_dogfood(
        "fake_lineprior_incomplete.sh",
        out_dir.path(),
        &["--strict-report-fields"],
    );
    assert!(
        !status.success(),
        "must fail when top5_hit_rate/mrr are missing under --strict-report-fields"
    );

    // report.md is still written for debugging, even though the run itself failed.
    let report = std::fs::read_to_string(out_dir.path().join("report.md")).unwrap();
    assert!(report.contains("top1_hit_rate | 0.31"));
    assert!(report.contains("top5_hit_rate | n/a"));
}
