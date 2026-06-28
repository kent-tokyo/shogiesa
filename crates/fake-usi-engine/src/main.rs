/// Fake USI engine for testing shogiesa-usi.
///
/// Flags:
///   --hang   : sleep forever after receiving "go", simulating a hung engine
use std::io::{self, BufRead, Write};
use std::thread;
use std::time::Duration;

fn main() {
    let hang = std::env::args().any(|a| a == "--hang");
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
                let depth: u32 = s
                    .split_whitespace()
                    .skip_while(|&t| t != "depth")
                    .nth(1)
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(1);
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
