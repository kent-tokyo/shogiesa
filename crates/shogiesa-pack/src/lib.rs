//! Binary pack format for shogiesa position records.
//!
//! ```text
//! Header (10 bytes):
//!   magic[8]  = b"SHOGIESA"
//!   version   = u16 le  (= 1)
//!
//! Record (variable, repeated until EOF):
//!   sfen            u16le + bytes
//!   source_kind     u8le  + bytes
//!   source_path     u16le + bytes
//!   source_ply      u32le
//!   phase           u8  (0=opening 1=middlegame 2=endgame)
//!   side_to_move    u8  (0=black 1=white)
//!   in_check        u8
//!   has_capture     u8
//!   stability_tag   u8  (0=absent 1=present)
//!     swing_tag     u8  (0=none 1=some)  [if stability_tag=1]
//!     swing_cp      i32le               [if swing_tag=1]
//!     agreement     u8                  [if stability_tag=1]
//!   obs_count       u16le
//!   per observation:
//!     engine        u8le  + bytes
//!     ver_tag       u8 (0/1)
//!     version       u8le  + bytes  [if ver_tag=1]
//!     depth         u32le
//!     score_kind    u8 (0=cp 1=mate)
//!     score_val     i32le
//!     bestmove      u8le  + bytes
//!     nodes_tag     u8 (0/1)
//!     nodes         u64le          [if nodes_tag=1]
//!     time_tag      u8 (0/1)
//!     time_ms       u64le          [if time_tag=1]
//!     pv_tag        u8 (0/1)
//!     pv_count      u16le          [if pv_tag=1]
//!     pv[i]         u8le  + bytes
//! ```

use std::io::{self, Read, Write};

use shogiesa_core::{
    GamePhase, Observation, PositionRecord, PositionTags, Score, SideToMove, SourceInfo,
    StabilityInfo, SCHEMA_VERSION,
};

pub const MAGIC: &[u8; 8] = b"SHOGIESA";
pub const FORMAT_VERSION: u16 = 1;

// ── write helpers ─────────────────────────────────────────────────────────────

fn wu8(w: &mut impl Write, v: u8) -> io::Result<()> { w.write_all(&[v]) }
fn wu16(w: &mut impl Write, v: u16) -> io::Result<()> { w.write_all(&v.to_le_bytes()) }
fn wu32(w: &mut impl Write, v: u32) -> io::Result<()> { w.write_all(&v.to_le_bytes()) }
fn wu64(w: &mut impl Write, v: u64) -> io::Result<()> { w.write_all(&v.to_le_bytes()) }
fn wi32(w: &mut impl Write, v: i32) -> io::Result<()> { w.write_all(&v.to_le_bytes()) }

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
    let mut b = [0u8; 1]; r.read_exact(&mut b)?; Ok(b[0])
}
fn ru16(r: &mut impl Read) -> io::Result<u16> {
    let mut b = [0u8; 2]; r.read_exact(&mut b)?; Ok(u16::from_le_bytes(b))
}
fn ru32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0u8; 4]; r.read_exact(&mut b)?; Ok(u32::from_le_bytes(b))
}
fn ru64(r: &mut impl Read) -> io::Result<u64> {
    let mut b = [0u8; 8]; r.read_exact(&mut b)?; Ok(u64::from_le_bytes(b))
}
fn ri32(r: &mut impl Read) -> io::Result<i32> {
    let mut b = [0u8; 4]; r.read_exact(&mut b)?; Ok(i32::from_le_bytes(b))
}

