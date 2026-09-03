//! Binary pack format for shogiesa position records.
//!
//! ```text
//! Header (10 bytes):
//!   magic[8]  = b"SHOGIESA"
//!   version   = u16 le  (= 11)
//!
//! Record (variable, repeated until EOF):
//!   sfen              u16le + bytes
//!   source_kind       u8le  + bytes
//!   source_path       u16le + bytes
//!   source_ply        u32le
//!   root_id_tag       u8 (0/1)
//!   root_id           u16le + bytes  [if root_id_tag=1]
//!   variation_id_tag  u8 (0/1)
//!   variation_id      u8le  + bytes  [if variation_id_tag=1]
//!   branch_ply_tag    u8 (0/1)
//!   branch_from_ply   u32le          [if branch_ply_tag=1]
//!   phase             u8  (0=opening 1=middlegame 2=endgame)
//!   side_to_move      u8  (0=black 1=white)
//!   in_check          u8
//!   has_capture       u8
//!   stability_tag     u8  (0=absent 1=present)
//!     swing_tag       u8  (0=none 1=some)  [if stability_tag=1]
//!     swing_cp        i32le               [if swing_tag=1]
//!     agreement       u8                  [if stability_tag=1]
//!     eng_agree_tag   u8  (0=none 1=some) [if stability_tag=1]
//!     eng_agree       u8                  [if eng_agree_tag=1]
//!     eng_swing_tag   u8  (0=none 1=some) [if stability_tag=1]
//!     eng_swing_cp    i32le               [if eng_swing_tag=1]
//!   game_result_tag   u8  (0=absent 1=present)
//!     outcome         u8  (0=black_wins 1=white_wins 2=draw 3=unknown) [if game_result_tag=1]
//!     result_source   u8le + bytes                                    [if game_result_tag=1]
//!   obs_count         u16le
//!   per observation:
//!     engine          u8le  + bytes
//!     ver_tag         u8 (0/1)
//!     version         u8le  + bytes  [if ver_tag=1]
//!     depth           u32le
//!     req_depth_tag   u8 (0/1)
//!     req_depth       u32le          [if req_depth_tag=1]
//!     search_limit_kind u8 (0=depth 1=nodes)
//!     req_nodes_tag   u8 (0/1)
//!     req_nodes       u64le          [if req_nodes_tag=1]
//!     score_kind      u8 (0=cp 1=mate)
//!     score_val       i32le
//!     score_perspective u8 (0=side_to_move 1=black)
//!     score_bound     u8 (0=exact 1=lowerbound 2=upperbound)
//!     bestmove        u8le  + bytes
//!     bestmove_kind   u8 (0=none 1=resign 2=win 3=no_move)
//!     nodes_tag       u8 (0/1)
//!     nodes           u64le          [if nodes_tag=1]
//!     time_tag        u8 (0/1)
//!     time_ms         u64le          [if time_tag=1]
//!     seldepth_tag    u8 (0/1)
//!     seldepth        u32le          [if seldepth_tag=1]
//!     nps_tag         u8 (0/1)
//!     nps             u64le          [if nps_tag=1]
//!     hashfull_tag    u8 (0/1)
//!     hashfull        u32le          [if hashfull_tag=1]
//!     pv_tag          u8 (0/1)
//!     pv_count        u16le          [if pv_tag=1]
//!     pv[i]           u8le  + bytes
//!     margin_tag      u8 (0/1)
//!     policy_margin   i32le          [if margin_tag=1]
//!     candidates_count u16le
//!     per candidate:
//!       multipv        u32le
//!       bestmove       u8le  + bytes
//!       score_kind     u8 (0=cp 1=mate)
//!       score_val      i32le
//!       score_bound    u8 (0=exact 1=lowerbound 2=upperbound)
//!       pv_tag         u8 (0/1)
//!       pv_count       u16le          [if pv_tag=1]
//!       pv[i]          u8le  + bytes
//!     eng_opts_hash_tag u8 (0/1)
//!     eng_opts_hash     u8le  + bytes  [if eng_opts_hash_tag=1]
//!     weight_sha256_tag u8 (0/1)
//!     weight_sha256     u8le  + bytes  [if weight_sha256_tag=1]
//! ```

