use std::fmt;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SideToMove {
    Black,
    White,
}

impl fmt::Display for SideToMove {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SideToMove::Black => write!(f, "black"),
            SideToMove::White => write!(f, "white"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GamePhase {
    Opening,
    Middlegame,
    Endgame,
}

impl fmt::Display for GamePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GamePhase::Opening => write!(f, "opening"),
            GamePhase::Middlegame => write!(f, "middlegame"),
            GamePhase::Endgame => write!(f, "endgame"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StabilityInfo {
    /// Max minus min of all cp-scored observations. None if fewer than 2 cp observations.
    pub score_swing_cp: Option<i32>,
    /// True when all observations agree on the same bestmove.
    pub bestmove_agreement: bool,
    /// True when every distinct engine's deepest observation agrees on bestmove.
    /// `None` if fewer than 2 distinct engines are represented.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_bestmove_agreement: Option<bool>,
    /// Cp swing across each distinct engine's deepest observation.
    /// `None` if fewer than 2 engines have a cp-scored observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_score_swing_cp: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionRecord {
    pub schema_version: u32,
    pub sfen: String,
    pub source: SourceInfo,
    pub tags: PositionTags,
    pub observations: Vec<Observation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stability: Option<StabilityInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_result: Option<GameResultInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    pub kind: String,
    pub path: String,
    pub ply: u32,
    /// Identifier shared by a game's mainline and every variation branching from it (e.g. the
    /// mainline's own `path`). `None` on records from extractors that don't produce variations
    /// (CSA) or on JSONL predating this field. Lets `split` group a mainline with its variations
    /// without depending on `path`'s `#varN@ply` suffix convention.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_id: Option<String>,
    /// This branch's identifier (e.g. `"var1"`) among its mainline's variations. `None` on the
    /// mainline itself and on records with no variation concept at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variation_id: Option<String>,
    /// The mainline ply this variation branched from. `None` on the mainline itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_from_ply: Option<u32>,
}

/// Game-level result, resolved once per game from its terminal action/marker -- independent of
/// which side is on move at any individual ply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameOutcome {
    BlackWins,
    WhiteWins,
    Draw,
    Unknown,
}

impl GameOutcome {
    /// Mover-relative label for lineprior-style export: the mover's own side compared against
    /// who won, e.g. a blunder in a game its side went on to win is still "success" -- this is a
    /// weak game-result signal, not a claim that the move itself was correct.
    pub fn for_mover(self, mover: SideToMove) -> &'static str {
        match self {
            GameOutcome::Draw => "draw",
            GameOutcome::Unknown => "unknown",
            GameOutcome::BlackWins => {
                if mover == SideToMove::Black {
                    "success"
                } else {
                    "failure"
                }
            }
            GameOutcome::WhiteWins => {
                if mover == SideToMove::White {
                    "success"
                } else {
                    "failure"
                }
            }
        }
    }
}

/// Game-level WDL provenance for `PositionRecord`, separate from `Observation`-derived signals --
/// lets a raw-vs-curated dataset comparison check whether filtering biases the dataset toward
/// wins/losses/certain phases, which nothing in the schema exposed before this. `outcome` is
/// intentionally NOT mover-relative (unlike `GameOutcome::for_mover`'s lineprior string) -- a
/// caller needing mover-relative WDL calls `outcome.for_mover(tags.side_to_move)` itself, so this
/// struct stays the single absolute source of truth instead of caching a value that could drift
/// from `tags.side_to_move`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameResultInfo {
    pub outcome: GameOutcome,
    /// Where `outcome` was resolved from: `"csa_terminal"`, `"kif_marker"`, `"match_header"`, or
    /// `"unknown"` (no terminal was resolved -- e.g. a KIF variation-branch position, matching
    /// `RawMove`'s existing convention of always carrying `GameOutcome::Unknown` there).
    pub result_source: String,
}

