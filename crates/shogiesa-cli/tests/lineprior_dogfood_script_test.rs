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
    assert!(report.contains("lineprior version: lineprior 0.12.3-fixture"));
    assert!(report.contains("lineprior binary SHA-256:"));
    assert!(report.contains("top3_hit_rate | 0.55"));
    assert!(report.contains("top5_hit_rate | 0.67"));
    assert!(report.contains("mrr | 0.44"));
    assert!(report.contains("## Best config"));
    assert!(report.contains("## Commands run"));

    let export_manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out_dir.path().join("export_manifest.json")).unwrap(),
    )
    .unwrap();
    assert!(export_manifest["records_exported"].as_u64().unwrap() > 0);
}

#[test]
fn lineprior_dogfood_script_accepts_shogiesa_path_with_spaces() {
    let temp = TempDir::new().unwrap();
    let bin_dir = temp.path().join("bin with spaces");
    std::fs::create_dir(&bin_dir).unwrap();
    let extension = std::env::consts::EXE_EXTENSION;
    let file_name = if extension.is_empty() {
        "shogiesa copy".to_string()
    } else {
        format!("shogiesa copy.{extension}")
    };
    let copied_bin = bin_dir.join(file_name);
    std::fs::copy(cargo_bin("shogiesa"), &copied_bin).unwrap();

    let out_dir = temp.path().join("run output");
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
            &to_bash_path(&out_dir),
            "--source",
            "test dogfood",
            "--shogiesa",
            &to_bash_path(&copied_bin),
        ])
        .status()
        .unwrap();

    assert!(status.success());
    let report = std::fs::read_to_string(out_dir.join("report.md")).unwrap();
    assert!(report.contains("shogiesa\\ copy"));
    assert!(report.contains("test\\ dogfood"));
}

fn run_dogfood(lineprior_stub: &str, out_dir: &Path, extra: &[&str]) -> std::process::ExitStatus {
    run_dogfood_with_omission(lineprior_stub, out_dir, extra, None)
}

fn run_dogfood_with_omission(
    lineprior_stub: &str,
    out_dir: &Path,
    extra: &[&str],
    omission: Option<&str>,
) -> std::process::ExitStatus {
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
    let mut command = bash_command();
    command
        .arg(to_bash_path(
            &repo_root().join("scripts/lineprior_dogfood.sh"),
        ))
        .args(args);
    if let Some(value) = omission {
        command.env("FAKE_LINEPRIOR_OMIT", value);
    }
    command.status().unwrap()
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
fn lineprior_dogfood_script_rejects_single_sequence_before_tuning() {
    let out_dir = TempDir::new().unwrap();
    let output = bash_command()
        .arg(to_bash_path(
            &repo_root().join("scripts/lineprior_dogfood.sh"),
        ))
        .args([
            "--games",
            &to_bash_path(&fixture("sample.csa")),
            "--lineprior",
            &to_bash_path(&fixture("fake_lineprior.sh")),
            "--out",
            &to_bash_path(out_dir.path()),
            "--source",
            "test_dogfood",
            "--shogiesa",
            &to_bash_path(&cargo_bin("shogiesa")),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("requires at least two sequences for a held-out sequence split; got 1")
    );
    assert!(!out_dir.path().join("shogi_tune_report.json").exists());
}

#[test]
fn lineprior_dogfood_script_preserves_an_existing_run_bundle() {
    let out_dir = TempDir::new().unwrap();
    let report_path = out_dir.path().join("report.md");
    std::fs::write(&report_path, "previous completed run\n").unwrap();

    let status = run_dogfood("fake_lineprior.sh", out_dir.path(), &[]);

    assert!(!status.success());
    assert_eq!(
        std::fs::read_to_string(report_path).unwrap(),
        "previous completed run\n"
    );
    assert!(!out_dir.path().join("shogi_observations.jsonl").exists());
}

#[test]
fn lineprior_dogfood_script_reports_a_missing_option_value() {
    let output = bash_command()
        .arg(to_bash_path(
            &repo_root().join("scripts/lineprior_dogfood.sh"),
        ))
        .arg("--games")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "error: --games requires a value\n"
    );
}

#[test]
fn lineprior_dogfood_script_strict_report_fields_fails_on_missing_top3() {
    let out_dir = TempDir::new().unwrap();
    let status = run_dogfood_with_omission(
        "fake_lineprior_incomplete.sh",
        out_dir.path(),
        &["--strict-report-fields"],
        Some("top3"),
    );
    assert!(!status.success(), "must fail when k=3 is missing");

    let report = std::fs::read_to_string(out_dir.path().join("report.md")).unwrap();
    assert!(report.contains("top3_hit_rate | n/a"));
    assert!(report.contains("top5_hit_rate | 0.67"));
    assert!(report.contains("mrr | 0.44"));
}

#[test]
fn lineprior_dogfood_script_strict_report_fields_fails_on_missing_top5() {
    let out_dir = TempDir::new().unwrap();
    let status = run_dogfood_with_omission(
        "fake_lineprior_incomplete.sh",
        out_dir.path(),
        &["--strict-report-fields"],
        Some("top5"),
    );
    assert!(!status.success(), "must fail when k=5 is missing");

    let report = std::fs::read_to_string(out_dir.path().join("report.md")).unwrap();
    assert!(report.contains("top5_hit_rate | n/a"));
    assert!(report.contains("mrr | 0.44"));
}

#[test]
fn lineprior_dogfood_script_strict_report_fields_fails_on_missing_mrr() {
    let out_dir = TempDir::new().unwrap();
    let status = run_dogfood_with_omission(
        "fake_lineprior_incomplete.sh",
        out_dir.path(),
        &["--strict-report-fields"],
        Some("mrr"),
    );
    assert!(!status.success(), "must fail when MRR is missing");

    let report = std::fs::read_to_string(out_dir.path().join("report.md")).unwrap();
    assert!(report.contains("top5_hit_rate | 0.67"));
    assert!(report.contains("mrr | n/a"));
}