use std::io::{self, Read, Write};

use shogiesa_core::{
    BestMoveKind, CandidateMove, GameOutcome, GamePhase, GameResultInfo, Observation,
    PositionRecord, PositionTags, SCHEMA_VERSION, Score, ScoreBound, ScorePerspective,
    SearchLimitKind, SideToMove, SourceInfo, StabilityInfo,
};

pub const MAGIC: &[u8; 8] = b"SHOGIESA";
pub const FORMAT_VERSION: u16 = 11;

// ── write helpers ─────────────────────────────────────────────────────────────

fn wu8(w: &mut impl Write, v: u8) -> io::Result<()> {
    w.write_all(&[v])
}
fn wu16(w: &mut impl Write, v: u16) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn wu32(w: &mut impl Write, v: u32) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn wu64(w: &mut impl Write, v: u64) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}
fn wi32(w: &mut impl Write, v: i32) -> io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn ws8(w: &mut impl Write, s: &str) -> io::Result<()> {
    let b = s.as_bytes();
    wu8(w, b.len() as u8)?;
    w.write_all(b)
}
fn ws16(w: &mut impl Write, s: &str) -> io::Result<()> {
    let b = s.as_bytes();
    wu16(w, b.len() as u16)?;
    w.write_all(b)
}

// ── read helpers ──────────────────────────────────────────────────────────────

fn ru8(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}
fn ru16(r: &mut impl Read) -> io::Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}
fn ru32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn ru64(r: &mut impl Read) -> io::Result<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}
fn ri32(r: &mut impl Read) -> io::Result<i32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}

fn rs8(r: &mut impl Read) -> io::Result<String> {
    let len = ru8(r)? as usize;
    let mut b = vec![0u8; len];
    r.read_exact(&mut b)?;
    String::from_utf8(b).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
fn rs16(r: &mut impl Read) -> io::Result<String> {
    let len = ru16(r)? as usize;
    let mut b = vec![0u8; len];
    r.read_exact(&mut b)?;
    String::from_utf8(b).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn bad(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

// ── public API ────────────────────────────────────────────────────────────────

/// Write the 10-byte file header. Call once before `encode_record`.
pub fn write_header(w: &mut impl Write) -> io::Result<()> {
    w.write_all(MAGIC)?;
    wu16(w, FORMAT_VERSION)
}

/// Verify the file header. Call once before `decode_record`.
pub fn read_header(r: &mut impl Read) -> io::Result<()> {
    let mut magic = [0u8; 8];
    r.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(bad("bad magic"));
    }
    let v = ru16(r)?;
    if v != FORMAT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported pack version {v}"),
        ));
    }
    Ok(())
}

