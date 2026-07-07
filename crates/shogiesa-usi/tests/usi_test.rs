use std::process::Command;

use assert_cmd::cargo::cargo_bin;
use shogiesa_core::{Score, ScoreBound};
use shogiesa_usi::{UsiEngine, UsiError};

const STARTPOS: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
const TIMEOUT: u64 = 5000;

// ponytail: `fake-usi-engine` lives in a sibling crate, so plain `cargo test`
// only builds its unit-test harness, not the plain bin CARGO_BIN_EXE_ needs.
// Build it explicitly once, then reuse assert_cmd's normal lookup.
fn fake_usi_engine_bin() -> std::path::PathBuf {
    static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    ONCE.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let status = Command::new(cargo)
            .args(["build", "-p", "fake-usi-engine"])
            .status()
            .expect("failed to run cargo build");
        assert!(status.success(), "failed to build fake-usi-engine");
    });
    cargo_bin("fake-usi-engine")
}

fn fake_engine() -> UsiEngine {
    UsiEngine::launch(&fake_usi_engine_bin(), String::new(), TIMEOUT, &[]).unwrap()
}

#[test]
fn handshake_succeeds() {
    let mut engine = fake_engine();
    assert_eq!(engine.engine_name, "FakeUsiEngine");
    engine.quit();
}

#[test]
fn analyse_returns_cp_score() {
    let mut engine = fake_engine();
    let result = engine.analyse(STARTPOS, 4, TIMEOUT).unwrap();
    assert!(matches!(result.score, Score::Cp { value: 100 }));
    assert_eq!(result.bestmove, "7g7f");
    assert_eq!(result.depth, 4);
    assert_eq!(result.bestmove_kind, None);
    engine.quit();
}

#[test]
fn analyse_classifies_resign_bestmove() {
    let mut engine = UsiEngine::launch(
        &fake_usi_engine_bin(),
        String::new(),
        TIMEOUT,
        &[("Bestmove".to_string(), "resign".to_string())],
    )
    .unwrap();
    let result = engine.analyse(STARTPOS, 4, TIMEOUT).unwrap();
    assert_eq!(result.bestmove, "resign");
    assert_eq!(
        result.bestmove_kind,
        Some(shogiesa_core::BestMoveKind::Resign)
    );
    engine.quit();
}

#[test]
fn analyse_returns_correct_depth() {
    let mut engine = fake_engine();
    for depth in [4u32, 6, 8] {
        let result = engine.analyse(STARTPOS, depth, TIMEOUT).unwrap();
        assert_eq!(result.depth, depth);
    }
    engine.quit();
}

#[test]
fn analyse_includes_pv() {
    let mut engine = fake_engine();
    let result = engine.analyse(STARTPOS, 4, TIMEOUT).unwrap();
    let pv = result.pv.unwrap();
    assert_eq!(pv[0], "7g7f");
    engine.quit();
}

#[test]
fn analyse_reports_actual_depth_when_engine_stops_early() {
    // fake-usi-engine --early-stop-depth 3 reports depth 3 regardless of the
    // requested depth, simulating an engine that stops early (e.g. forced mate).
    let mut cmd = Command::new(fake_usi_engine_bin());
    cmd.args(["--early-stop-depth", "3"]);
    let mut engine = UsiEngine::launch_command(cmd, String::new(), TIMEOUT, &[]).unwrap();
    let result = engine.analyse(STARTPOS, 8, TIMEOUT).unwrap();
    assert_eq!(
        result.depth, 3,
        "should report the depth the engine actually reached, not the requested depth"
    );
    engine.quit();
}

#[test]
fn analyse_computes_policy_margin_from_multipv() {
    // fake-usi-engine --multipv-margin 310 reports a multipv 2 runner-up 310cp
    // below the bestmove's score.
    let mut cmd = Command::new(fake_usi_engine_bin());
    cmd.args(["--multipv-margin", "310"]);
    let mut engine = UsiEngine::launch_command(cmd, String::new(), TIMEOUT, &[]).unwrap();
    let result = engine.analyse(STARTPOS, 4, TIMEOUT).unwrap();
    assert_eq!(result.policy_margin_cp, Some(310));
    engine.quit();
}

#[test]
fn analyse_ignores_bound_tagged_runner_up() {
    // fake-usi-engine --multipv-bound sends a multipv 2 line tagged "lowerbound",
    // which should not be trusted as a real evaluation for margin purposes.
    let mut cmd = Command::new(fake_usi_engine_bin());
    cmd.arg("--multipv-bound");
    let mut engine = UsiEngine::launch_command(cmd, String::new(), TIMEOUT, &[]).unwrap();
    let result = engine.analyse(STARTPOS, 4, TIMEOUT).unwrap();
    assert_eq!(result.policy_margin_cp, None);
    engine.quit();
}