fn rs8(r: &mut impl Read) -> io::Result<String> {
    let len = ru8(r)? as usize;
    let mut b = vec![0u8; len]; r.read_exact(&mut b)?;
    String::from_utf8(b).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
fn rs16(r: &mut impl Read) -> io::Result<String> {
    let len = ru16(r)? as usize;
    let mut b = vec![0u8; len]; r.read_exact(&mut b)?;
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

    wu8(w, match rec.tags.phase {
        GamePhase::Opening    => 0,
        GamePhase::Middlegame => 1,
        GamePhase::Endgame    => 2,
    })?;
    wu8(w, match rec.tags.side_to_move {
        SideToMove::Black => 0,
        SideToMove::White => 1,
    })?;
    wu8(w, rec.tags.in_check as u8)?;
    wu8(w, rec.tags.has_capture as u8)?;

    match &rec.stability {
        None => wu8(w, 0)?,
        Some(s) => {
            wu8(w, 1)?;
            match s.score_swing_cp {
                None    => wu8(w, 0)?,
                Some(v) => { wu8(w, 1)?; wi32(w, v)?; }
            }
            wu8(w, s.bestmove_agreement as u8)?;
        }
    }

    wu16(w, rec.observations.len() as u16)?;
    for obs in &rec.observations {
        ws8(w, &obs.engine)?;
        match &obs.engine_version {
            None    => wu8(w, 0)?,
            Some(v) => { wu8(w, 1)?; ws8(w, v)?; }
        }
        wu32(w, obs.depth)?;
        match obs.score {
            Score::Cp   { value } => { wu8(w, 0)?; wi32(w, value)?; }
            Score::Mate { moves } => { wu8(w, 1)?; wi32(w, moves)?; }
        }
        ws8(w, &obs.bestmove)?;
        match obs.nodes {
            None    => wu8(w, 0)?,
            Some(v) => { wu8(w, 1)?; wu64(w, v)?; }
        }
        match obs.time_ms {
            None    => wu8(w, 0)?,
            Some(v) => { wu8(w, 1)?; wu64(w, v)?; }
        }
        match &obs.pv {
            None => wu8(w, 0)?,
            Some(pv) => {
                wu8(w, 1)?;
                wu16(w, pv.len() as u16)?;
                for mv in pv { ws8(w, mv)?; }
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
        ply:  ru32(r)?,
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
    let in_check    = ru8(r)? != 0;
    let has_capture = ru8(r)? != 0;
    let tags = PositionTags { phase, side_to_move, in_check, has_capture };

    let stability = if ru8(r)? == 0 {
        None
    } else {
        let score_swing_cp = if ru8(r)? == 0 { None } else { Some(ri32(r)?) };
        let bestmove_agreement = ru8(r)? != 0;
        Some(StabilityInfo { score_swing_cp, bestmove_agreement })
    };

    let obs_count = ru16(r)? as usize;
    let mut observations = Vec::with_capacity(obs_count);
    for _ in 0..obs_count {
        let engine         = rs8(r)?;
        let engine_version = if ru8(r)? == 0 { None } else { Some(rs8(r)?) };
        let depth          = ru32(r)?;
        let score = match ru8(r)? {
            0 => Score::Cp   { value: ri32(r)? },
            1 => Score::Mate { moves: ri32(r)? },
            _ => return Err(bad("bad score kind")),
        };
        let bestmove = rs8(r)?;
        let nodes    = if ru8(r)? == 0 { None } else { Some(ru64(r)?) };
        let time_ms  = if ru8(r)? == 0 { None } else { Some(ru64(r)?) };
        let pv = if ru8(r)? == 0 {
            None
        } else {
            let n = ru16(r)? as usize;
            let mut moves = Vec::with_capacity(n);
            for _ in 0..n { moves.push(rs8(r)?); }
            Some(moves)
        };
        observations.push(Observation {
            engine, engine_version, depth, score, bestmove, nodes, time_ms, pv,
        });
    }

    Ok(PositionRecord {
        schema_version: SCHEMA_VERSION,
        sfen, source, tags, observations, stability,
    })
}

/// Encode all records with a header (batch convenience).
pub fn encode(records: &[PositionRecord], w: &mut impl Write) -> io::Result<()> {
    write_header(w)?;
    for rec in records { encode_record(rec, w)?; }
    Ok(())
}

/// Read header then decode all records until EOF (batch convenience).
pub fn decode(r: &mut impl Read) -> io::Result<Vec<PositionRecord>> {
    read_header(r)?;
    let mut out = Vec::new();
    loop {
        match decode_record(r) {
            Ok(rec) => out.push(rec),
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
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
                    score: Score::Cp { value: 42 },
                    bestmove: "7g7f".to_string(),
                    nodes: Some(12345),
                    time_ms: Some(100),
                    pv: Some(vec!["7g7f".to_string(), "3c3d".to_string()]),
                },
                Observation {
                    engine: "TestEngine".to_string(),
                    engine_version: None,
                    depth: 12,
                    score: Score::Mate { moves: 3 },
                    bestmove: "2b3c".to_string(),
                    nodes: None,
                    time_ms: None,
                    pv: None,
                },
            ],
            stability: Some(StabilityInfo {
                score_swing_cp: Some(100),
                bestmove_agreement: false,
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
        assert!(!got.tags.in_check);
        assert!(got.tags.has_capture);
        assert_eq!(got.observations.len(), 2);
        assert_eq!(got.observations[0].depth, 8);
        assert!(matches!(got.observations[0].score, Score::Cp { value: 42 }));
        assert_eq!(got.observations[0].engine_version, Some("1.0".to_string()));
        assert_eq!(got.observations[0].nodes, Some(12345));
        assert_eq!(got.observations[0].pv, Some(vec!["7g7f".to_string(), "3c3d".to_string()]));
        assert_eq!(got.observations[1].engine_version, None);
        assert!(matches!(got.observations[1].score, Score::Mate { moves: 3 }));
        let stab = got.stability.as_ref().unwrap();
        assert_eq!(stab.score_swing_cp, Some(100));
        assert!(!stab.bestmove_agreement);
    }

    #[test]
    fn bad_magic_rejected() {
        let buf = b"BADSIG!!\x01\x00".as_slice();
        assert!(decode(&mut { buf }).is_err());
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
}
