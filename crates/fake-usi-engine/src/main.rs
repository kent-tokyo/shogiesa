/// Fake USI engine for testing shogiesa-usi.
///
/// Flags:
///   --hang               : sleep forever after receiving "go", simulating a hung engine
///   --spam-info          : after "go", send "info" lines forever without ever sending
///                          "bestmove", simulating an engine that never terminates search
///   --early-stop-depth N : ignore the requested depth and report depth N before bestmove,
///                          simulating an engine that stops early (e.g. found a forced mate)
///   --multipv-margin N   : emit a multipv 2 runner-up line N cp below the bestmove's score,
///                          simulating a MultiPV≥2 engine (used to test policy_margin_cp)
///   --multipv-bound      : like --multipv-margin, but the runner-up line is tagged
///                          "lowerbound" (used to test that bound-tagged runner-ups are ignored)
///   --bestmove MOVE      : report MOVE instead of the default "7g7f" (used to simulate two
///                          engines disagreeing)
///
/// Also honors, sent over stdin (as the real `label` command does via `--engine-option`,
/// since `label` never passes extra argv to the spawned engine):
///   setoption name MultiPV value N        : N>=2 has the same effect as --multipv-margin 310
///   setoption name EarlyStopDepth value N : same effect as --early-stop-depth N
///   setoption name Bestmove value MOVE    : same effect as --bestmove MOVE
use std::io::{self, BufRead, Write};
use std::thread;
use std::time::Duration;

const DEFAULT_MULTIPV_MARGIN: i32 = 310;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let hang = args.iter().any(|a| a == "--hang");
    let spam_info = args.iter().any(|a| a == "--spam-info");
    let mut early_stop_depth: Option<u32> = args
        .iter()
        .position(|a| a == "--early-stop-depth")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok());
    let mut multipv_margin: Option<i32> = args
        .iter()
        .position(|a| a == "--multipv-margin")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok());
    let multipv_bound = args.iter().any(|a| a == "--multipv-bound");
    let mut bestmove: String = args
        .iter()
        .position(|a| a == "--bestmove")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "7g7f".to_string());
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let trimmed = line.trim();
        match trimmed {
            "usi" => {
                writeln!(out, "id name FakeUsiEngine").unwrap();
                writeln!(out, "id author test").unwrap();
                writeln!(out, "usiok").unwrap();
                out.flush().unwrap();
            }
            "isready" => {
                writeln!(out, "readyok").unwrap();
                out.flush().unwrap();
            }
            "usinewgame" => {}
            s if s.starts_with("setoption name MultiPV value ") => {
                let n: u32 = s
                    .rsplit(' ')
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1);
                if n >= 2 && multipv_margin.is_none() {
                    multipv_margin = Some(DEFAULT_MULTIPV_MARGIN);
                }
            }
            s if s.starts_with("setoption name EarlyStopDepth value ") => {
                early_stop_depth = s.rsplit(' ').next().and_then(|v| v.parse().ok());
            }
            s if s.starts_with("setoption name Bestmove value ") => {
                if let Some(mv) = s.rsplit(' ').next() {
                    bestmove = mv.to_string();
                }
            }
            s if s.starts_with("position") => {}
            s if s.starts_with("go") => {
                if hang {
                    thread::sleep(Duration::from_secs(9999));
                }
                if spam_info {
                    loop {
                        writeln!(out, "info depth 1 score cp 0 nodes 1 time 10").unwrap();
                        out.flush().unwrap();
                        thread::sleep(Duration::from_millis(50));
                    }
                }
                let depth: u32 = early_stop_depth.unwrap_or_else(|| {
                    s.split_whitespace()
                        .skip_while(|&t| t != "depth")
                        .nth(1)
                        .and_then(|t| t.parse().ok())
                        .unwrap_or(1)
                });
                let multipv_prefix = if multipv_margin.is_some() || multipv_bound {
                    "multipv 1 "
                } else {
                    ""
                };
                writeln!(
                    out,
                    "info depth {depth} {multipv_prefix}score cp 100 nodes 1000 time 50 pv 7g7f 8h7g"
                )
                .unwrap();
                if multipv_bound {
                    writeln!(
                        out,
                        "info depth {depth} multipv 2 score cp 50 lowerbound nodes 1000 time 50 pv 2g2f 8h7g"
                    )
                    .unwrap();
                } else if let Some(margin) = multipv_margin {
                    writeln!(
                        out,
                        "info depth {depth} multipv 2 score cp {} nodes 1000 time 50 pv 2g2f 8h7g",
                        100 - margin
                    )
                    .unwrap();
                }
                writeln!(out, "bestmove {bestmove}").unwrap();
                out.flush().unwrap();
            }
            "quit" => break,
            _ => {}
        }
    }
}