/// One played ply from an unfiltered full-game walk: the pre-move SFEN plus the move itself.
/// Distinct from `PositionRecord` (built by the filtered/deduped `extract_from_str` pipeline,
/// which only keeps the POST-move SFEN) -- an internal handoff type for lineprior-style export,
/// not a JSONL schema itself, so no `Serialize`.
///
/// `outcome` is resolved per-move rather than once per game so that KIF variation-branch moves
/// (whose terminal, if any, isn't the actual game's result) can carry `GameOutcome::Unknown`
/// while their mainline sibling moves carry the real result, within the same returned `Vec`.
#[derive(Debug, Clone)]
pub struct RawMove {
    pub ply: u32,
    pub mover: SideToMove,
    pub sfen_before: String,
    pub usi_move: String,
    pub source: SourceInfo,
    pub outcome: GameOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionTags {
    pub phase: GamePhase,
    pub side_to_move: SideToMove,
    pub in_check: bool,
    pub has_capture: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Score {
    Cp { value: i32 },
    Mate { moves: i32 },
}

/// Which side `Score::Cp`'s sign is relative to. USI's `info score cp` is side-to-move-relative
/// by protocol convention (positive = good for whoever is on move), and `shogiesa-usi` stores
/// that raw value with no sign-flipping -- so every `Observation` produced before this field
/// existed, and every one produced by `label` today, is implicitly `SideToMove`. This field makes
/// that convention explicit in the schema instead of leaving it an undocumented assumption that
/// every consumer has to independently get right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScorePerspective {
    #[default]
    SideToMove,
    Black,
}

/// Classifies a `bestmove` response into "no move at all" categories, distinct from an ordinary
/// move string. USI defines `resign`/`win`/`none` as literal non-move tokens; comparing them as
/// if they were moves (as plain `bestmove` string equality does) conflates "the engine considers
/// the position decided" with "the engines disagree on the best move" -- two different things
/// that `bestmove_agreement`-style checks need to tell apart. `NoMove` names the literal USI
/// token `"none"`, distinct from Rust's `Option::None` (which instead means "an ordinary move,
/// no special handling needed" on `Observation.bestmove_kind` below).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BestMoveKind {
    Resign,
    Win,
    NoMove,
}

/// Classifies a literal USI `bestmove` token, `None` for an ordinary move string. The single
/// source of truth for the resign/win/none token set -- `shogiesa-usi`'s live classification and
/// `effective_bestmove_kind`'s legacy-JSONL fallback below both call this instead of each keeping
/// their own copy of the match.
pub fn classify_bestmove_token(token: &str) -> Option<BestMoveKind> {
    match token {
        "resign" => Some(BestMoveKind::Resign),
        "win" => Some(BestMoveKind::Win),
        "none" => Some(BestMoveKind::NoMove),
        _ => None,
    }
}

/// Whether a USI `info` line's score is a confirmed evaluation or a search bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScoreBound {
    #[default]
    Exact,
    Lowerbound,
    Upperbound,
}

/// Which kind of limit `label` asked the engine to search under. `#[serde(default)]` without
/// `skip_serializing_if`, same convention as `score_perspective` -- every future observation
/// should self-describe explicitly rather than leave a reader to assume the default forever.
/// Old JSONL (pre-v11, always depth-limited) still parses unchanged: absent -> `Depth`, exactly
/// what that data always meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SearchLimitKind {
    #[default]
    Depth,
    Nodes,
}