/// Encode one record. Precede the file with `write_header`.
pub fn encode_record(rec: &PositionRecord, w: &mut impl Write) -> io::Result<()> {
    ws16(w, &rec.sfen)?;

    ws8(w, &rec.source.kind)?;
    ws16(w, &rec.source.path)?;
    wu32(w, rec.source.ply)?;
    match &rec.source.root_id {
        None => wu8(w, 0)?,
        Some(v) => {
            wu8(w, 1)?;
            ws16(w, v)?;
        }
    }
    match &rec.source.variation_id {
        None => wu8(w, 0)?,
        Some(v) => {
            wu8(w, 1)?;
            ws8(w, v)?;
        }
    }
    match rec.source.branch_from_ply {
        None => wu8(w, 0)?,
        Some(v) => {
            wu8(w, 1)?;
            wu32(w, v)?;
        }
    }

    wu8(
        w,
        match rec.tags.phase {
            GamePhase::Opening => 0,
            GamePhase::Middlegame => 1,
            GamePhase::Endgame => 2,
        },
    )?;
    wu8(
        w,
        match rec.tags.side_to_move {
            SideToMove::Black => 0,
            SideToMove::White => 1,
        },
    )?;
    wu8(w, rec.tags.in_check as u8)?;
    wu8(w, rec.tags.has_capture as u8)?;

    match &rec.stability {
        None => wu8(w, 0)?,
        Some(s) => {
            wu8(w, 1)?;
            match s.score_swing_cp {
                None => wu8(w, 0)?,
                Some(v) => {
                    wu8(w, 1)?;
                    wi32(w, v)?;
                }
            }
            wu8(w, s.bestmove_agreement as u8)?;
            match s.engine_bestmove_agreement {
                None => wu8(w, 0)?,
                Some(v) => {
                    wu8(w, 1)?;
                    wu8(w, v as u8)?;
                }
            }
            match s.engine_score_swing_cp {
                None => wu8(w, 0)?,
                Some(v) => {
                    wu8(w, 1)?;
                    wi32(w, v)?;
                }
            }
        }
    }

    match &rec.game_result {
        None => wu8(w, 0)?,
        Some(gr) => {
            wu8(w, 1)?;
            wu8(
                w,
                match gr.outcome {
                    GameOutcome::BlackWins => 0,
                    GameOutcome::WhiteWins => 1,
                    GameOutcome::Draw => 2,
                    GameOutcome::Unknown => 3,
                },
            )?;
            ws8(w, &gr.result_source)?;
        }
    }

    wu16(w, rec.observations.len() as u16)?;
    for obs in &rec.observations {
        ws8(w, &obs.engine)?;
        match &obs.engine_version {
            None => wu8(w, 0)?,
            Some(v) => {
                wu8(w, 1)?;
                ws8(w, v)?;
            }
        }
        wu32(w, obs.depth)?;
        match obs.requested_depth {
            None => wu8(w, 0)?,
            Some(v) => {
                wu8(w, 1)?;
                wu32(w, v)?;
            }
        }
        wu8(
            w,
            match obs.search_limit_kind {
                SearchLimitKind::Depth => 0,
                SearchLimitKind::Nodes => 1,
            },
        )?;
        match obs.requested_nodes {
            None => wu8(w, 0)?,
            Some(v) => {
                wu8(w, 1)?;
                wu64(w, v)?;
            }
        }
        match obs.score {
            Score::Cp { value } => {
                wu8(w, 0)?;
                wi32(w, value)?;
            }
            Score::Mate { moves } => {
                wu8(w, 1)?;
                wi32(w, moves)?;
            }
        }
        wu8(
            w,
            match obs.score_perspective {
                ScorePerspective::SideToMove => 0,
                ScorePerspective::Black => 1,
            },
        )?;
        wu8(
            w,
            match obs.score_bound {
                ScoreBound::Exact => 0,
                ScoreBound::Lowerbound => 1,
                ScoreBound::Upperbound => 2,
            },
        )?;
        ws8(w, &obs.bestmove)?;
        wu8(
            w,
            match obs.bestmove_kind {
                None => 0,
                Some(BestMoveKind::Resign) => 1,
                Some(BestMoveKind::Win) => 2,
                Some(BestMoveKind::NoMove) => 3,
            },
        )?;
        match obs.nodes {
            None => wu8(w, 0)?,
            Some(v) => {
                wu8(w, 1)?;
                wu64(w, v)?;
            }
        }
        match obs.time_ms {
            None => wu8(w, 0)?,
            Some(v) => {
                wu8(w, 1)?;
                wu64(w, v)?;
            }
        }
        match obs.seldepth {
            None => wu8(w, 0)?,
            Some(v) => {
                wu8(w, 1)?;
                wu32(w, v)?;
            }
        }
        match obs.nps {
            None => wu8(w, 0)?,
            Some(v) => {
                wu8(w, 1)?;
                wu64(w, v)?;
            }
        }
        match obs.hashfull {
            None => wu8(w, 0)?,
            Some(v) => {
                wu8(w, 1)?;
                wu32(w, v)?;
            }
        }
        match &obs.pv {
            None => wu8(w, 0)?,
            Some(pv) => {
                wu8(w, 1)?;
                wu16(w, pv.len() as u16)?;
                for mv in pv {
                    ws8(w, mv)?;
                }
            }
        }
        match obs.policy_margin_cp {
            None => wu8(w, 0)?,
            Some(v) => {
                wu8(w, 1)?;
                wi32(w, v)?;
            }
        }
        wu16(w, obs.candidates.len() as u16)?;
        for c in &obs.candidates {
            wu32(w, c.multipv)?;
            ws8(w, &c.bestmove)?;
            match c.score {
                Score::Cp { value } => {
                    wu8(w, 0)?;
                    wi32(w, value)?;
                }
                Score::Mate { moves } => {
                    wu8(w, 1)?;
                    wi32(w, moves)?;
                }
            }
            wu8(
                w,
                match c.score_bound {
                    ScoreBound::Exact => 0,
                    ScoreBound::Lowerbound => 1,
                    ScoreBound::Upperbound => 2,
                },
            )?;
            match &c.pv {
                None => wu8(w, 0)?,
                Some(pv) => {
                    wu8(w, 1)?;
                    wu16(w, pv.len() as u16)?;
                    for mv in pv {
                        ws8(w, mv)?;
                    }
                }
            }
        }
        wu8(w, obs.was_timeout_salvaged as u8)?;
        match &obs.engine_options_hash {
            None => wu8(w, 0)?,
            Some(v) => {
                wu8(w, 1)?;
                ws8(w, v)?;
            }
        }
        match &obs.weight_sha256 {
            None => wu8(w, 0)?,
            Some(v) => {
                wu8(w, 1)?;
                ws8(w, v)?;
            }
        }
    }

    Ok(())
}

