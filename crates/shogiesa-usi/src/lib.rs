use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command as StdCommand, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use shogiesa_core::{BestMoveKind, CandidateMove, Score, ScoreBound, SearchLimitKind};
use thiserror::Error;

/// Classifies a raw USI `bestmove` token. `None` for an ordinary move -- kept out of storage via
/// `Option` since that's the overwhelming common case. Delegates to `shogiesa-core` so the token
/// set (resign/win/none) is defined in exactly one place, shared with `effective_bestmove_kind`'s
/// legacy-JSONL fallback.
pub fn classify_bestmove(token: &str) -> Option<BestMoveKind> {
    shogiesa_core::classify_bestmove_token(token)
}

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
    #[error("bestmove received before any `go` was ever sent")]
    BestmoveWithoutGo,
    #[error("second bestmove received for the same `go`")]
    DuplicateBestmove,
    #[error("bestmove arrived after a timeout was already salvaged for its `go`")]
    DelayedBestmoveAfterTimeout,
    #[error(
        "engine returned a move that is not legal in the position it was given: {mv} (sfen {sfen})"
    )]
    IllegalBestmove { sfen: String, mv: String },
}

/// The USI `go` limit `label` asked for -- `Depth(d)` sends `go depth {d}`, `Nodes(n)` sends
/// `go nodes {n}`. Distinct from `shogiesa_core::SearchLimitKind` (a bare JSON tag with no
/// payload, used on `Observation`) -- this carries the actual requested value for building the
/// USI command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchLimit {
    Depth(u32),
    Nodes(u64),
}