/// One MultiPV candidate line from a `label --multipv N` (N≥2) pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateMove {
    pub multipv: u32,
    pub bestmove: String,
    pub score: Score,
    #[serde(default)]
    pub score_bound: ScoreBound,
    pub pv: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub engine: String,
    pub engine_version: Option<String>,
    pub depth: u32,
    /// The depth `label` asked the engine to search to, distinct from `depth` (what it actually
    /// reached) — an engine can stop early on a forced mate. `None` on records labeled before
    /// this field existed. Lets `require_requested_depth_reached` tell "requested 12, reached 8"
    /// apart from "requested 8, reached 8", which `min_depth_reached` alone cannot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_depth: Option<u32>,
    /// The node count `label --nodes` asked the engine to search to -- `requested_depth`'s
    /// counterpart when `search_limit_kind == Nodes`. `nodes` below (the engine's own reported
    /// count) already serves as "actual nodes" in both modes, exactly parallel to how `depth`
    /// already serves as "actual depth" opposite `requested_depth`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_nodes: Option<u64>,
    /// Which kind of limit this observation was searched under. See `requested_depth`'s doc
    /// comment for why this field, unlike this struct's other new-since-v9 fields, isn't gated
    /// by `skip_serializing_if`.
    #[serde(default)]
    pub search_limit_kind: SearchLimitKind,
    pub score: Score,
    /// Which side `score`'s cp sign is relative to. `#[serde(default)]` without
    /// `skip_serializing_if`, unlike this struct's other optional fields -- every future
    /// observation should self-describe its perspective explicitly rather than leave a reader to
    /// assume the default forever. Old JSONL still parses unchanged (absent → `SideToMove`,
    /// exactly what that data always meant).
    #[serde(default)]
    pub score_perspective: ScorePerspective,
    /// Whether `score` is a confirmed evaluation or a search bound (e.g. an aspiration-window
    /// fail-high/low). Only ever set from the engine's own bestmove line, independent of
    /// whether MultiPV was used — `CandidateMove.score_bound` covers runner-up ranks.
    #[serde(default)]
    pub score_bound: ScoreBound,
    pub bestmove: String,
    /// Set only when `bestmove` is a special USI token (`resign`/`win`/`none`) rather than an
    /// ordinary move -- `None` for the common case, matching this struct's convention for
    /// `policy_margin_cp`/`candidates`. `None` on records labeled before this field existed, even
    /// if `bestmove` happens to hold one of those literal strings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bestmove_kind: Option<BestMoveKind>,
    pub nodes: Option<u64>,
    pub time_ms: Option<u64>,
    /// The engine's own `info seldepth` at the bestmove line -- how far a selective search
    /// extended past `depth`. `None` when the engine didn't report it or this observation
    /// predates the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seldepth: Option<u32>,
    /// The engine's own `info nps` at the bestmove line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nps: Option<u64>,
    /// The engine's own `info hashfull` (per-mille) at the bestmove line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hashfull: Option<u32>,
    pub pv: Option<Vec<String>>,
    /// `score_cp(bestmove) - score_cp(runner_up)` from a MultiPV≥2 label pass.
    /// `None` when MultiPV wasn't used, either score was a mate score, or the
    /// runner-up's score was a lowerbound/upperbound rather than a confirmed eval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_margin_cp: Option<i32>,
    /// Every MultiPV rank from the search, populated only when the engine was run with
    /// MultiPV≥2 (empty otherwise, matching `policy_margin_cp`'s convention).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<CandidateMove>,
    /// Full blake3 hex digest of the sorted `--engine-option` pairs this observation was
    /// produced under (including the CLI-synthesized `MultiPV` entry, if any) -- distinct from
    /// the internal truncated `u64` `label`'s on-disk cache key uses, which never leaves that
    /// process. This hex form is for the JSONL artifact itself, which does leave it (shogiesa ->
    /// quietset -> lineprior -> veridict); a truncated `u64` exceeds 2^53 almost always and a
    /// JS/TS consumer's `JSON.parse` would silently mangle it as an IEEE-754 double.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine_options_hash: Option<String>,
    /// SHA-256 hex digest of the file passed to `label --weight-file`, if any -- identifies the
    /// NNUE/eval weight file the engine was configured to use, distinct from `engine_options_hash`
    /// (which only covers the *option string*, e.g. an `EvalFile` path -- a retrained net written
    /// over the same path is invisible to it). SHA-256, not blake3, so sibling tools can verify
    /// the exact bytes without sharing shogiesa's manifest schema. See
    /// `RunManifest.weight_sha256` in `shogiesa-cli` for the run-level counterpart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight_sha256: Option<String>,
    /// `true` when `--timeout-ms` elapsed before `bestmove` arrived and this observation is the
    /// degraded-but-real result salvaged from the last `info` line rather than a normal
    /// completion or the engine's own early stop (e.g. a forced mate). `false` on records
    /// labeled before this field existed -- a genuine no-op on all pre-v9 data, since the new
    /// `exclude_timeout_salvaged` gate only ever fires on data re-labeled with this field set.
    #[serde(default)]
    pub was_timeout_salvaged: bool,
}
