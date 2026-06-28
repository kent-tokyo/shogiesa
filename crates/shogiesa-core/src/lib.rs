use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionRecord {
    pub schema_version: u32,
    pub sfen: String,
    pub source: SourceInfo,
    pub tags: PositionTags,
    pub observations: Vec<Observation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    pub kind: String,
    pub path: String,
    pub ply: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionTags {
    pub phase: String,
    pub side_to_move: String,
    pub in_check: bool,
    pub has_capture: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub engine: String,
    pub engine_version: Option<String>,
    pub depth: u32,
    pub score_cp: i32,
    pub bestmove: String,
    pub nodes: Option<u64>,
}

impl PositionRecord {
    pub fn new(sfen: String, source: SourceInfo, tags: PositionTags) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            sfen,
            source,
            tags,
            observations: Vec::new(),
        }
    }
}

pub fn phase_from_ply(ply: u32) -> &'static str {
    match ply {
        0..=20 => "opening",
        21..=100 => "middlegame",
        _ => "endgame",
    }
}