impl SearchLimit {
    pub fn kind(self) -> SearchLimitKind {
        match self {
            SearchLimit::Depth(_) => SearchLimitKind::Depth,
            SearchLimit::Nodes(_) => SearchLimitKind::Nodes,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub depth: u32,
    pub score: Score,
    pub bestmove: String,
    pub nodes: Option<u64>,
    pub time_ms: Option<u64>,
    /// The engine's own `info seldepth` at the bestmove line.
    pub seldepth: Option<u32>,
    /// The engine's own `info nps` at the bestmove line.
    pub nps: Option<u64>,
    /// The engine's own `info hashfull` (per-mille) at the bestmove line.
    pub hashfull: Option<u32>,
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
    /// Set when `bestmove` is a special USI token (`resign`/`win`/`none`) rather than an
    /// ordinary move. See `classify_bestmove`.
    pub bestmove_kind: Option<BestMoveKind>,
    /// `true` when this result was salvaged after `--timeout-ms` elapsed (see
    /// `UsiEngine::salvage_after_timeout`), rather than a `bestmove` line arriving normally.
    /// Not propagated to `Observation`/JSONL -- `depth`/`requested_depth` already carry the
    /// data-quality signal a consumer needs (same shape as an engine's own early stop). This
    /// field exists purely so the CLI can count how often timeouts, specifically, forced an
    /// under-reached result, distinct from an engine's own early stop.
    pub timed_out: bool,
}

struct InfoLine {
    depth: Option<u32>,
    multipv: Option<u32>,
    score: Option<Score>,
    bound: ScoreBound,
    nodes: Option<u64>,
    time_ms: Option<u64>,
    seldepth: Option<u32>,
    nps: Option<u64>,
    hashfull: Option<u32>,
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
        seldepth: None,
        nps: None,
        hashfull: None,
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
            "seldepth" => {
                info.seldepth = tokens.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            "nps" => {
                info.nps = tokens.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            "hashfull" => {
                info.hashfull = tokens.get(i + 1).and_then(|s| s.parse().ok());
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

/// How long `analyse()` waits for a response after sending `stop` on timeout, before giving up
/// and salvaging from whatever `info` line was last captured. Fixed, not a CLI flag -- a secondary
/// timing knob for a secondary path isn't worth exposing. Generous enough for realistic USI
/// engine stop-compliance latency (engines typically check a stop flag every few tens of ms)
/// without meaningfully hurting throughput -- a fixed additive cost only on the already-failing
/// tail of calls that would otherwise return nothing at all.
const STOP_GRACE_MS: u64 = 500;

pub struct UsiEngine {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    rx: Receiver<io::Result<String>>,
    pub engine_name: String,
    pub engine_version: Option<String>,
    quit_called: bool,
    /// `--usi-strict`: detect protocol violations (unsolicited/duplicate/delayed bestmove,
    /// illegal bestmove) instead of silently discarding them. Off by default via `with_strict`.
    strict: bool,
    /// Sticky: true once the first `go` has ever been written to this engine instance. Lets a
    /// bestmove found before any `go` be distinguished from a duplicate/delayed one.
    go_ever_sent: bool,
    /// True iff the previous `analyse()` call's timeout was salvaged by synthesizing from the
    /// last `info` line without a real `bestmove` ever arriving -- one is still owed. Checked at
    /// the top of the next `analyse()` call.
    awaiting_late_bestmove: bool,
    pending_handshake_bestmoves: Vec<String>,
    /// `--transcript-on-error`: capture raw USI lines (both directions) for the current exchange.
    capture_transcript: bool,
    transcript: Vec<String>,
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
            strict: false,
            go_ever_sent: false,
            awaiting_late_bestmove: false,
            pending_handshake_bestmoves: Vec::new(),
            capture_transcript: false,
            transcript: Vec::new(),
        };
        engine.handshake(timeout_ms, options)?;
        Ok(engine)
    }

    /// Enables `--usi-strict` protocol-violation detection. Call any time after `launch()`/
    /// `launch_command()` returns (handshake has already finished by then). Off by default, so
    /// existing callers are unaffected without this call.
    pub fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Enables `--transcript-on-error` raw-line capture. Off by default (no buffering cost
    /// unless set). Handshake traffic is never captured, since this is only ever called after
    /// `launch()`/`launch_command()` (which already ran the handshake) returns.
    pub fn with_transcript_capture(mut self, capture: bool) -> Self {
        self.capture_transcript = capture;
        self
    }

    /// Raw USI lines (both directions) since the start of the current (or most recently failed)
    /// exchange. Empty unless `with_transcript_capture(true)` was set.
    pub fn transcript(&self) -> &[String] {
        &self.transcript
    }

    fn write_line(&mut self, cmd: &str) -> Result<(), UsiError> {
        if self.capture_transcript {
            self.transcript.push(format!("> {cmd}"));
        }
        writeln!(self.stdin, "{cmd}")?;
        self.stdin.flush()?;
        Ok(())
    }

    /// Receive one line, timing out at `deadline` regardless of how many
    /// lines arrive before it — a per-call `recv_timeout` would let a chatty
    /// engine reset the clock on every line and never time out.
    ///
    /// Distinguishes a genuine wall-clock timeout (`RecvTimeoutError::Timeout` -- the engine is
    /// still alive, just slow) from the stdout-forwarding channel disconnecting
    /// (`RecvTimeoutError::Disconnected` -- that reader thread only ends once the child's
    /// stdout hits EOF, which for a long-running USI engine that never explicitly closes stdout
    /// while alive reliably means the process has exited). Conflating the two into one
    /// `UsiError::Timeout` (as this used to do) is what let a dead engine's unconditional
    /// restart in `label` silently fail to trigger: it depended on a *later* call's `write_line`
    /// tripping an `EPIPE`/`Io` error, which is not reliable across platforms -- observed to
    /// fire promptly on macOS, but the disconnect can persist through several subsequent calls
    /// on Linux/Windows CI without any write ever erroring. Mapping the disconnect straight to
    /// `Io` here instead makes death detection immediate and platform-independent.
    fn recv_until(&mut self, deadline: Instant) -> Result<String, UsiError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let line = match self.rx.recv_timeout(remaining) {
            Ok(line) => line.map_err(UsiError::Io)?,
            Err(mpsc::RecvTimeoutError::Timeout) => return Err(UsiError::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(UsiError::Io(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "engine's stdout-forwarding channel disconnected (process likely exited)",
                )));
            }
        };
        if self.capture_transcript {
            self.transcript.push(format!("< {line}"));
        }
        Ok(line)
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
            } else if line.starts_with("bestmove ") {
                self.pending_handshake_bestmoves.push(line);
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
            } else if line.starts_with("bestmove ") {
                self.pending_handshake_bestmoves.push(line);
            }
        }
        self.write_line("usinewgame")?;
        Ok(())
    }

    /// Builds the final result once a `bestmove` token is in hand (`Ok`, from the main loop
    /// below or `salvage_after_timeout`'s grace period) or synthesized from a PV (the salvage
    /// fallback below). Shared so all three call sites agree on how `policy_margin_cp`/
    /// `candidates`/`score_bound` are derived from `candidates_by_rank`. Sets `timed_out: false`;
    /// callers on the salvage path flip it to `true` after the fact.
    fn build_analysis_result(
        bestmove: String,
        candidates_by_rank: &BTreeMap<u32, InfoLine>,
        depth_fallback: u32,
    ) -> Result<AnalysisResult, UsiError> {
        let info = candidates_by_rank.get(&1).ok_or(UsiError::NoBestmove)?;
        let policy_margin_cp = match (&info.score, candidates_by_rank.get(&2)) {
            (Some(Score::Cp { value: best }), Some(runner_up))
                if info.bound == ScoreBound::Exact && runner_up.bound == ScoreBound::Exact =>
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
        let bestmove_kind = classify_bestmove(&bestmove);
        Ok(AnalysisResult {
            depth: info.depth.unwrap_or(depth_fallback),
            score: info.score.clone().ok_or(UsiError::InvalidResponse)?,
            bestmove,
            nodes: info.nodes,
            time_ms: info.time_ms,
            seldepth: info.seldepth,
            nps: info.nps,
            hashfull: info.hashfull,
            pv: info.pv.clone(),
            policy_margin_cp,
            candidates,
            score_bound: info.bound,
            bestmove_kind,
            timed_out: false,
        })
    }

    /// Called when the main search deadline elapses with no `bestmove` yet. Sends `stop` (best
    /// effort -- the pipe may already be dead) and waits out a short grace period for the engine
    /// to respond, since USI engines are expected to react to `stop` promptly; if a real
    /// `bestmove` arrives within that window, this is a graceful recovery, not a degraded result.
    /// If nothing arrives even then, synthesizes a result from the last `info` line's own PV
    /// (the engine's own claimed best move at whatever depth it actually reached) instead of the
    /// bare `Err(UsiError::Timeout)` this used to return unconditionally -- returns that same
    /// error, unchanged, only when there's truly nothing to salvage (e.g. a fully unresponsive
    /// engine that never produced any output at all).
    fn salvage_after_timeout(
        &mut self,
        candidates_by_rank: &mut BTreeMap<u32, InfoLine>,
        depth_fallback: u32,
    ) -> Result<AnalysisResult, UsiError> {
        let _ = self.write_line("stop");
        let grace_deadline = Instant::now() + Duration::from_millis(STOP_GRACE_MS);
        loop {
            match self.recv_until(grace_deadline) {
                Ok(line) if line.starts_with("bestmove ") => {
                    let bestmove = line
                        .strip_prefix("bestmove ")
                        .and_then(|s| s.split_whitespace().next())
                        .ok_or(UsiError::InvalidResponse)?
                        .to_string();
                    let mut result =
                        Self::build_analysis_result(bestmove, candidates_by_rank, depth_fallback)?;
                    result.timed_out = true;
                    return Ok(result);
                }
                Ok(line) if line.starts_with("info ") => {
                    if let Some(info) = parse_info(&line) {
                        candidates_by_rank.insert(info.multipv.unwrap_or(1), info);
                    }
                }
                Ok(_) => {}
                Err(_) => break, // grace period elapsed, or the pipe died -- fall through below
            }
        }
        // Nothing arrived within the grace period, but the engine may still be about to answer
        // (it just isn't respecting `stop` promptly) -- a real bestmove for this `go` is still
        // owed. `analyse()`'s next call checks `awaiting_late_bestmove` for exactly this.
        self.awaiting_late_bestmove = true;
        let bestmove = candidates_by_rank
            .get(&1)
            .and_then(|info| info.pv.as_ref())
            .and_then(|pv| pv.first())
            .cloned()
            .ok_or(UsiError::Timeout)?;
        let mut result = Self::build_analysis_result(bestmove, candidates_by_rank, depth_fallback)?;
        result.timed_out = true;
        Ok(result)
    }

    pub fn analyse(
        &mut self,
        sfen: &str,
        limit: SearchLimit,
        timeout_ms: u64,
    ) -> Result<AnalysisResult, UsiError> {
        if self.strict && !self.pending_handshake_bestmoves.is_empty() {
            self.pending_handshake_bestmoves.clear();
            return Err(UsiError::BestmoveWithoutGo);
        }

        // Strict mode: if the previous call's timeout was salvaged by synthesizing from the last
        // `info` line, a real `bestmove` for that `go` may still be in flight. A non-blocking
        // poll below would race it and risk letting it land during THIS call's read loop instead
        // -- exactly the stale-bestmove mislabel this feature exists to prevent -- so give it one
        // bounded window first.
        if self.strict && self.awaiting_late_bestmove {
            match self.recv_until(Instant::now() + Duration::from_millis(STOP_GRACE_MS)) {
                Ok(line) if line.starts_with("bestmove ") => {
                    return Err(UsiError::DelayedBestmoveAfterTimeout);
                }
                Ok(_) | Err(_) => {} // benign stray line, or nothing arrived -- fall through
            }
        }

        // Discard anything a previous call's engine emitted after that call already returned
        // (e.g. a stray line landing just after a salvage's grace period expired) -- the same
        // engine process is reused across every position a worker thread picks up, so leftover
        // output here would otherwise be misattributed to this new position's response. In
        // strict mode, a stray `bestmove` found here is itself a protocol violation.
        while let Ok(res) = self.rx.try_recv() {
            let Ok(line) = res else { continue };
            if self.capture_transcript {
                self.transcript.push(format!("< {line}"));
            }
            if self.strict && line.starts_with("bestmove ") {
                return Err(if !self.go_ever_sent {
                    UsiError::BestmoveWithoutGo
                } else if self.awaiting_late_bestmove {
                    UsiError::DelayedBestmoveAfterTimeout
                } else {
                    UsiError::DuplicateBestmove
                });
            }
        }

        // No violation found -- starting a fresh exchange. Clear before, not after, so an error
        // path above keeps the exchange that led to it instead of discarding it.
        self.transcript.clear();
        self.write_line(&format!("position sfen {sfen}"))?;
        match limit {
            SearchLimit::Depth(d) => self.write_line(&format!("go depth {d}"))?,
            SearchLimit::Nodes(n) => self.write_line(&format!("go nodes {n}"))?,
        }
        self.go_ever_sent = true;
        self.awaiting_late_bestmove = false;
        let depth_fallback = match limit {
            SearchLimit::Depth(d) => d,
            SearchLimit::Nodes(_) => 0,
        };

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut candidates_by_rank: BTreeMap<u32, InfoLine> = BTreeMap::new();
        loop {
            match self.recv_until(deadline) {
                Ok(line) => {
                    if line.starts_with("bestmove ") {
                        let bestmove = line
                            .strip_prefix("bestmove ")
                            .and_then(|s| s.split_whitespace().next())
                            .ok_or(UsiError::InvalidResponse)?
                            .to_string();
                        return Self::build_analysis_result(
                            bestmove,
                            &candidates_by_rank,
                            depth_fallback,
                        );
                    } else if line.starts_with("info ")
                        && let Some(info) = parse_info(&line)
                    {
                        let rank = info.multipv.unwrap_or(1);
                        candidates_by_rank.insert(rank, info);
                    }
                }
                Err(UsiError::Timeout) => {
                    return self.salvage_after_timeout(&mut candidates_by_rank, depth_fallback);
                }
                Err(e) => return Err(e),
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
    fn parse_info_seldepth_nps_hashfull() {
        let info = parse_info(
            "info depth 8 seldepth 12 score cp 43 nodes 12345 nps 500000 hashfull 321 time 100 pv 7g7f",
        )
        .unwrap();
        assert_eq!(info.seldepth, Some(12));
        assert_eq!(info.nps, Some(500000));
        assert_eq!(info.hashfull, Some(321));
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

    #[test]
    fn classify_bestmove_recognizes_all_three_special_tokens() {
        assert_eq!(classify_bestmove("resign"), Some(BestMoveKind::Resign));
        assert_eq!(classify_bestmove("win"), Some(BestMoveKind::Win));
        assert_eq!(classify_bestmove("none"), Some(BestMoveKind::NoMove));
    }

    #[test]
    fn classify_bestmove_is_none_for_an_ordinary_move() {
        assert_eq!(classify_bestmove("7g7f"), None);
    }
}
