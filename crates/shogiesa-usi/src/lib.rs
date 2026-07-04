use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command as StdCommand, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use shogiesa_core::{CandidateMove, Score, ScoreBound};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum UsiError {
    #[error("IO: {0}")]
    Io(#[from] io::Error),
    #[error("engine did not respond in time")]
    Timeout,
    #[error("unexpected engine response")]
    InvalidResponse,
    #[error("bestmove received without info")]
    NoBestmove,
}

#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub depth: u32,
    pub score: Score,
    pub bestmove: String,
    pub nodes: Option<u64>,
    pub time_ms: Option<u64>,
    pub pv: Option<Vec<String>>,
    /// `score_cp(bestmove) - score_cp(runner_up)` when the engine was run with
    /// MultiPV≥2. `None` if MultiPV wasn't used, either score was a mate score,
    /// or either score was a lowerbound/upperbound rather than a confirmed evaluation.
    pub policy_margin_cp: Option<i32>,
    /// Every MultiPV rank observed, populated only when the engine was run with MultiPV≥2.
    pub candidates: Vec<CandidateMove>,
    /// Whether `score` (the bestmove's own line) is a confirmed evaluation or a search bound.
    /// Always set, independent of MultiPV -- this is what a plain single-PV label would
    /// otherwise silently discard (only `candidates[0].score_bound` carried it before).
    pub score_bound: ScoreBound,
}

struct InfoLine {
    depth: Option<u32>,
    multipv: Option<u32>,
    score: Option<Score>,
    bound: ScoreBound,
    nodes: Option<u64>,
    time_ms: Option<u64>,
    pv: Option<Vec<String>>,
}

