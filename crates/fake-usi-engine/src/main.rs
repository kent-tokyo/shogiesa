/// Fake USI engine for testing shogiesa-usi.
///
/// Flags:
///   --hang               : sleep forever after receiving "go", simulating a hung engine
///   --spam-info          : after "go", send "info" lines forever without ever sending
///                          "bestmove", simulating an engine that never terminates search
///   --early-stop-depth N : ignore the requested depth and report depth N before bestmove,
///                          simulating an engine that stops early (e.g. found a forced mate)
///   --multipv-margin N   : cp margin between adjacent multipv ranks,
///                          simulating a MultiPV≥2 engine (used to test policy_margin_cp)
///   --multipv-bound      : tags rank 2 as "lowerbound" (used to test that bound-tagged
///                          runner-ups are ignored)
///   --bestmove-bound     : tags rank 1 (the bestmove) as "lowerbound" (used to test that a
///                          bound-tagged bestmove score is never trusted for policy_margin_cp)
///   --multipv-count N    : emit N multipv-tagged ranks instead of the default 2 (used to test
///                          Observation.candidates capturing every rank, not just top-2)
///   --bestmove MOVE      : report MOVE instead of the default "7g7f" (used to simulate two
///                          engines disagreeing)
///
/// Also honors, sent over stdin (as the real `label` command does via `--engine-option`,
/// since `label` never passes extra argv to the spawned engine):
///   setoption name MultiPV value N        : same effect as --multipv-count N (N>=2)
///   setoption name EarlyStopDepth value N : same effect as --early-stop-depth N
///   setoption name Bestmove value MOVE    : same effect as --bestmove MOVE
///   setoption name SlowMoveCount value N  : sleep SlowDelayMs before answering "go" for whichever
///                                           position's sfen move-count field (the trailing token
///                                           of "position sfen ...") equals N
///   setoption name SlowDelayMs value N    : delay, in ms, used by SlowMoveCount above (used to
///                                           deterministically make one specific position finish
///                                           last, so tests of label's write-order behavior don't
///                                           depend on OS thread-scheduling jitter)
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
    let multipv_margin: Option<i32> = args
        .iter()
        .position(|a| a == "--multipv-margin")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok());
    let multipv_bound = args.iter().any(|a| a == "--multipv-bound");
    let bestmove_bound = args.iter().any(|a| a == "--bestmove-bound");
    let mut multipv_count: u32 = args
        .iter()
        .position(|a| a == "--multipv-count")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut bestmove: String = args
        .iter()
        .position(|a| a == "--bestmove")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "7g7f".to_string());
    let mut current_move_count: Option<u32> = None;
    let mut slow_move_count: Option<u32> = None;
    let mut slow_delay_ms: u64 = 0;
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
                if n >= 2 {
                    multipv_count = n;
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
            s if s.starts_with("setoption name SlowMoveCount value ") => {
                slow_move_count = s.rsplit(' ').next().and_then(|v| v.parse().ok());
            }
            s if s.starts_with("setoption name SlowDelayMs value ") => {
                slow_delay_ms = s
                    .rsplit(' ')
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
            }
            s if s.starts_with("position") => {
                current_move_count = s
                    .split_whitespace()
                    .next_back()
                    .and_then(|t| t.parse().ok());
            }
            s if s.starts_with("go") => {
                if hang {
                    thread::sleep(Duration::from_secs(9999));
                }
                if slow_move_count.is_some() && current_move_count == slow_move_count {
                    thread::sleep(Duration::from_millis(slow_delay_ms));
                }
                if spam_info {
                    loop {
                        writeln!(out, "info depth 1 score cp 0 nodes 1 time 10 pv 7g7f").unwrap();
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
                let effective_count = if multipv_count >= 2 {
                    multipv_count
                } else if multipv_margin.is_some() || multipv_bound || bestmove_bound {
                    2
                } else {
                    0
                };
                if effective_count == 0 {
                    writeln!(
                        out,
                        "info depth {depth} score cp 100 nodes 1000 time 50 pv 7g7f 8h7g"
                    )
                    .unwrap();
                } else {
                    let margin = multipv_margin.unwrap_or(DEFAULT_MULTIPV_MARGIN);
                    for rank in 1..=effective_count {
                        let score = 100 - (rank as i32 - 1) * margin;
                        let mv = if rank == 1 {
                            "7g7f".to_string()
                        } else {
                            format!("{rank}g{rank}f")
                        };
                        let bound_suffix =
                            if (rank == 2 && multipv_bound) || (rank == 1 && bestmove_bound) {
                                " lowerbound"
                            } else {
                                ""
                            };
                        writeln!(
                            out,
                            "info depth {depth} multipv {rank} score cp {score}{bound_suffix} nodes 1000 time 50 pv {mv} 8h7g"
                        )
                        .unwrap();
                    }
                }
                writeln!(out, "bestmove {bestmove}").unwrap();
                out.flush().unwrap();
            }
            "quit" => break,
            _ => {}
        }
    }
}
