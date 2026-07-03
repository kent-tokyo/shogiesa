/// Fake USI engine for testing shogiesa-usi.
///
/// Flags:
///   --hang               : sleep forever after receiving "go", simulating a hung engine
///   --spam-info          : after "go", send "info" lines forever without ever sending
///                          "bestmove", simulating an engine that never terminates search
///   --early-stop-depth N : ignore the requested depth and report depth N before bestmove,
///                          simulating an engine that stops early (e.g. found a forced mate)
use std::io::{self, BufRead, Write};
use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let hang = args.iter().any(|a| a == "--hang");
    let spam_info = args.iter().any(|a| a == "--spam-info");
    let early_stop_depth: Option<u32> = args
        .iter()
        .position(|a| a == "--early-stop-depth")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok());
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
                writeln!(
                    out,
                    "info depth {depth} score cp 100 nodes 1000 time 50 pv 7g7f 8h7g"
                )
                .unwrap();
                writeln!(out, "bestmove 7g7f").unwrap();
                out.flush().unwrap();
            }
            "quit" => break,
            _ => {}
        }
    }
}