/// Decode one record. Returns `Err(UnexpectedEof)` when the stream is exhausted.
pub fn decode_record(r: &mut impl Read) -> io::Result<PositionRecord> {
    let sfen = rs16(r)?;

    let source = SourceInfo {
        kind: rs8(r)?,
        path: rs16(r)?,
        ply: ru32(r)?,
        root_id: if ru8(r)? == 0 { None } else { Some(rs16(r)?) },
        variation_id: if ru8(r)? == 0 { None } else { Some(rs8(r)?) },
        branch_from_ply: if ru8(r)? == 0 { None } else { Some(ru32(r)?) },
    };

    let phase = match ru8(r)? {
        0 => GamePhase::Opening,
        1 => GamePhase::Middlegame,
        2 => GamePhase::Endgame,
        _ => return Err(bad("bad phase")),
    };
    let side_to_move = match ru8(r)? {
        0 => SideToMove::Black,
        1 => SideToMove::White,
        _ => return Err(bad("bad side")),
    };
    let in_check = ru8(r)? != 0;
    let has_capture = ru8(r)? != 0;
    let tags = PositionTags {
        phase,
        side_to_move,
        in_check,
        has_capture,
    };

    let stability = if ru8(r)? == 0 {
        None
    } else {
        let score_swing_cp = if ru8(r)? == 0 { None } else { Some(ri32(r)?) };
        let bestmove_agreement = ru8(r)? != 0;
        let engine_bestmove_agreement = if ru8(r)? == 0 {
            None
        } else {
            Some(ru8(r)? != 0)
        };
        let engine_score_swing_cp = if ru8(r)? == 0 { None } else { Some(ri32(r)?) };
        Some(StabilityInfo {
            score_swing_cp,
            bestmove_agreement,
            engine_bestmove_agreement,
            engine_score_swing_cp,
        })
    };

    let game_result = if ru8(r)? == 0 {
        None
    } else {
        let outcome = match ru8(r)? {
            0 => GameOutcome::BlackWins,
            1 => GameOutcome::WhiteWins,
            2 => GameOutcome::Draw,
            3 => GameOutcome::Unknown,
            _ => return Err(bad("bad game outcome")),
        };
        let result_source = rs8(r)?;
        Some(GameResultInfo {
            outcome,
            result_source,
        })
    };

    let obs_count = ru16(r)? as usize;
    let mut observations = Vec::with_capacity(obs_count);
    for _ in 0..obs_count {
        let engine = rs8(r)?;
        let engine_version = if ru8(r)? == 0 { None } else { Some(rs8(r)?) };
        let depth = ru32(r)?;
        let requested_depth = if ru8(r)? == 0 { None } else { Some(ru32(r)?) };
        let search_limit_kind = match ru8(r)? {
            0 => SearchLimitKind::Depth,
            1 => SearchLimitKind::Nodes,
            _ => return Err(bad("bad search limit kind")),
        };
        let requested_nodes = if ru8(r)? == 0 { None } else { Some(ru64(r)?) };
        let score = match ru8(r)? {
            0 => Score::Cp { value: ri32(r)? },
            1 => Score::Mate { moves: ri32(r)? },
            _ => return Err(bad("bad score kind")),
        };
        let score_perspective = match ru8(r)? {
            0 => ScorePerspective::SideToMove,
            1 => ScorePerspective::Black,
            _ => return Err(bad("bad score perspective")),
        };
        let obs_score_bound = match ru8(r)? {
            0 => ScoreBound::Exact,
            1 => ScoreBound::Lowerbound,
            2 => ScoreBound::Upperbound,
            _ => return Err(bad("bad score bound")),
        };
        let bestmove = rs8(r)?;
        let bestmove_kind = match ru8(r)? {
            0 => None,
            1 => Some(BestMoveKind::Resign),
            2 => Some(BestMoveKind::Win),
            3 => Some(BestMoveKind::NoMove),
            _ => return Err(bad("bad bestmove kind")),
        };
        let nodes = if ru8(r)? == 0 { None } else { Some(ru64(r)?) };
        let time_ms = if ru8(r)? == 0 { None } else { Some(ru64(r)?) };
        let seldepth = if ru8(r)? == 0 { None } else { Some(ru32(r)?) };
        let nps = if ru8(r)? == 0 { None } else { Some(ru64(r)?) };
        let hashfull = if ru8(r)? == 0 { None } else { Some(ru32(r)?) };
        let pv = if ru8(r)? == 0 {
            None
        } else {
            let n = ru16(r)? as usize;
            let mut moves = Vec::with_capacity(n);
            for _ in 0..n {
                moves.push(rs8(r)?);
            }
            Some(moves)
        };
        let policy_margin_cp = if ru8(r)? == 0 { None } else { Some(ri32(r)?) };
        let candidate_count = ru16(r)? as usize;
        let mut candidates = Vec::with_capacity(candidate_count);
        for _ in 0..candidate_count {
            let multipv = ru32(r)?;
            let c_bestmove = rs8(r)?;
            let c_score = match ru8(r)? {
                0 => Score::Cp { value: ri32(r)? },
                1 => Score::Mate { moves: ri32(r)? },
                _ => return Err(bad("bad score kind")),
            };
            let score_bound = match ru8(r)? {
                0 => ScoreBound::Exact,
                1 => ScoreBound::Lowerbound,
                2 => ScoreBound::Upperbound,
                _ => return Err(bad("bad score bound")),
            };
            let c_pv = if ru8(r)? == 0 {
                None
            } else {
                let n = ru16(r)? as usize;
                let mut moves = Vec::with_capacity(n);
                for _ in 0..n {
                    moves.push(rs8(r)?);
                }
                Some(moves)
            };
            candidates.push(CandidateMove {
                multipv,
                bestmove: c_bestmove,
                score: c_score,
                score_bound,
                pv: c_pv,
            });
        }
        let was_timeout_salvaged = ru8(r)? != 0;
        let engine_options_hash = if ru8(r)? == 0 { None } else { Some(rs8(r)?) };
        let weight_sha256 = if ru8(r)? == 0 { None } else { Some(rs8(r)?) };
        observations.push(Observation {
            engine,
            engine_version,
            depth,
            requested_depth,
            requested_nodes,
            search_limit_kind,
            score,
            score_perspective,
            score_bound: obs_score_bound,
            bestmove,
            bestmove_kind,
            nodes,
            time_ms,
            seldepth,
            nps,
            hashfull,
            pv,
            policy_margin_cp,
            candidates,
            engine_options_hash,
            weight_sha256,
            was_timeout_salvaged,
        });
    }

    Ok(PositionRecord {
        schema_version: SCHEMA_VERSION,
        sfen,
        source,
        tags,
        observations,
        stability,
        game_result,
    })
}

