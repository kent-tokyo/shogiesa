use std::process::Command;

use assert_cmd::cargo::cargo_bin;
use shogiesa_core::Score;
use shogiesa_usi::{UsiEngine, UsiError};

const STARTPOS: &str = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
const TIMEOUT: u64 = 5000;

fn fake_engine() -> UsiEngine {
    UsiEngine::launch(&cargo_bin("fake-usi-engine"), String::new(), TIMEOUT, &[]).unwrap()
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
fn timeout_returns_error() {
    // fake-usi-engine --hang sleeps forever on "go" commands
    let mut cmd = Command::new(cargo_bin("fake-usi-engine"));
    cmd.arg("--hang");
    let mut engine = UsiEngine::launch_command(cmd, String::new(), TIMEOUT, &[]).unwrap();
    let result = engine.analyse(STARTPOS, 4, 300); // short timeout
    assert!(matches!(result, Err(UsiError::Timeout)));
    // engine.quit() would hang too; just drop (child gets killed on Drop of Child)
}

#[test]
fn timeout_not_reset_by_continuous_info() {
    // fake-usi-engine --spam-info sends "info" lines forever without ever
    // sending "bestmove" — a per-line-reset timeout would never fire here.
    let mut cmd = Command::new(cargo_bin("fake-usi-engine"));
    cmd.arg("--spam-info");
    let mut engine = UsiEngine::launch_command(cmd, String::new(), TIMEOUT, &[]).unwrap();
    let start = std::time::Instant::now();
    let result = engine.analyse(STARTPOS, 4, 300); // short timeout
    assert!(matches!(result, Err(UsiError::Timeout)));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(2),
        "timeout should fire even though the engine keeps sending info lines"
    );
}