fn parse_info(line: &str) -> Option<InfoLine> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.first().copied() != Some("info") {
        return None;
    }
    let mut info = InfoLine {
        depth: None,
        multipv: None,
        score: None,
        bound: ScoreBound::Exact,
        nodes: None,
        time_ms: None,
        pv: None,
    };
    let mut i = 1;
    while i < tokens.len() {
        match tokens[i] {
            "depth" => {
                info.depth = tokens.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            "multipv" => {
                info.multipv = tokens.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            "score" => {
                match tokens.get(i + 1).copied() {
                    Some("cp") => {
                        info.score = tokens
                            .get(i + 2)
                            .and_then(|s| s.parse::<i32>().ok())
                            .map(|v| Score::Cp { value: v });
                        i += 3;
                    }
                    Some("mate") => {
                        info.score = tokens
                            .get(i + 2)
                            .and_then(|s| s.parse::<i32>().ok())
                            .map(|m| Score::Mate { moves: m });
                        i += 3;
                    }
                    _ => {
                        i += 1;
                    }
                }
                match tokens.get(i).copied() {
                    Some("lowerbound") => {
                        info.bound = ScoreBound::Lowerbound;
                        i += 1;
                    }
                    Some("upperbound") => {
                        info.bound = ScoreBound::Upperbound;
                        i += 1;
                    }
                    _ => {}
                }
            }
            "nodes" => {
                info.nodes = tokens.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            "time" => {
                info.time_ms = tokens.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            "pv" => {
                info.pv = Some(tokens[i + 1..].iter().map(|s| s.to_string()).collect());
                break;
            }
            _ => {
                i += 1;
            }
        }
    }
    Some(info)
}

pub struct UsiEngine {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    rx: Receiver<io::Result<String>>,
    pub engine_name: String,
    pub engine_version: Option<String>,
    quit_called: bool,
}

impl UsiEngine {
    /// Launch an engine from a path.
    pub fn launch(
        path: &Path,
        name: String,
        timeout_ms: u64,
        options: &[(String, String)],
    ) -> Result<Self, UsiError> {
        Self::launch_command(StdCommand::new(path), name, timeout_ms, options)
    }

    /// Launch from a pre-built `Command` (useful for passing flags in tests).
    pub fn launch_command(
        mut cmd: StdCommand,
        name: String,
        timeout_ms: u64,
        options: &[(String, String)],
    ) -> Result<Self, UsiError> {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdin = BufWriter::new(child.stdin.take().expect("stdin piped"));
        let stdout = BufReader::new(child.stdout.take().expect("stdout piped"));

        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            for line in stdout.lines() {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });

        let mut engine = UsiEngine {
            child,
            stdin,
            rx,
            engine_name: name,
            engine_version: None,
            quit_called: false,
        };
        engine.handshake(timeout_ms, options)?;
        Ok(engine)
    }

    fn write_line(&mut self, cmd: &str) -> Result<(), UsiError> {
        writeln!(self.stdin, "{cmd}")?;
        self.stdin.flush()?;
        Ok(())
    }

    /// Receive one line, timing out at `deadline` regardless of how many
    /// lines arrive before it — a per-call `recv_timeout` would let a chatty
    /// engine reset the clock on every line and never time out.
    fn recv_until(&self, deadline: Instant) -> Result<String, UsiError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        self.rx
            .recv_timeout(remaining)
            .map_err(|_| UsiError::Timeout)?
            .map_err(UsiError::Io)
    }

    fn handshake(&mut self, timeout_ms: u64, options: &[(String, String)]) -> Result<(), UsiError> {
        self.write_line("usi")?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let line = self.recv_until(deadline)?;
            if line.starts_with("id name ") && self.engine_name.is_empty() {
                self.engine_name = line.strip_prefix("id name ").unwrap_or("").to_string();
            } else if line.starts_with("id version ") {
                self.engine_version =
                    Some(line.strip_prefix("id version ").unwrap_or("").to_string());
            } else if line == "usiok" {
                break;
            }
        }
        for (k, v) in options {
            self.write_line(&format!("setoption name {k} value {v}"))?;
        }
        self.write_line("isready")?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            let line = self.recv_until(deadline)?;
            if line == "readyok" {
                break;
            }
        }
        self.write_line("usinewgame")?;
        Ok(())
    }

    pub fn analyse(
        &mut self,
        sfen: &str,
        depth: u32,
        timeout_ms: u64,
    ) -> Result<AnalysisResult, UsiError> {
        self.write_line(&format!("position sfen {sfen}"))?;
        self.write_line(&format!("go depth {depth}"))?;

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut candidates_by_rank: BTreeMap<u32, InfoLine> = BTreeMap::new();
        loop {
            let line = self.recv_until(deadline)?;
            if line.starts_with("bestmove ") {
                let bestmove = line
                    .strip_prefix("bestmove ")
                    .and_then(|s| s.split_whitespace().next())
                    .ok_or(UsiError::InvalidResponse)?
                    .to_string();
                let info = candidates_by_rank.get(&1).ok_or(UsiError::NoBestmove)?;
                let policy_margin_cp = match (&info.score, candidates_by_rank.get(&2)) {
                    (Some(Score::Cp { value: best }), Some(runner_up))
                        if info.bound == ScoreBound::Exact
                            && runner_up.bound == ScoreBound::Exact =>
                    {
                        match runner_up.score {
                            Some(Score::Cp { value: second }) => Some(best - second),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                let candidates: Vec<CandidateMove> = if candidates_by_rank.len() >= 2 {
                    candidates_by_rank
                        .iter()
                        .filter_map(|(&multipv, info)| {
                            let pv = info.pv.clone()?;
                            let bestmove = pv.first()?.clone();
                            let score = info.score.clone()?;
                            Some(CandidateMove {
                                multipv,
                                bestmove,
                                score,
                                score_bound: info.bound,
                                pv: Some(pv),
                            })
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                return Ok(AnalysisResult {
                    depth: info.depth.unwrap_or(depth),
                    score: info.score.clone().ok_or(UsiError::InvalidResponse)?,
                    bestmove,
                    nodes: info.nodes,
                    time_ms: info.time_ms,
                    pv: info.pv.clone(),
                    policy_margin_cp,
                    candidates,
                    score_bound: info.bound,
                });
            } else if line.starts_with("info ")
                && let Some(info) = parse_info(&line)
            {
                let rank = info.multipv.unwrap_or(1);
                candidates_by_rank.insert(rank, info);
            }
        }
    }

    pub fn quit(&mut self) {
        if !self.quit_called {
            self.quit_called = true;
            let _ = self.write_line("quit");
            let _ = self.child.kill(); // no-op if already exited; guards against hanging engines
            let _ = self.child.wait();
        }
    }
}

impl Drop for UsiEngine {
    fn drop(&mut self) {
        self.quit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_info_cp() {
        let info =
            parse_info("info depth 8 score cp 43 nodes 12345 time 100 pv 7g7f 8h7g").unwrap();
        assert_eq!(info.depth, Some(8));
        assert!(matches!(info.score, Some(Score::Cp { value: 43 })));
        assert_eq!(info.nodes, Some(12345));
        assert_eq!(info.time_ms, Some(100));
        assert_eq!(
            info.pv
                .as_ref()
                .map(|v| v.iter().map(String::as_str).collect::<Vec<_>>()),
            Some(vec!["7g7f", "8h7g"])
        );
    }

    #[test]
    fn parse_info_mate() {
        let info = parse_info("info depth 10 score mate 3 nodes 500 time 20").unwrap();
        assert!(matches!(info.score, Some(Score::Mate { moves: 3 })));
    }

    #[test]
    fn parse_info_rejects_non_info() {
        assert!(parse_info("bestmove 7g7f").is_none());
    }

    #[test]
    fn parse_info_multipv() {
        let info = parse_info("info depth 8 multipv 2 score cp 39 nodes 1 time 1").unwrap();
        assert_eq!(info.multipv, Some(2));
        assert_eq!(info.bound, ScoreBound::Exact);
    }

    #[test]
    fn parse_info_no_multipv_token_is_none() {
        let info = parse_info("info depth 8 score cp 43 nodes 1 time 1").unwrap();
        assert_eq!(info.multipv, None);
    }

    #[test]
    fn parse_info_detects_lowerbound() {
        let info = parse_info("info depth 8 score cp 39 lowerbound nodes 1 time 1").unwrap();
        assert_eq!(info.bound, ScoreBound::Lowerbound);
        assert!(matches!(info.score, Some(Score::Cp { value: 39 })));
    }

    #[test]
    fn parse_info_detects_upperbound() {
        let info = parse_info("info depth 8 score cp 39 upperbound nodes 1 time 1").unwrap();
        assert_eq!(info.bound, ScoreBound::Upperbound);
    }
}