/// Encode all records with a header (batch convenience).
pub fn encode(records: &[PositionRecord], w: &mut impl Write) -> io::Result<()> {
    write_header(w)?;
    for rec in records {
        encode_record(rec, w)?;
    }
    Ok(())
}

/// Read header then decode all records until EOF (batch convenience).
pub fn decode(r: &mut impl Read) -> io::Result<Vec<PositionRecord>> {
    // This convenience API is intentionally strict: buffering the input lets us distinguish a
    // clean EOF between records from an incomplete record. Streaming callers should use
    // `read_header` and `decode_record` directly and apply the same boundary check.
    let mut bytes = Vec::new();
    r.read_to_end(&mut bytes)?;
    let mut cursor = io::Cursor::new(bytes);
    read_header(&mut cursor)?;
    let mut out = Vec::new();
    while (cursor.position() as usize) < cursor.get_ref().len() {
        out.push(decode_record(&mut cursor)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PositionRecord {
        PositionRecord {
            schema_version: SCHEMA_VERSION,
            sfen: "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1".to_string(),
            source: SourceInfo {
                kind: "csa".to_string(),
                path: "test.csa".to_string(),
                ply: 3,
                root_id: Some("test.kif".to_string()),
                variation_id: Some("var1".to_string()),
                branch_from_ply: Some(5),
            },
            tags: PositionTags {
                phase: GamePhase::Opening,
                side_to_move: SideToMove::Black,
                in_check: false,
                has_capture: true,
            },
            observations: vec![
                Observation {
                    engine: "TestEngine".to_string(),
                    engine_version: Some("1.0".to_string()),
                    depth: 8,
                    requested_depth: Some(12),
                    requested_nodes: None,
                    search_limit_kind: SearchLimitKind::Depth,
                    score: Score::Cp { value: 42 },
                    score_perspective: ScorePerspective::Black,
                    score_bound: ScoreBound::Lowerbound,
                    bestmove: "7g7f".to_string(),
                    bestmove_kind: None,
                    nodes: Some(12345),
                    time_ms: Some(100),
                    seldepth: Some(14),
                    nps: Some(500_000),
                    hashfull: Some(321),
                    pv: Some(vec!["7g7f".to_string(), "3c3d".to_string()]),
                    policy_margin_cp: Some(310),
                    candidates: vec![
                        CandidateMove {
                            multipv: 1,
                            bestmove: "7g7f".to_string(),
                            score: Score::Cp { value: 42 },
                            score_bound: ScoreBound::Exact,
                            pv: Some(vec!["7g7f".to_string(), "3c3d".to_string()]),
                        },
                        CandidateMove {
                            multipv: 2,
                            bestmove: "2g2f".to_string(),
                            score: Score::Cp { value: -268 },
                            score_bound: ScoreBound::Lowerbound,
                            pv: None,
                        },
                        CandidateMove {
                            multipv: 3,
                            bestmove: "3c3d".to_string(),
                            score: Score::Cp { value: -50 },
                            score_bound: ScoreBound::Upperbound,
                            pv: None,
                        },
                    ],
                    engine_options_hash: Some("a1b2c3".to_string()),
                    weight_sha256: Some("d4e5f6".to_string()),
                    was_timeout_salvaged: true,
                },
                Observation {
                    engine: "TestEngine".to_string(),
                    engine_version: None,
                    depth: 12,
                    requested_depth: None,
                    requested_nodes: None,
                    search_limit_kind: SearchLimitKind::Depth,
                    score: Score::Mate { moves: 3 },
                    score_perspective: ScorePerspective::SideToMove,
                    score_bound: ScoreBound::Exact,
                    bestmove: "resign".to_string(),
                    bestmove_kind: Some(BestMoveKind::Resign),
                    nodes: None,
                    time_ms: None,
                    seldepth: None,
                    nps: None,
                    hashfull: None,
                    pv: None,
                    policy_margin_cp: None,
                    candidates: Vec::new(),
                    engine_options_hash: None,
                    weight_sha256: None,
                    was_timeout_salvaged: false,
                },
            ],
            stability: Some(StabilityInfo {
                score_swing_cp: Some(100),
                bestmove_agreement: false,
                engine_bestmove_agreement: Some(false),
                engine_score_swing_cp: Some(60),
            }),
            game_result: Some(GameResultInfo {
                outcome: GameOutcome::WhiteWins,
                result_source: "csa_terminal".to_string(),
            }),
        }
    }

    #[test]
    fn round_trip() {
        let original = sample();
        let mut buf = Vec::new();
        encode(std::slice::from_ref(&original), &mut buf).unwrap();

        let decoded = decode(&mut buf.as_slice()).unwrap();
        assert_eq!(decoded.len(), 1);
        let got = &decoded[0];

        assert_eq!(got.sfen, original.sfen);
        assert_eq!(got.source.kind, "csa");
        assert_eq!(got.source.ply, 3);
        assert_eq!(got.source.root_id, Some("test.kif".to_string()));
        assert_eq!(got.source.variation_id, Some("var1".to_string()));
        assert_eq!(got.source.branch_from_ply, Some(5));
        assert!(!got.tags.in_check);
        assert!(got.tags.has_capture);
        assert_eq!(got.observations.len(), 2);
        assert_eq!(got.observations[0].depth, 8);
        assert_eq!(got.observations[0].requested_depth, Some(12));
        assert!(matches!(got.observations[0].score, Score::Cp { value: 42 }));
        assert_eq!(
            got.observations[0].score_perspective,
            ScorePerspective::Black
        );
        assert_eq!(got.observations[0].score_bound, ScoreBound::Lowerbound);
        assert_eq!(got.observations[0].bestmove_kind, None);
        assert_eq!(got.observations[0].engine_version, Some("1.0".to_string()));
        assert_eq!(got.observations[0].nodes, Some(12345));
        assert_eq!(
            got.observations[0].search_limit_kind,
            SearchLimitKind::Depth
        );
        assert_eq!(got.observations[0].requested_nodes, None);
        assert_eq!(got.observations[0].seldepth, Some(14));
        assert_eq!(got.observations[0].nps, Some(500_000));
        assert_eq!(got.observations[0].hashfull, Some(321));
        assert_eq!(
            got.observations[0].engine_options_hash,
            Some("a1b2c3".to_string())
        );
        assert_eq!(
            got.observations[0].weight_sha256,
            Some("d4e5f6".to_string())
        );
        assert_eq!(
            got.observations[0].pv,
            Some(vec!["7g7f".to_string(), "3c3d".to_string()])
        );
        assert_eq!(got.observations[0].policy_margin_cp, Some(310));
        assert_eq!(got.observations[0].candidates.len(), 3);
        assert_eq!(got.observations[0].candidates[0].multipv, 1);
        assert_eq!(got.observations[0].candidates[0].bestmove, "7g7f");
        assert_eq!(
            got.observations[0].candidates[0].score_bound,
            ScoreBound::Exact
        );
        assert_eq!(
            got.observations[0].candidates[0].pv,
            Some(vec!["7g7f".to_string(), "3c3d".to_string()])
        );
        assert_eq!(got.observations[0].candidates[1].multipv, 2);
        assert_eq!(got.observations[0].candidates[1].bestmove, "2g2f");
        assert!(matches!(
            got.observations[0].candidates[1].score,
            Score::Cp { value: -268 }
        ));
        assert_eq!(
            got.observations[0].candidates[1].score_bound,
            ScoreBound::Lowerbound
        );
        assert_eq!(got.observations[0].candidates[1].pv, None);
        assert_eq!(got.observations[0].candidates[2].multipv, 3);
        assert_eq!(got.observations[0].candidates[2].bestmove, "3c3d");
        assert!(matches!(
            got.observations[0].candidates[2].score,
            Score::Cp { value: -50 }
        ));
        assert_eq!(
            got.observations[0].candidates[2].score_bound,
            ScoreBound::Upperbound
        );
        assert_eq!(got.observations[1].engine_version, None);
        assert_eq!(got.observations[1].requested_depth, None);
        assert!(matches!(
            got.observations[1].score,
            Score::Mate { moves: 3 }
        ));
        assert_eq!(got.observations[1].policy_margin_cp, None);
        assert_eq!(
            got.observations[1].score_perspective,
            ScorePerspective::SideToMove
        );
        assert_eq!(got.observations[1].score_bound, ScoreBound::Exact);
        assert_eq!(got.observations[1].bestmove, "resign");
        assert_eq!(
            got.observations[1].bestmove_kind,
            Some(BestMoveKind::Resign)
        );
        assert!(got.observations[1].candidates.is_empty());
        let stab = got.stability.as_ref().unwrap();
        assert_eq!(stab.score_swing_cp, Some(100));
        assert!(!stab.bestmove_agreement);
        assert_eq!(stab.engine_bestmove_agreement, Some(false));
        assert_eq!(stab.engine_score_swing_cp, Some(60));
        let gr = got.game_result.as_ref().unwrap();
        assert_eq!(gr.outcome, GameOutcome::WhiteWins);
        assert_eq!(gr.result_source, "csa_terminal");
    }

    #[test]
    fn stability_with_no_engine_fields_round_trips() {
        let mut rec = sample();
        rec.stability = Some(StabilityInfo {
            score_swing_cp: None,
            bestmove_agreement: true,
            engine_bestmove_agreement: None,
            engine_score_swing_cp: None,
        });
        let mut buf = Vec::new();
        encode(std::slice::from_ref(&rec), &mut buf).unwrap();
        let got = &decode(&mut buf.as_slice()).unwrap()[0];
        let stab = got.stability.as_ref().unwrap();
        assert_eq!(stab.score_swing_cp, None);
        assert!(stab.bestmove_agreement);
        assert_eq!(stab.engine_bestmove_agreement, None);
        assert_eq!(stab.engine_score_swing_cp, None);
    }

    #[test]
    fn bad_magic_rejected() {
        let buf = b"BADSIG!!\x01\x00".as_slice();
        let err = decode(&mut { buf }).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(err.to_string(), "bad magic");
    }

    #[test]
    fn header_error_classes_are_distinct() {
        let err = decode(&mut b"SHOGIESA".as_slice()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);

        let err = decode(&mut b"SHOGIESA\xff\xff".as_slice()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(err.to_string(), "unsupported pack version 65535");

        let err = decode(&mut b"SHOGIESA\x00\x0b".as_slice()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(err.to_string(), "unsupported pack version 2816");
    }

    #[test]
    fn truncated_record_is_not_treated_as_clean_eof() {
        let bytes = b"SHOGIESA\x0b\x00\x0b\x00";
        let err = decode(&mut bytes.as_slice()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn empty_pack_ok() {
        let mut buf = Vec::new();
        write_header(&mut buf).unwrap();
        let records = decode(&mut buf.as_slice()).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn no_stability_round_trips() {
        let mut rec = sample();
        rec.stability = None;
        let mut buf = Vec::new();
        encode(std::slice::from_ref(&rec), &mut buf).unwrap();
        let got = &decode(&mut buf.as_slice()).unwrap()[0];
        assert!(got.stability.is_none());
    }

    #[test]
    fn no_game_result_round_trips() {
        let mut rec = sample();
        rec.game_result = None;
        let mut buf = Vec::new();
        encode(std::slice::from_ref(&rec), &mut buf).unwrap();
        let got = &decode(&mut buf.as_slice()).unwrap()[0];
        assert!(got.game_result.is_none());
    }

    #[test]
    fn roundtrip_nodes_limited_observation() {
        // sample()'s fixture is depth-mode only -- this exercises the nodes-mode wire fields
        // (search_limit_kind, requested_nodes) which round_trip's own fixture never sets.
        let mut rec = sample();
        rec.observations[0].search_limit_kind = SearchLimitKind::Nodes;
        rec.observations[0].requested_depth = None;
        rec.observations[0].requested_nodes = Some(200_000);
        let mut buf = Vec::new();
        encode(std::slice::from_ref(&rec), &mut buf).unwrap();
        let got = &decode(&mut buf.as_slice()).unwrap()[0];
        assert_eq!(
            got.observations[0].search_limit_kind,
            SearchLimitKind::Nodes
        );
        assert_eq!(got.observations[0].requested_depth, None);
        assert_eq!(got.observations[0].requested_nodes, Some(200_000));
    }

    #[test]
    fn source_without_root_id_round_trips() {
        // A mainline (or CSA-extracted) record has no root_id/variation_id/branch_from_ply.
        let mut rec = sample();
        rec.source.root_id = None;
        rec.source.variation_id = None;
        rec.source.branch_from_ply = None;
        let mut buf = Vec::new();
        encode(std::slice::from_ref(&rec), &mut buf).unwrap();
        let got = &decode(&mut buf.as_slice()).unwrap()[0];
        assert_eq!(got.source.root_id, None);
        assert_eq!(got.source.variation_id, None);
        assert_eq!(got.source.branch_from_ply, None);
    }
}