#[test]
fn analyse_ignores_bound_tagged_bestmove() {
    // fake-usi-engine --bestmove-bound tags rank 1 (the bestmove) itself as "lowerbound" --
    // a bound-tagged bestmove score must not be trusted for margin purposes either, even
    // though the runner-up (rank 2) is a confirmed exact score.
    let mut cmd = Command::new(fake_usi_engine_bin());
    cmd.args(["--bestmove-bound", "--multipv-margin", "10"]);
    let mut engine = UsiEngine::launch_command(cmd, String::new(), TIMEOUT, &[]).unwrap();
    let result = engine.analyse(STARTPOS, 4, TIMEOUT).unwrap();
    assert_eq!(result.policy_margin_cp, None);
    // The bound tag on the bestmove's own line must surface on AnalysisResult.score_bound --
    // this is what a plain single-PV label (no MultiPV at all) would otherwise silently lose.
    assert_eq!(result.score_bound, ScoreBound::Lowerbound);
    engine.quit();
}

#[test]
fn analyse_without_multipv_has_no_margin() {
    let mut engine = fake_engine();
    let result = engine.analyse(STARTPOS, 4, TIMEOUT).unwrap();
    assert_eq!(result.policy_margin_cp, None);
    assert!(result.candidates.is_empty());
    engine.quit();
}

#[test]
fn analyse_returns_all_multipv_candidates() {
    // fake-usi-engine --multipv-count 4 emits 4 multipv-tagged ranks; candidates must capture
    // every rank, not just the top-2 used for policy_margin_cp.
    let mut cmd = Command::new(fake_usi_engine_bin());
    cmd.args(["--multipv-count", "4", "--multipv-margin", "10"]);
    let mut engine = UsiEngine::launch_command(cmd, String::new(), TIMEOUT, &[]).unwrap();
    let result = engine.analyse(STARTPOS, 4, TIMEOUT).unwrap();
    assert_eq!(result.candidates.len(), 4);
    for (i, candidate) in result.candidates.iter().enumerate() {
        let rank = (i + 1) as u32;
        assert_eq!(candidate.multipv, rank);
        let expected_score = 100 - (rank as i32 - 1) * 10;
        assert!(matches!(
            candidate.score,
            shogiesa_core::Score::Cp { value } if value == expected_score
        ));
    }
    assert_eq!(result.candidates[0].bestmove, "7g7f");
    assert_eq!(result.candidates[1].bestmove, "2g2f");
    assert_eq!(result.policy_margin_cp, Some(10));
    engine.quit();
}

#[test]
fn timeout_returns_error() {
    // fake-usi-engine --hang sleeps forever on "go" commands, producing zero output and never
    // reacting to the `stop` sent on timeout either -- there's nothing for the salvage fallback
    // to salvage, so this stays a hard failure, unchanged from before the timeout-salvage fix.
    let mut cmd = Command::new(fake_usi_engine_bin());
    cmd.arg("--hang");
    let mut engine = UsiEngine::launch_command(cmd, String::new(), TIMEOUT, &[]).unwrap();
    let result = engine.analyse(STARTPOS, 4, 300); // short timeout
    assert!(matches!(result, Err(UsiError::Timeout)));
    // engine.quit() would hang too; just drop (child gets killed on Drop of Child)
}

#[test]
fn timeout_salvages_last_known_depth_when_no_bestmove_ever_arrives() {
    // fake-usi-engine --spam-info sends "info" lines forever without ever sending "bestmove" --
    // a per-line-reset timeout would never fire here, and it can't react to the `stop` sent on
    // timeout either (same single-threaded stdin-loop limitation as --hang). So this exercises
    // the final fallback: no bestmove even after the stop grace period, salvage from the last
    // known info line's own PV instead of returning nothing.
    let mut cmd = Command::new(fake_usi_engine_bin());
    cmd.arg("--spam-info");
    let mut engine = UsiEngine::launch_command(cmd, String::new(), TIMEOUT, &[]).unwrap();
    let start = std::time::Instant::now();
    let result = engine.analyse(STARTPOS, 4, 300).unwrap(); // short timeout
    assert!(result.timed_out);
    assert_eq!(result.depth, 1);
    assert!(matches!(result.score, Score::Cp { value: 0 }));
    assert_eq!(result.bestmove, "7g7f");
    assert!(
        start.elapsed() < std::time::Duration::from_secs(2),
        "timeout should fire even though the engine keeps sending info lines"
    );
}

#[test]
fn timeout_grace_period_recovers_a_real_bestmove() {
    // SlowMoveCount/SlowDelayMs (built for label's write-order jitter fix) makes the engine sleep
    // once before its normal single-shot info+bestmove response -- set the delay to land well
    // after the outer timeout but comfortably inside the stop grace period, so this exercises
    // "a real bestmove arrives during the grace period" rather than the last-resort PV fallback.
    // 200ms outer timeout / 350ms delay gives a 150ms margin on the lower edge (timeout must fire
    // before the response lands) and 350ms on the upper edge (response must land within the 500ms
    // grace window) -- both comfortably wide so this doesn't flake under scheduler jitter/CI load.
    let mut engine = UsiEngine::launch(
        &fake_usi_engine_bin(),
        String::new(),
        TIMEOUT,
        &[
            ("SlowMoveCount".to_string(), "1".to_string()),
            ("SlowDelayMs".to_string(), "350".to_string()),
        ],
    )
    .unwrap();
    let result = engine.analyse(STARTPOS, 4, 200).unwrap(); // outer timeout fires well before 350ms
    assert!(result.timed_out);
    assert!(matches!(result.score, Score::Cp { value: 100 }));
    assert_eq!(result.bestmove, "7g7f");
    assert_eq!(result.depth, 4);
    engine.quit();
}
