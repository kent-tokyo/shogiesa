use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet};
use std::fmt::Write as _; // writeln! into a String, for cmd_tune's Markdown report
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use shogiesa_core::{
    Board, GamePhase, Observation, PositionRecord, PositionTags, QualityConfig, QualityDecision,
    SCHEMA_VERSION, Score, ScorePerspective, SideToMove, SourceInfo, UsiMove, bestmove_agreement,
    cp_from_black_perspective, effective_bestmove_kind, engine_bestmove_agreement,
    evaluate_quality, has_special_bestmove, parse_usi_move, phase_from_ply,
    requested_depth_underreached, score_swing, sfen::Sfen, zobrist_from_sfen,
};
use shogiesa_pack as pack;
use shogiesa_usi::UsiEngine;
use tracing::info;

#[derive(Parser)]
#[command(
    name = "shogiesa",
    version,
    about = "Shogi training data feed for NNUE engines."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Extract positions from CSA game records
    Extract(ExtractArgs),
    /// Label positions with engine evaluations
    Label(LabelArgs),
    /// Compute stability scores and attach them to each position record
    Stability(StabilityArgs),
    /// Pack positions JSONL into binary format
    Pack(PackArgs),
    /// Unpack binary format back to JSONL
    Unpack(UnpackArgs),
    /// Split positions JSONL by source game file
    Split(SplitArgs),
    /// Sample N positions from a dataset
    Sample(SampleArgs),
    /// Mine hard positions (blunders, losing positions) from labeled data
    Mine(MineArgs),
    /// Balance dataset distribution by phase / side / eval-bucket
    Balance(BalanceArgs),
    /// Select positions worth a closer look (re-labeling candidates), instead of re-labeling
    /// everything at higher depth
    Select(SelectArgs),
    /// Filter labeled positions by stability criteria
    Filter(FilterArgs),
    /// Sweep quality-gate thresholds and report coverage/drop-reasons/distributions per value,
    /// to calibrate a filter config against a specific dataset/engine instead of guessing
    Calibrate(CalibrateArgs),
    /// Compare shallow ("student") observations against a deep ("teacher") observation from the
    /// same engine, within an already-labeled file
    Audit(AuditArgs),
    /// Grid-sweep quality-gate thresholds AND compare against a teacher depth in one pass,
    /// reporting the coverage/reliability trade-off (Pareto frontier) instead of measuring each
    /// separately -- a strict superset of `calibrate`/`audit` combined
    Tune(TuneArgs),
    /// Report statistics about a positions dataset
    Report(ReportArgs),
    /// Validate data integrity of a positions dataset
    Validate(ValidateArgs),
    /// Inspect/maintain a `label --cache-dir` cache
    Cache(CacheArgs),
    /// Extract positions from a Sekirei match-runner's per-game kifu .txt files. A pure
    /// extractor -- does not label. Feed the output through the existing `label`/`select`/
    /// `filter` commands, same as any other extracted dataset.
    FromMatch(FromMatchArgs),
    /// Merge two labeled positions JSONL files' observations (e.g. a shallow pass and a deeper
    /// relabel pass), with configurable duplicate-observation resolution
    MergeObservations(MergeObservationsArgs),
}

#[derive(clap::Args)]
struct ExtractArgs {
    /// Input file or directory of CSA files
    #[arg(short, long)]
    input: PathBuf,
    /// Output JSONL file
    #[arg(short, long)]
    out: PathBuf,
    /// Minimum ply to extract (1 = after first move)
    #[arg(long, default_value = "1")]
    min_ply: u32,
    /// Maximum ply to extract
    #[arg(long)]
    max_ply: Option<u32>,
    /// Extract every N plies
    #[arg(long, default_value = "1", name = "every-n-plies")]
    every_n_plies: u32,
    /// Deduplicate positions by SFEN string
    #[arg(long)]
    dedup: bool,
    /// Deduplicate using Zobrist hash (faster/less memory; ~1/2^64 collision chance)
    #[arg(long)]
    dedup_zobrist: bool,
}

#[derive(clap::Args)]
struct FromMatchArgs {
    /// A per-game kifu .txt file, or a directory of them (e.g. a Sekirei match-runner's
    /// `--output <dir>` kifu directory)
    #[arg(short, long)]
    input: PathBuf,
    /// Output JSONL file
    #[arg(short, long)]
    out: PathBuf,
    /// Only extract from games where this literal kifu-file label lost, per its own
    /// "# Result: Engine1 Win"/"Engine2 Win" line -- not an inferred candidate/baseline mapping
    /// (match-runner doesn't guarantee which is which). Omit to extract from every game
    /// regardless of result.
    #[arg(long, value_parser = ["engine1", "engine2"])]
    losing_side: Option<String>,
    /// Minimum ply to extract (1 = after first move)
    #[arg(long, default_value = "1")]
    min_ply: u32,
    /// Maximum ply to extract
    #[arg(long)]
    max_ply: Option<u32>,
    /// Extract every N plies
    #[arg(long, default_value = "1", name = "every-n-plies")]
    every_n_plies: u32,
    /// Deduplicate positions by SFEN string
    #[arg(long)]
    dedup: bool,
}

#[derive(clap::Args)]
struct MergeObservationsArgs {
    /// First labeled positions JSONL (e.g. a shallow labeling pass) -- wins ties under
    /// --on-collision prefer-primary
    #[arg(long)]
    primary: PathBuf,
    /// Second labeled positions JSONL (e.g. a deeper relabel pass) -- wins ties under
    /// --on-collision prefer-secondary
    #[arg(long)]
    secondary: PathBuf,
    /// Output JSONL file
    #[arg(short, long)]
    out: PathBuf,
    /// What happens when both files have an observation with the same (engine, engine_version,
    /// depth, requested_depth): keep-both (default -- no data loss, matches `label`'s own
    /// ExistingPolicy::Append-is-default convention), prefer-primary, or prefer-secondary
    #[arg(
        long,
        default_value = "keep-both",
        value_parser = ["keep-both", "prefer-primary", "prefer-secondary"]
    )]
    on_collision: String,
}

#[derive(clap::Args)]
struct ReportArgs {
    /// Input JSONL file
    #[arg(short, long)]
    input: PathBuf,
}

#[derive(clap::Args)]
struct ValidateArgs {
    /// Input JSONL file
    #[arg(short, long)]
    input: PathBuf,
    /// Exit 1 on any warning (for CI)
    #[arg(long)]
    strict: bool,
}

#[derive(clap::Args)]
struct LabelArgs {
    /// Input positions JSONL
    #[arg(short, long)]
    input: PathBuf,
    /// USI engine binary
    #[arg(long)]
    engine: PathBuf,
    /// Engine name (defaults to USI id name)
    #[arg(long)]
    engine_name: Option<String>,
    /// Comma-separated search depths, e.g. "4,6,8"
    #[arg(long)]
    depths: String,
    /// Per-depth timeout in milliseconds
    #[arg(long, default_value = "10000")]
    timeout_ms: u64,
    /// Number of parallel engine processes (1 = sequential)
    #[arg(long, default_value = "1")]
    jobs: usize,
    /// USI engine option in Key=Value format; can be repeated
    #[arg(long = "engine-option", value_name = "KEY=VALUE")]
    engine_options: Vec<String>,
    /// Number of PV lines the engine should report (sends `setoption name MultiPV`);
    /// 2+ populates each observation's policy_margin_cp
    #[arg(long, default_value = "1")]
    multipv: u32,
    /// Skip depths already covered (to at least this depth) by an observation from this engine
    #[arg(long, conflicts_with = "replace_existing")]
    skip_existing: bool,
    /// Replace an existing observation from this engine at the same (achieved) depth instead
    /// of appending a duplicate
    #[arg(long)]
    replace_existing: bool,
    /// Output JSONL file
    #[arg(short, long)]
    out: PathBuf,
    /// Write a run manifest (git sha, input hash, counts, engine/depth config, coverage stats)
    /// to this path
    #[arg(long)]
    manifest: Option<PathBuf>,
    /// Write results in strict input order (the pre-existing default before this flag existed).
    /// Trades interrupt-safety for order: a slow-to-label position holds every already-finished
    /// position behind it in memory, unwritten, until it catches up -- killing `label` (Ctrl-C,
    /// SIGTERM, SIGKILL; no signal handler exists here) loses all of that already-completed work.
    /// Omit this flag unless you specifically need output order to match input order (e.g. for
    /// diffing against a prior run).
    #[arg(long)]
    preserve_order: bool,
    /// Cache observations under this directory, keyed by (sfen, engine, engine version, engine
    /// options, requested depth, multipv, schema version) — repeated experiments over the same
    /// positions reuse a cached observation instead of re-running the engine
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    /// How the engine binary itself contributes to the label cache key, on top of its USI id
    /// name/version (which aren't guaranteed to change after a local rebuild): `content` hashes
    /// the binary's bytes (read once at startup); `metadata` hashes its path/size/mtime (cheaper,
    /// but invalidates on every rebuild into a fresh path even if the bytes are identical);
    /// `none` relies solely on the USI id strings (today's original behavior). No effect without
    /// `--cache-dir`.
    #[arg(
        long,
        default_value = "content",
        value_parser = ["content", "metadata", "none"]
    )]
    engine_fingerprint_mode: String,
}

#[derive(Clone, Copy)]
enum ExistingPolicy {
    Append,
    Skip,
    Replace,
}

#[derive(clap::Args)]
struct StabilityArgs {
    /// Input labeled positions JSONL
    #[arg(short, long)]
    input: PathBuf,
    /// Output JSONL file with stability field populated
    #[arg(short, long)]
    out: PathBuf,
}

#[derive(clap::Args)]
struct SplitArgs {
    /// Input positions JSONL
    #[arg(short, long)]
    input: PathBuf,
    /// Output directory (one file per source game)
    #[arg(long)]
    by_source: bool,
    #[arg(long = "out-dir")]
    out_dir: Option<PathBuf>,
    /// (--by-source) Maximum number of per-source output files held open at once. A source
    /// beyond this limit reuses the least-recently-written file handle, closing (and later
    /// reopening, in append mode) whichever source wrote longest ago -- keeps FD usage bounded on
    /// corpora with many more distinct source games than a process's FD limit.
    #[arg(long, default_value = "256")]
    max_open_writers: usize,
    /// Train-split output JSONL — enables the train/valid/test ratio-split mode
    /// (requires --valid and --test too)
    #[arg(long)]
    train: Option<PathBuf>,
    /// Valid-split output JSONL
    #[arg(long)]
    valid: Option<PathBuf>,
    /// Test-split output JSONL
    #[arg(long)]
    test: Option<PathBuf>,
    /// Fraction of source games assigned to the valid split
    #[arg(long, default_value = "0.1")]
    valid_frac: f64,
    /// Fraction of source games assigned to the test split
    #[arg(long, default_value = "0.1")]
    test_frac: f64,
    /// Seed for deterministic source-to-split assignment
    #[arg(long, default_value = "0")]
    seed: u64,
}

#[derive(clap::Args)]
struct SampleArgs {
    /// Input positions JSONL
    #[arg(short, long)]
    input: PathBuf,
    /// Output JSONL file
    #[arg(short, long)]
    out: PathBuf,
    /// Number of positions to sample
    #[arg(long)]
    count: usize,
    /// Seed for deterministic sampling (default 0)
    #[arg(long, default_value = "0")]
    seed: u64,
    /// Write a run manifest (git sha, input hash, counts, coverage stats) to this path
    #[arg(long)]
    manifest: Option<PathBuf>,
}

#[derive(clap::Args)]
struct MineArgs {
    /// Input labeled positions JSONL
    #[arg(short, long)]
    input: PathBuf,
    /// Output JSONL file
    #[arg(short, long)]
    out: PathBuf,
    /// Eval swing (cp, black's perspective) between consecutive plies to count as a blunder
    #[arg(long, default_value = "200")]
    blunder_threshold: i32,
    /// Include positions within N plies of a blunder (0 = blunder ply only)
    #[arg(long, default_value = "1")]
    blunder_window: usize,
    /// Include positions where the eval for the side to move is worse than -N cp
    #[arg(long)]
    losing_threshold: Option<i32>,
}

#[derive(clap::Args)]
struct BalanceArgs {
    /// Input positions JSONL
    #[arg(short, long)]
    input: PathBuf,
    /// Output JSONL file
    #[arg(short, long)]
    out: PathBuf,
    /// Dimension(s) to balance by: phase, side, eval-bucket (repeatable)
    #[arg(long = "by", value_name = "DIMENSION")]
    by: Vec<String>,
    /// Target per bucket; defaults to the smallest bucket's count
    #[arg(long)]
    target: Option<usize>,
    /// Write a run manifest (git sha, input hash, counts, coverage stats) to this path
    #[arg(long)]
    manifest: Option<PathBuf>,
}

#[derive(clap::Args)]
struct SelectArgs {
    /// Input labeled positions JSONL
    #[arg(short, long)]
    input: PathBuf,
    /// Which positions are worth a closer look: `uncertain` (weak/missing label signals),
    /// `hard` (large eval swings, bestmove disagreement, blunder-adjacent), or `coverage`
    /// (thin phase/side/eval-bucket combinations)
    #[arg(long, value_parser = ["uncertain", "hard", "coverage"])]
    strategy: String,
    /// Number of positions to select
    #[arg(long)]
    count: usize,
    /// Seed for deterministic tie-breaking (default 0)
    #[arg(long, default_value = "0")]
    seed: u64,
    /// Output JSONL file
    #[arg(short, long)]
    out: PathBuf,
    /// (uncertain strategy) also require a minimum policy_margin_cp, like `filter
    /// --min-policy-margin-cp`
    #[arg(long, allow_hyphen_values = true)]
    min_policy_margin_cp: Option<i32>,
    /// (hard strategy) eval swing (cp, black's perspective) between consecutive plies to count
    /// as a blunder
    #[arg(long, default_value = "200")]
    blunder_threshold: i32,
    /// (hard strategy) include positions within N plies of a blunder
    #[arg(long, default_value = "1")]
    blunder_window: usize,
}

#[derive(clap::Args)]
struct PackArgs {
    /// Input positions JSONL
    #[arg(short, long)]
    input: PathBuf,
    /// Output binary pack file (.shgpk)
    #[arg(short, long)]
    out: PathBuf,
    /// Write a run manifest (git sha, input hash, counts, coverage stats) to this path
    #[arg(long)]
    manifest: Option<PathBuf>,
}

#[derive(clap::Args)]
struct UnpackArgs {
    /// Input binary pack file (.shgpk)
    #[arg(short, long)]
    input: PathBuf,
    /// Output positions JSONL
    #[arg(short, long)]
    out: PathBuf,
}

#[derive(clap::Args)]
struct FilterArgs {
    /// Input labeled positions JSONL
    #[arg(short, long)]
    input: PathBuf,
    /// Output JSONL file. Not required with --dry-run.
    #[arg(short, long, required_unless_present = "dry_run")]
    out: Option<PathBuf>,
    /// Report what would be kept/dropped (and why) without writing --out. Combine with
    /// --manifest to get a structured preview of a filter config's effect.
    #[arg(long)]
    dry_run: bool,
    /// Require all observations to agree on bestmove, excluding resign/win/none tokens from the
    /// comparison
    #[arg(long)]
    require_bestmove_agreement: bool,
    /// Exclude positions where any observation has a mate score
    #[arg(long)]
    exclude_mate: bool,
    /// Exclude positions where the side to move is in check
    #[arg(long)]
    exclude_in_check: bool,
    /// Exclude positions reached by an immediate capture
    #[arg(long)]
    exclude_capture: bool,
    /// Maximum allowed cp swing across observations (abs(max_cp - min_cp))
    #[arg(long)]
    max_score_swing_cp: Option<i32>,
    /// Minimum cp score, from Black's perspective (positive = good for Black) regardless of
    /// whose turn it was — positions below it are excluded (e.g. --eval-min=-1200)
    #[arg(long, allow_hyphen_values = true)]
    eval_min: Option<i32>,
    /// Maximum cp score, from Black's perspective (positive = good for Black) regardless of
    /// whose turn it was — positions above it are excluded (e.g. --eval-max=1200)
    #[arg(long, allow_hyphen_values = true)]
    eval_max: Option<i32>,
    /// Minimum number of observations required (default: 1)
    #[arg(long, default_value = "1")]
    min_observations: u32,
    /// Filter by game phase: comma-separated (opening,middlegame,endgame)
    #[arg(long)]
    phase: Option<String>,
    /// Minimum policy_margin_cp (best move vs. runner-up from a MultiPV label pass) —
    /// positions with a smaller margin are excluded. Observations without a computed
    /// margin never trigger this gate.
    #[arg(long, allow_hyphen_values = true)]
    min_policy_margin_cp: Option<i32>,
    /// Exclude positions where any observation's score is a search bound (lowerbound/
    /// upperbound) rather than a confirmed evaluation
    #[arg(long)]
    require_exact_score: bool,
    /// Exclude positions where no observation has a computed policy_margin_cp at all.
    /// Unlike --min-policy-margin-cp (a no-op when every margin is unset), this requires a
    /// margin to have been computed in the first place.
    #[arg(long)]
    require_policy_margin: bool,
    /// Exclude positions where any non-mate observation's achieved depth is below this.
    /// Mate observations are exempt: an engine stopping short of the requested depth is
    /// dominantly caused by finding a forced mate, a confirmed result, not a weak search.
    #[arg(long)]
    min_depth_reached: Option<u32>,
    /// Exclude positions where any non-mate observation's achieved depth fell short of its own
    /// requested_depth (from `label`). Unlike --min-depth-reached (a fixed floor), this checks
    /// each observation against the depth it was itself asked to reach. A no-op on observations
    /// with no recorded requested_depth (e.g. labeled before this field existed).
    #[arg(long)]
    require_requested_depth_reached: bool,
    /// Require every distinct engine's deepest observation to agree on bestmove. A no-op
    /// unless the position was labeled by 2+ engines (see `label --engine-name`).
    #[arg(long)]
    require_engine_agreement: bool,
    /// Maximum allowed cp swing across engines' deepest observations (like
    /// --max-score-swing-cp, but grouped by engine first). A no-op with fewer than 2 engines.
    #[arg(long)]
    max_engine_score_swing_cp: Option<i32>,
    /// Write a run manifest (git sha, input hash, counts, drop reasons, filter config,
    /// coverage stats) to this path
    #[arg(long)]
    manifest: Option<PathBuf>,
    /// Write rejected records to this JSONL path, each line `{"record": ..., "quality": ...}`
    /// pairing the dropped record with its full QualityDecision (all failing reasons, not just
    /// the first). Combine with --dry-run to inspect what a config would drop without writing
    /// --out.
    #[arg(long)]
    explain_out: Option<PathBuf>,
    /// Load this run's QualityConfig from a `tune --preset-out` JSON file instead of building one
    /// from the flags below (FILE.json:label, e.g. "tuning.json:balanced"). Supplies the entire
    /// resolved config, not a partial override -- conflicts with every individual gate flag so
    /// precedence is never ambiguous.
    #[arg(long, conflicts_with_all = [
        "min_observations", "phase", "exclude_mate", "exclude_in_check", "exclude_capture",
        "max_score_swing_cp", "eval_min", "eval_max", "min_policy_margin_cp",
        "require_exact_score", "require_policy_margin", "min_depth_reached",
        "require_requested_depth_reached", "require_engine_agreement",
        "max_engine_score_swing_cp", "require_bestmove_agreement",
    ])]
    preset: Option<String>,
}

#[derive(clap::Args)]
struct CalibrateArgs {
    /// Input labeled positions JSONL
    #[arg(short, long)]
    input: PathBuf,
    /// Output CSV: one row per (swept parameter, swept value)
    #[arg(short, long)]
    out: PathBuf,
    /// Comma-separated values to sweep for the equivalent of `filter --min-policy-margin-cp`
    /// (e.g. "0,40,80,120,160")
    #[arg(long, conflicts_with = "min_policy_margin_cp")]
    sweep_policy_margin: Option<String>,
    /// Comma-separated values to sweep for the equivalent of `filter --max-score-swing-cp`
    /// (e.g. "50,100,150,200")
    #[arg(long, conflicts_with = "max_score_swing_cp")]
    sweep_score_swing: Option<String>,
    /// Hold min_policy_margin_cp at this fixed value while sweeping --sweep-score-swing
    /// (rejected together with --sweep-policy-margin, which sweeps this same field)
    #[arg(long, allow_hyphen_values = true)]
    min_policy_margin_cp: Option<i32>,
    /// Hold max_score_swing_cp at this fixed value while sweeping --sweep-policy-margin
    /// (rejected together with --sweep-score-swing, which sweeps this same field)
    #[arg(long)]
    max_score_swing_cp: Option<i32>,
    /// Minimum number of observations required (default: 1) -- same base-config fields as
    /// `filter`, held fixed across every swept value
    #[arg(long, default_value = "1")]
    min_observations: u32,
    /// Filter by game phase: comma-separated (opening,middlegame,endgame)
    #[arg(long)]
    phase: Option<String>,
    #[arg(long)]
    exclude_mate: bool,
    #[arg(long)]
    exclude_in_check: bool,
    #[arg(long)]
    exclude_capture: bool,
    /// Minimum cp score, from Black's perspective, regardless of whose turn it was
    #[arg(long, allow_hyphen_values = true)]
    eval_min: Option<i32>,
    /// Maximum cp score, from Black's perspective, regardless of whose turn it was
    #[arg(long, allow_hyphen_values = true)]
    eval_max: Option<i32>,
    #[arg(long)]
    require_bestmove_agreement: bool,
    #[arg(long)]
    require_engine_agreement: bool,
    #[arg(long)]
    max_engine_score_swing_cp: Option<i32>,
    #[arg(long)]
    require_exact_score: bool,
    #[arg(long)]
    require_policy_margin: bool,
    #[arg(long)]
    min_depth_reached: Option<u32>,
    #[arg(long)]
    require_requested_depth_reached: bool,
}

#[derive(clap::Args)]
struct AuditArgs {
    /// Input labeled positions JSONL (must already contain both the teacher and student depths,
    /// e.g. from one `label --depths 6,8,10,14` run)
    #[arg(short, long)]
    input: PathBuf,
    /// The depth treated as ground truth for each engine
    #[arg(long)]
    teacher_depth: u32,
    /// Comma-separated shallower depths to compare against the teacher depth (e.g. "6,8,10")
    #[arg(long)]
    student_depths: String,
    /// Output JSONL: one line per (record, engine, student_depth) pair with both depths present
    #[arg(short, long)]
    out: PathBuf,
}

#[derive(clap::Args)]
struct TuneArgs {
    /// Input labeled positions JSONL (must already contain both the teacher and student depths,
    /// e.g. from one `label --depths 6,8,10,14` run)
    #[arg(short, long)]
    input: PathBuf,
    /// The depth treated as ground truth for each engine
    #[arg(long)]
    teacher_depth: u32,
    /// Comma-separated shallower depths to compare against the teacher depth (e.g. "6,8,10")
    #[arg(long)]
    student_depths: String,
    /// Comma-separated values to sweep for the equivalent of `filter --min-policy-margin-cp`
    /// (e.g. "0,40,80,120,160")
    #[arg(long, conflicts_with = "min_policy_margin_cp")]
    sweep_policy_margin: Option<String>,
    /// Comma-separated values to sweep for the equivalent of `filter --max-score-swing-cp`
    /// (e.g. "50,100,150,200")
    #[arg(long, conflicts_with = "max_score_swing_cp")]
    sweep_score_swing: Option<String>,
    /// Hold min_policy_margin_cp at this fixed value while sweeping --sweep-score-swing
    /// (rejected together with --sweep-policy-margin, which sweeps this same field)
    #[arg(long, allow_hyphen_values = true)]
    min_policy_margin_cp: Option<i32>,
    /// Hold max_score_swing_cp at this fixed value while sweeping --sweep-policy-margin
    /// (rejected together with --sweep-score-swing, which sweeps this same field)
    #[arg(long)]
    max_score_swing_cp: Option<i32>,
    /// Minimum number of observations required (default: 1) -- same base-config fields as
    /// `filter`/`calibrate`, held fixed across every grid cell
    #[arg(long, default_value = "1")]
    min_observations: u32,
    /// Filter by game phase: comma-separated (opening,middlegame,endgame)
    #[arg(long)]
    phase: Option<String>,
    #[arg(long)]
    exclude_mate: bool,
    #[arg(long)]
    exclude_in_check: bool,
    #[arg(long)]
    exclude_capture: bool,
    /// Minimum cp score, from Black's perspective, regardless of whose turn it was
    #[arg(long, allow_hyphen_values = true)]
    eval_min: Option<i32>,
    /// Maximum cp score, from Black's perspective, regardless of whose turn it was
    #[arg(long, allow_hyphen_values = true)]
    eval_max: Option<i32>,
    #[arg(long)]
    require_bestmove_agreement: bool,
    #[arg(long)]
    require_engine_agreement: bool,
    #[arg(long)]
    max_engine_score_swing_cp: Option<i32>,
    #[arg(long)]
    require_exact_score: bool,
    #[arg(long)]
    require_policy_margin: bool,
    #[arg(long)]
    min_depth_reached: Option<u32>,
    #[arg(long)]
    require_requested_depth_reached: bool,
    /// Output CSV: one row per (policy_margin, score_swing) grid configuration
    #[arg(short, long)]
    out: PathBuf,
    /// Optional Markdown report with a Pareto-frontier analysis and 3 recommended candidates
    /// (broad/balanced/strict) -- shogiesa doesn't pick a single "correct" threshold, since
    /// whether a training run wants quantity or reliability varies run to run.
    #[arg(long)]
    report: Option<PathBuf>,
    /// Optional machine-readable JSON with the same broad/balanced/strict candidates as
    /// --report, each carrying a full QualityConfig ready for `filter --preset FILE.json:label`.
    /// Unlike --report's Markdown (for humans), this is meant to be fed directly back into
    /// filter -- hand-transcribing thresholds from the Markdown report breaks reproducibility.
    #[arg(long)]
    preset_out: Option<PathBuf>,
}

#[derive(clap::Args)]
struct CacheArgs {
    #[command(subcommand)]
    action: CacheAction,
}

#[derive(clap::Subcommand)]
enum CacheAction {
    /// File count, total size, oldest/newest entry age, and per-engine distribution
    Stats(CacheStatsArgs),
    /// Detect corrupted (unparseable) cache entries
    Verify(CacheVerifyArgs),
    /// Delete matched cache entries (dry run by default)
    Prune(CachePruneArgs),
}

#[derive(clap::Args)]
struct CacheStatsArgs {
    /// The `label --cache-dir` directory to inspect
    #[arg(long)]
    cache_dir: PathBuf,
}

#[derive(clap::Args)]
struct CacheVerifyArgs {
    /// The `label --cache-dir` directory to inspect
    #[arg(long)]
    cache_dir: PathBuf,
}

#[derive(clap::Args)]
struct CachePruneArgs {
    /// The `label --cache-dir` directory to prune
    #[arg(long)]
    cache_dir: PathBuf,
    /// Delete entries whose JSON fails to parse
    #[arg(long)]
    corrupted_only: bool,
    /// Delete entries in the old (pre-envelope) bare-Observation cache format, once v2 has been
    /// running long enough that the legacy bucket is confidently redundant
    #[arg(long)]
    legacy_only: bool,
    /// Delete entries whose file hasn't been written to in at least this many days
    #[arg(long)]
    older_than_days: Option<u64>,
    /// Actually delete matched entries. Without this, reports what would be deleted and deletes
    /// nothing -- this is the first genuinely destructive command in this CLI, so it defaults to
    /// dry-run rather than everything else's "only ever writes new output files" convention.
    #[arg(long)]
    yes: bool,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Extract(args) => cmd_extract(args),
        Commands::Label(args) => cmd_label(args),
        Commands::Stability(args) => cmd_stability(args),
        Commands::Split(args) => cmd_split(args),
        Commands::Sample(args) => cmd_sample(args),
        Commands::Mine(args) => cmd_mine(args),
        Commands::Balance(args) => cmd_balance(args),
        Commands::Select(args) => cmd_select(args),
        Commands::Pack(args) => cmd_pack(args),
        Commands::Unpack(args) => cmd_unpack(args),
        Commands::Filter(args) => cmd_filter(args),
        Commands::Calibrate(args) => cmd_calibrate(args),
        Commands::Audit(args) => cmd_audit(args),
        Commands::Tune(args) => cmd_tune(args),
        Commands::Report(args) => cmd_report(args),
        Commands::Validate(args) => cmd_validate(args),
        Commands::Cache(args) => match args.action {
            CacheAction::Stats(a) => cmd_cache_stats(a),
            CacheAction::Verify(a) => cmd_cache_verify(a),
            CacheAction::Prune(a) => cmd_cache_prune(a),
        },
        Commands::FromMatch(args) => cmd_from_match(args),
        Commands::MergeObservations(args) => cmd_merge_observations(args),
    }
}

/// Why: extracted so the zobrist-sentinel-collision fix (an earlier `unwrap_or(0)` merged every
/// unparseable SFEN into one "duplicate") can be unit-tested directly -- CSA/KIF extraction from
/// real game files can't produce an unparseable SFEN, so this can't be exercised end-to-end.
fn zobrist_dedup_keep(
    rec: &PositionRecord,
    seen_hashes: &mut HashSet<u64>,
    skipped: &mut usize,
) -> bool {
    match zobrist_from_sfen(&rec.sfen) {
        Some(hash) => seen_hashes.insert(hash),
        None => {
            tracing::warn!(sfen = %rec.sfen, "cannot zobrist-hash SFEN, skipping");
            *skipped += 1;
            false
        }
    }
}

fn cmd_extract(args: ExtractArgs) -> Result<()> {
    let config = shogiesa_core::ExtractConfig {
        min_ply: args.min_ply,
        max_ply: args.max_ply,
        every_n: args.every_n_plies,
        dedup: args.dedup,
    };

    let paths = collect_game_paths(&args.input)?;
    if paths.is_empty() {
        anyhow::bail!("no .csa or .kif files found in {:?}", args.input);
    }

    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);

    let use_zobrist = args.dedup_zobrist;
    // For Zobrist dedup, disable SFEN dedup in the extractor and handle it here.
    let extract_config = if use_zobrist {
        shogiesa_core::ExtractConfig {
            dedup: false,
            ..config
        }
    } else {
        config
    };
    let mut seen: HashSet<String> = HashSet::new();
    let mut seen_hashes: HashSet<u64> = HashSet::new();
    let mut total_games = 0usize;
    let mut total_positions = 0usize;
    let mut skipped = 0usize;

    for path in &paths {
        total_games += 1;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let result = match ext {
            "kif" | "ki2" => shogiesa_kif::extract_from_path(path, &extract_config, &mut seen)
                .map_err(|e| e.to_string()),
            _ => shogiesa_csa::extract_from_path(path, &extract_config, &mut seen)
                .map_err(|e| e.to_string()),
        };
        match result {
            Ok(records) => {
                for rec in &records {
                    if use_zobrist && !zobrist_dedup_keep(rec, &mut seen_hashes, &mut skipped) {
                        continue;
                    }
                    serde_json::to_writer(&mut writer, rec)?;
                    writer.write_all(b"\n")?;
                    total_positions += 1;
                }
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), "skipped: {e}");
                skipped += 1;
            }
        }
        info!(
            games = total_games,
            positions = total_positions,
            "processed {}",
            path.display()
        );
    }

    writer.flush()?;
    eprintln!(
        "done: {} games, {} positions extracted, {} skipped → {:?}",
        total_games, total_positions, skipped, args.out
    );
    Ok(())
}

/// Which literal kifu-file label ("Engine1"/"Engine2") won a match-runner game, per its own
/// "# Result: ..." header line. Kept distinct from any candidate/baseline naming convention --
/// match-runner's own source doesn't guarantee which physical engine slot is "the candidate"
/// under test, so `from-match` only ever reasons about the labels the kifu file itself states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchResult {
    Engine1Win,
    Engine2Win,
    Draw,
}

/// Every `.txt` file directly inside `input` if it's a directory, or `input` itself if it's a
/// single file -- mirrors `collect_game_paths`'s file-or-directory convention but filters on the
/// match-runner's own kifu extension instead of game-record formats.
fn collect_match_kifu_paths(input: &Path) -> Result<Vec<PathBuf>> {
    if input.is_file() {
        return Ok(vec![input.to_path_buf()]);
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(input).with_context(|| format!("cannot read directory {input:?}"))? {
        let p = entry?.path();
        if p.extension().and_then(|e| e.to_str()) == Some("txt") {
            paths.push(p);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Parses one Sekirei match-runner kifu `.txt` file's header lines (everything before the
/// `position ...` line). Unrecognized lines are ignored, not errors -- forward-compatible with
/// extra headers the match-runner might add later.
fn parse_match_kifu_header(line: &str, result: &mut Option<MatchResult>) {
    match line.trim() {
        "# Result: Engine1 Win" => *result = Some(MatchResult::Engine1Win),
        "# Result: Engine2 Win" => *result = Some(MatchResult::Engine2Win),
        "# Result: Draw" => *result = Some(MatchResult::Draw),
        _ => {}
    }
}

/// Whether `--losing-side` (if given) selects this game, per the kifu's own literal Result line.
/// A Draw or missing/unparseable Result is never "a loss for either side" -- skipped entirely
/// under an explicit `--losing-side`, matching the plain English reading of the flag.
fn match_qualifies(losing_side: Option<&str>, result: Option<MatchResult>) -> bool {
    match losing_side {
        None => true,
        Some("engine1") => result == Some(MatchResult::Engine2Win),
        Some("engine2") => result == Some(MatchResult::Engine1Win),
        Some(_) => false, // unreachable: clap's value_parser restricts this already
    }
}

/// Extracts positions from one match-runner kifu file's content. Mirrors
/// `shogiesa_csa::extract_from_str`'s structure (stop-this-game-on-error resilience, same
/// `ExtractConfig` gates, same dedup set) but replays a `position startpos moves ...` USI move
/// list instead of a CSA/KIF game record.
fn extract_from_match_kifu(
    content: &str,
    source_path: &str,
    losing_side: Option<&str>,
    config: &shogiesa_core::ExtractConfig,
    seen: &mut HashSet<String>,
) -> Vec<PositionRecord> {
    let mut result = None;
    let mut position_line = None;
    for line in content.lines() {
        if line.starts_with("position ") {
            position_line = Some(line);
            break;
        }
        parse_match_kifu_header(line, &mut result);
    }
    if !match_qualifies(losing_side, result) {
        return Vec::new();
    }
    let Some(position_line) = position_line else {
        tracing::warn!(
            path = source_path,
            "no `position ...` line found, skipping game"
        );
        return Vec::new();
    };

    let tokens: Vec<&str> = position_line.split_whitespace().collect();
    let (mut board, move_tokens): (Board, &[&str]) = match tokens.as_slice() {
        ["position", "startpos", "moves", rest @ ..] => (Board::initial(SideToMove::Black), rest),
        ["position", "startpos"] => (Board::initial(SideToMove::Black), &[]),
        ["position", "sfen", ..] => {
            // No SFEN -> Board reconstructor exists anywhere in shogiesa today (verified during
            // planning) and this form was never observed in real match-runner output -- warn and
            // skip rather than build an unexercised subsystem for a form that hasn't occurred.
            tracing::warn!(
                path = source_path,
                "`position sfen ...` not supported, skipping game"
            );
            return Vec::new();
        }
        _ => {
            tracing::warn!(
                path = source_path,
                "unrecognized `position` line, skipping game"
            );
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    let mut ply: u32 = 0;
    for token in move_tokens {
        let mv = match parse_usi_move(token) {
            Ok(mv) => mv,
            Err(e) => {
                tracing::warn!(path = source_path, ply, "malformed move {token:?}: {e}");
                break;
            }
        };
        let mover = board.side;
        let has_capture = match mv {
            UsiMove::Normal {
                to_file, to_rank, ..
            } => board.is_capture(to_file, to_rank, mover),
            UsiMove::Drop { .. } => false,
        };
        if let Err(e) = board.apply_usi_move(mover, &mv) {
            tracing::warn!(path = source_path, ply, "board error: {e}");
            break;
        }
        ply += 1;

        if config.max_ply.is_some_and(|max| ply > max) {
            break;
        }
        if ply < config.min_ply {
            continue;
        }
        if !(ply - config.min_ply).is_multiple_of(config.every_n) {
            continue;
        }

        let sfen = board.to_sfen();
        if config.dedup && !seen.insert(sfen.clone()) {
            continue;
        }

        let tags = PositionTags {
            phase: phase_from_ply(ply),
            side_to_move: board.side,
            in_check: board.is_in_check(),
            has_capture,
        };
        // A match-runner game is strictly linear (no variation/branch concept) -- the kifu
        // file's own path already carries unambiguous run+game identity, exactly how CSA
        // extraction already relies on bare `path` with `root_id: None` today. No stamped
        // win/loss "outcome" field either: `--losing-side` selection above already IS the
        // filter, so by the time a record exists here its qualifying-game status is a fact
        // about which file it came from, not something downstream code needs to re-check.
        let source = SourceInfo {
            kind: "from_match".to_string(),
            path: source_path.to_string(),
            ply,
            root_id: None,
            variation_id: None,
            branch_from_ply: None,
        };
        out.push(PositionRecord::new(sfen, source, tags));
    }
    out
}

fn cmd_from_match(args: FromMatchArgs) -> Result<()> {
    let config = shogiesa_core::ExtractConfig {
        min_ply: args.min_ply,
        max_ply: args.max_ply,
        every_n: args.every_n_plies,
        dedup: args.dedup,
    };

    let paths = collect_match_kifu_paths(&args.input)?;
    if paths.is_empty() {
        anyhow::bail!("no .txt kifu files found in {:?}", args.input);
    }

    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);
    let mut seen: HashSet<String> = HashSet::new();
    let mut total_games = 0usize;
    let mut total_positions = 0usize;

    for path in &paths {
        total_games += 1;
        let content = fs::read_to_string(path).with_context(|| format!("cannot read {path:?}"))?;
        let source = path.to_string_lossy().into_owned();
        let records = extract_from_match_kifu(
            &content,
            &source,
            args.losing_side.as_deref(),
            &config,
            &mut seen,
        );
        for rec in &records {
            serde_json::to_writer(&mut writer, rec)?;
            writer.write_all(b"\n")?;
            total_positions += 1;
        }
    }

    writer.flush()?;
    eprintln!(
        "done: {total_games} games read, {total_positions} positions extracted → {:?}",
        args.out
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum MergeObservationPolicy {
    KeepBoth,
    PreferPrimary,
    PreferSecondary,
}

/// Merges `incoming` into `base` per `policy`, keyed on `(engine, engine_version, depth,
/// requested_depth)` -- deliberately broader than `label`'s own in-place dedup key (which omits
/// `engine_version`): that narrower key is safe there because `label` already knows it's
/// re-running the same engine binary, but `merge-observations` is explicitly merging data whose
/// provenance might differ, so conflating two different engine versions at the same nominal
/// depth would be a real bug this command must guard against that `label` doesn't need to.
/// Returns how many keys collided.
fn merge_observations_into(
    base: &mut Vec<Observation>,
    incoming: Vec<Observation>,
    policy: MergeObservationPolicy,
) -> usize {
    let key = |o: &Observation| {
        (
            o.engine.clone(),
            o.engine_version.clone(),
            o.depth,
            o.requested_depth,
        )
    };
    let mut collisions = 0usize;
    match policy {
        MergeObservationPolicy::KeepBoth => base.extend(incoming),
        MergeObservationPolicy::PreferPrimary => {
            for obs in incoming {
                let k = key(&obs);
                if base.iter().any(|o| key(o) == k) {
                    collisions += 1;
                    continue;
                }
                base.push(obs);
            }
        }
        MergeObservationPolicy::PreferSecondary => {
            for obs in incoming {
                let k = key(&obs);
                if base.iter().any(|o| key(o) == k) {
                    collisions += 1;
                    base.retain(|o| key(o) != k);
                }
                base.push(obs);
            }
        }
    }
    collisions
}

/// Which record in `--primary` a record in `--secondary` corresponds to: `(sfen, source.path,
/// source.ply)`, not bare `sfen` alone -- bare `sfen` would wrongly conflate two different
/// games/plies that happen to reach an identical position (common in early openings). This
/// triple already uniquely identifies a specific extracted occurrence for every extractor in
/// this codebase and survives unchanged through `label`/`select`/`filter`.
fn merge_alignment_key(rec: &PositionRecord) -> (String, String, u32) {
    (rec.sfen.clone(), rec.source.path.clone(), rec.source.ply)
}

fn cmd_merge_observations(args: MergeObservationsArgs) -> Result<()> {
    let policy = match args.on_collision.as_str() {
        "keep-both" => MergeObservationPolicy::KeepBoth,
        "prefer-primary" => MergeObservationPolicy::PreferPrimary,
        "prefer-secondary" => MergeObservationPolicy::PreferSecondary,
        _ => unreachable!("clap's value_parser already restricts --on-collision"),
    };

    let (secondary_records, _) = load_records(&args.secondary)?;
    // Last-wins on an internally-duplicated secondary key -- a documented limitation, not worth
    // extra bookkeeping for an edge case the real "aligned relabel pass" use case won't hit.
    let secondary_by_key: HashMap<(String, String, u32), usize> = secondary_records
        .iter()
        .enumerate()
        .map(|(i, r)| (merge_alignment_key(r), i))
        .collect();
    let mut consumed = vec![false; secondary_records.len()];

    let (primary_records, _) = load_records(&args.primary)?;
    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);
    let (mut both, mut primary_only, mut collisions) = (0usize, 0usize, 0usize);

    for mut rec in primary_records {
        if let Some(&idx) = secondary_by_key.get(&merge_alignment_key(&rec)) {
            consumed[idx] = true;
            both += 1;
            collisions += merge_observations_into(
                &mut rec.observations,
                secondary_records[idx].observations.clone(),
                policy,
            );
            // Stale after merging in observations the pre-merge stability wasn't computed from --
            // a stability score reflecting only one side's observations would silently
            // misrepresent the merged set. Re-run `stability` after merging if wanted.
            rec.stability = None;
        } else {
            primary_only += 1;
        }
        serde_json::to_writer(&mut writer, &rec)?;
        writer.write_all(b"\n")?;
    }

    let mut secondary_only = 0usize;
    for (idx, rec) in secondary_records.into_iter().enumerate() {
        if !consumed[idx] {
            serde_json::to_writer(&mut writer, &rec)?;
            writer.write_all(b"\n")?;
            secondary_only += 1;
        }
    }
    writer.flush()?;

    eprintln!(
        "done: {} written ({both} merged, {primary_only} primary-only, {secondary_only} \
         secondary-only, {collisions} colliding observation keys resolved by --on-collision {}) \
         → {:?}",
        both + primary_only + secondary_only,
        args.on_collision,
        args.out
    );
    Ok(())
}

/// Per-run label cache config, shared read-only across worker threads (`hits`/`misses` are the
/// only mutable state, via atomics). Owned rather than borrowed since worker closures need
/// `'static` data.
struct LabelCache {
    dir: PathBuf,
    engine_options_hash: u64,
    multipv: u32,
    /// `None` under `--engine-fingerprint-mode none` (today's behavior: engine identity relies
    /// solely on the USI-reported `id name`/`id version` strings folded in separately). `Some`
    /// otherwise, folding the engine binary itself into the cache key -- see
    /// `compute_engine_fingerprint`.
    engine_fingerprint: Option<u64>,
    /// Which mode produced `engine_fingerprint` -- stored separately from the `Option<u64>` value
    /// itself so a v2 `CacheEntry` can record it (an `Option<u64>` alone can't distinguish "no
    /// fingerprint because mode is `none`" from "no fingerprint because computing it failed").
    engine_fingerprint_mode: EngineFingerprintMode,
    hits: Arc<AtomicUsize>,
    misses: Arc<AtomicUsize>,
}

/// How (if at all) the engine binary itself contributes to the label cache key, on top of its
/// USI-reported `id name`/`id version`. Those strings are controlled by the engine and aren't
/// guaranteed to change after a local rebuild, so relying on them alone risks a cache hit
/// silently reusing labels produced by a different executable.
///
/// Derives `Serialize`/`Deserialize` purely so a v2 `CacheEntry` (below) can store which mode
/// produced its `engine_fingerprint` -- the existing hand-rolled `as_str`/`parse` below (used for
/// CLI arg parsing and the `label --manifest` field) are untouched by this; `serde`'s
/// `rename_all = "snake_case"` happens to already match those same literal strings.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EngineFingerprintMode {
    /// blake3 of the engine binary's bytes, read once at probe launch. The strongest guarantee,
    /// and negligible cost next to actually running search across a dataset.
    Content,
    /// Canonical path + file size + mtime. Cheaper than reading the whole binary, but path- and
    /// mtime-sensitive: a CI job that rebuilds the byte-identical binary into a fresh build
    /// directory every run would invalidate the cache every time despite nothing changing.
    Metadata,
    /// Today's behavior: identity relies solely on the USI id name/version strings.
    None,
}

impl EngineFingerprintMode {
    fn parse(s: &str) -> Self {
        match s {
            "content" => Self::Content,
            "metadata" => Self::Metadata,
            _ => Self::None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Metadata => "metadata",
            Self::None => "none",
        }
    }
}

/// Why this can't just propagate an error: `--engine` is passed straight to
/// `std::process::Command`, which resolves a bare name (no path separator) via `PATH` at spawn
/// time -- but `fs::read`/`fs::canonicalize` only understand literal filesystem paths, with no
/// `PATH` search. A bare engine name that launches fine today would otherwise make `content`/
/// `metadata` fingerprinting hard-fail before the engine is ever spawned, breaking a case that
/// worked before this fingerprinting existed. So an unreadable path falls back to no fingerprint
/// (identical to `--engine-fingerprint-mode none` for this run) with a warning, rather than
/// aborting the whole `label` invocation over what is, at worst, a weaker cache guarantee.
fn compute_engine_fingerprint(mode: EngineFingerprintMode, engine_path: &Path) -> Option<u64> {
    match mode {
        EngineFingerprintMode::None => None,
        EngineFingerprintMode::Content => match fs::read(engine_path) {
            Ok(bytes) => Some(hash_parts_u64(&[&bytes])),
            Err(e) => {
                tracing::warn!(
                    engine = %engine_path.display(),
                    "cannot read engine binary for fingerprinting ({e}) -- is --engine a bare \
                     name resolved via PATH? falling back to no fingerprint for this run"
                );
                None
            }
        },
        EngineFingerprintMode::Metadata => {
            let stat = fs::canonicalize(engine_path).and_then(|p| fs::metadata(&p).map(|m| (p, m)));
            match stat {
                Ok((canonical, meta)) => {
                    let mtime_nanos = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0);
                    let path_bytes = canonical.to_string_lossy().into_owned();
                    Some(hash_parts_u64(&[
                        path_bytes.as_bytes(),
                        &meta.len().to_le_bytes(),
                        &mtime_nanos.to_le_bytes(),
                    ]))
                }
                Err(e) => {
                    tracing::warn!(
                        engine = %engine_path.display(),
                        "cannot stat engine binary for fingerprinting ({e}) -- is --engine a bare \
                         name resolved via PATH? falling back to no fingerprint for this run"
                    );
                    None
                }
            }
        }
    }
}

// Why blake3, not `std::collections::hash_map::DefaultHasher`: DefaultHasher is deterministic
// within one build, but std's own docs disclaim stability *across Rust toolchain versions* --
// every use below is persisted (a cache filename, a manifest fingerprint, or the outcome of a
// split/sample/select decision written to an output file), so a future toolchain silently
// changing the digest would break reproducibility with no error. blake3's digest for a fixed
// input is stable forever by spec, which is the actual property this tool's reproducibility
// mandate needs.

/// Hashes each part with an explicit length prefix so a naive concatenation can't collide across
/// a field boundary (e.g. `"ab"+"c"` vs `"a"+"bc"`) regardless of what the fields contain --
/// used everywhere a persistent, multi-field fingerprint is built.
fn hash_parts(parts: &[&[u8]]) -> blake3::Hash {
    let mut h = blake3::Hasher::new();
    for p in parts {
        h.update(&(p.len() as u64).to_le_bytes());
        h.update(p);
    }
    h.finalize()
}

/// Truncates a blake3 digest to a `u64` for callers that need a plain integer (a sort key, or an
/// `f64` normalization) rather than a persisted filename/string -- blake3 is cryptographic
/// strength, so the leading 8 bytes stay uniformly distributed after truncation.
fn hash_parts_u64(parts: &[&[u8]]) -> u64 {
    u64::from_le_bytes(hash_parts(parts).as_bytes()[..8].try_into().unwrap())
}

/// Hash of the resolved USI engine options, sorted so option order doesn't change the label
/// cache key below.
fn engine_options_hash(options: &[(String, String)]) -> u64 {
    let mut sorted: Vec<&(String, String)> = options.iter().collect();
    sorted.sort();
    let mut parts: Vec<&[u8]> = Vec::with_capacity(sorted.len() * 2);
    for (k, v) in &sorted {
        parts.push(k.as_bytes());
        parts.push(v.as_bytes());
    }
    hash_parts_u64(&parts)
}

/// Versions the on-disk cache-file *envelope* shape, distinct from `SCHEMA_VERSION` (which
/// versions the JSONL/pack `Observation` data model -- the two change independently: an envelope
/// shape change doesn't imply the `Observation` payload inside it changed, and vice versa).
/// Deliberately does NOT participate in `label_cache_path`'s hash below: an old (v1) file at a
/// given key is still a byte-truthful cache hit for that key's `Observation`, just read through
/// the legacy branch in `parse_cache_entry` -- envelope format is a read-time parsing concern, not
/// a cache-validity concern. `SCHEMA_VERSION` already changes the key (and therefore the file) by
/// construction when it bumps; this constant only needs to change if `CacheEntry`'s own field set
/// changes shape in a future round.
const CACHE_SCHEMA_VERSION: u32 = 1;

/// A cached label, plus the metadata that produced it. Introduced in this round specifically so
/// `cache stats`/`verify`/`prune` can report real distributions (which schema/engine/fingerprint/
/// depth/multipv a cache dir's entries were written under) instead of only corruption/size/age --
/// the cache *key* already encodes all of this (see `label_cache_path`), but a key is a one-way
/// hash: there's no way to recover "what schema version was this?" from the filename alone. Storing
/// it in the payload too costs nothing at write time and unlocks introspection at read time.
#[derive(Serialize, Deserialize)]
struct CacheEntry {
    cache_schema_version: u32,
    /// Unix epoch seconds (`SystemTime::now()`). No chrono/time/humantime dependency exists
    /// anywhere in this workspace (`time` is pulled in only transitively via the `csa` crate) --
    /// adding one solely to format a timestamp string isn't justified when a plain integer works.
    created_at: u64,
    schema_version: u32,
    engine_name: String,
    engine_version: Option<String>,
    engine_fingerprint: Option<u64>,
    engine_fingerprint_mode: EngineFingerprintMode,
    engine_options_hash: u64,
    requested_depth: u32,
    multipv: u32,
    observation: Observation,
}

/// Parsed form of one cache file's JSON, regardless of which format wrote it. v2 (`CacheEntry`) is
/// tried first; v1 (a bare `Observation`, this cache format's shape before this round) is the
/// fallback. A v1 entry is old-format, not corrupted, and must never be misreported as corrupted
/// by `cache verify` -- this is the one place every reader (the cache-hit path in
/// `analyze_record`, and `cache stats`/`verify`/`prune`) goes through, so "v1 vs v2 vs actually
/// corrupted" can't drift into disagreeing answers across those call sites.
enum CacheRead {
    V2(CacheEntry),
    V1(Observation),
}

impl CacheRead {
    fn observation(&self) -> &Observation {
        match self {
            Self::V2(entry) => &entry.observation,
            Self::V1(obs) => obs,
        }
    }

    fn into_observation(self) -> Observation {
        match self {
            Self::V2(entry) => entry.observation,
            Self::V1(obs) => obs,
        }
    }
}

/// v2 tried first since it's the active write format (most reads will be a v2 hit) -- not because
/// the reverse order would misparse: `CacheEntry`'s required fields (`cache_schema_version`,
/// `observation`, ...) and `Observation`'s (`engine`, `depth`, `score`, `bestmove`, ...) don't
/// overlap, so each format's JSON object correctly fails the other's shape and falls through
/// either way (verified directly: swapping this order doesn't break any test).
fn parse_cache_entry(json: &str) -> Option<CacheRead> {
    if let Ok(entry) = serde_json::from_str::<CacheEntry>(json) {
        return Some(CacheRead::V2(entry));
    }
    serde_json::from_str::<Observation>(json)
        .ok()
        .map(CacheRead::V1)
}

// Why cache at all: labeling (running the engine) is the dominant cost of the whole pipeline.
// The same engine run against the same position at the same requested depth always produces the
// same observation, so repeated experiments (tuning filter/select downstream, re-running after a
// crash, sharing a labeling budget across datasets) can reuse it instead of paying search cost
// again. Content-addressed sharded JSON files need no database dependency and are trivial to
// inspect/delete by hand, at the cost of one small file per cached observation.
fn label_cache_path(
    cache: &LabelCache,
    sfen: &str,
    engine_name: &str,
    engine_version: Option<&str>,
    requested_depth: u32,
) -> PathBuf {
    // Why an explicit discriminant byte for `engine_version`, not a sentinel string: `None` must
    // never collide with `Some("")` (or any other literal an engine could plausibly report).
    let (version_tag, version_bytes): (&[u8], &[u8]) = match engine_version {
        Some(v) => (&[1], v.as_bytes()),
        None => (&[0], &[]),
    };
    let engine_options_hash_bytes = cache.engine_options_hash.to_le_bytes();
    let requested_depth_bytes = requested_depth.to_le_bytes();
    let multipv_bytes = cache.multipv.to_le_bytes();
    let schema_version_bytes = SCHEMA_VERSION.to_le_bytes();
    // Same Option-discriminant pattern as `engine_version` above: under
    // `--engine-fingerprint-mode none`, every run has `engine_fingerprint: None`, so this
    // contributes the same constant bytes regardless of which binary is running -- i.e. cache
    // identity is exactly today's "USI id name/version only" behavior for that mode.
    let fingerprint_tag: &[u8] = if cache.engine_fingerprint.is_some() {
        &[1]
    } else {
        &[0]
    };
    let fingerprint_bytes = cache.engine_fingerprint.unwrap_or(0).to_le_bytes();
    let key = hash_parts(&[
        sfen.as_bytes(),
        engine_name.as_bytes(),
        version_tag,
        version_bytes,
        &engine_options_hash_bytes,
        &requested_depth_bytes,
        &multipv_bytes,
        &schema_version_bytes,
        fingerprint_tag,
        &fingerprint_bytes,
    ])
    .to_hex()
    .to_string();
    cache.dir.join(&key[0..2]).join(format!("{key}.json"))
}

/// Writes a cache entry via temp-file-then-rename instead of a direct `fs::write`, so a crash,
/// kill, or disk-full mid-write can never leave a torn file visible at `path`. This matters
/// because `--cache-dir` is documented as shareable across concurrent `label` processes (e.g.
/// several experiments pointed at the same cache) -- a torn file could be read by a *different*
/// process mid-write, not just self-heal on a later run of the same one (which already tolerates
/// a missing or corrupt file as a plain cache miss).
fn write_cache_entry_atomically(path: &Path, json: &str) -> std::io::Result<()> {
    let tmp = path.with_file_name(format!(
        "{}.tmp.{}.{:?}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id(),
        std::thread::current().id(),
    ));
    fs::write(&tmp, json)?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        // Someone else already wrote this exact key -- fine, since the key is content-addressed,
        // whoever got there first wrote the same bytes we would have.
        Err(_) if path.exists() => {
            let _ = fs::remove_file(&tmp);
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn analyze_record(
    rec: &mut PositionRecord,
    engine: &mut UsiEngine,
    depths: &[u32],
    timeout_ms: u64,
    existing_policy: ExistingPolicy,
    cache: Option<&LabelCache>,
) {
    for &depth in depths {
        // The engine may stop before reaching `depth` (e.g. a forced mate) — check coverage
        // against what was actually achieved, not the requested depth, before spending a call.
        if matches!(existing_policy, ExistingPolicy::Skip)
            && rec
                .observations
                .iter()
                .any(|o| o.engine == engine.engine_name && o.depth >= depth)
        {
            continue;
        }

        let cache_path = cache.map(|c| {
            label_cache_path(
                c,
                &rec.sfen,
                &engine.engine_name,
                engine.engine_version.as_deref(),
                depth,
            )
        });
        let cached: Option<Observation> = cache_path.as_ref().and_then(|path| {
            let hit = fs::read_to_string(path)
                .ok()
                .and_then(|s| parse_cache_entry(&s))
                .map(CacheRead::into_observation);
            if let Some(c) = cache {
                c.hits.fetch_add(hit.is_some() as usize, Ordering::Relaxed);
                c.misses
                    .fetch_add(hit.is_none() as usize, Ordering::Relaxed);
            }
            hit
        });

        let observation = match cached {
            Some(obs) => Some(obs),
            None => match engine.analyse(&rec.sfen, depth, timeout_ms) {
                Ok(result) => {
                    let obs = Observation {
                        engine: engine.engine_name.clone(),
                        engine_version: engine.engine_version.clone(),
                        depth: result.depth,
                        requested_depth: Some(depth),
                        score: result.score,
                        // `label` never converts the engine's raw side-to-move-relative cp, so
                        // every observation it produces is explicitly `SideToMove`, not just
                        // implicitly so.
                        score_perspective: ScorePerspective::SideToMove,
                        score_bound: result.score_bound,
                        bestmove: result.bestmove,
                        bestmove_kind: result.bestmove_kind,
                        nodes: result.nodes,
                        time_ms: result.time_ms,
                        pv: result.pv,
                        policy_margin_cp: result.policy_margin_cp,
                        candidates: result.candidates,
                    };
                    if let Some(path) = &cache_path {
                        if let Some(parent) = path.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        // Cache writes always produce v2 (CacheEntry) -- only the read path needs
                        // to understand v1, for cache dirs populated before this round.
                        if let Some(c) = cache {
                            let entry = CacheEntry {
                                cache_schema_version: CACHE_SCHEMA_VERSION,
                                created_at: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0),
                                schema_version: SCHEMA_VERSION,
                                engine_name: obs.engine.clone(),
                                engine_version: obs.engine_version.clone(),
                                engine_fingerprint: c.engine_fingerprint,
                                engine_fingerprint_mode: c.engine_fingerprint_mode,
                                engine_options_hash: c.engine_options_hash,
                                requested_depth: depth,
                                multipv: c.multipv,
                                observation: obs.clone(),
                            };
                            if let Ok(json) = serde_json::to_string(&entry) {
                                let _ = write_cache_entry_atomically(path, &json);
                            }
                        }
                    }
                    Some(obs)
                }
                Err(e) => {
                    tracing::warn!(depth, "analysis error: {e}");
                    None
                }
            },
        };

        if let Some(obs) = observation {
            // Dedupe on the achieved depth, not the requested one, for the same reason —
            // if Skip couldn't skip (under-reach) and re-achieves the same depth, this
            // replaces the stale entry instead of duplicating it. Also require
            // requested_depth to match (or be absent, for pre-field legacy entries): without
            // this, "requested 12, reached 8" and "requested 8, reached 8" would collide and
            // silently erase the distinction requested_depth exists to preserve.
            if !matches!(existing_policy, ExistingPolicy::Append) {
                rec.observations.retain(|o| {
                    !(o.engine == obs.engine
                        && o.depth == obs.depth
                        && (o.requested_depth.is_none()
                            || o.requested_depth == obs.requested_depth))
                });
            }
            rec.observations.push(obs);
        }
    }
}

/// One position in flight through the label pipeline, tagged with its input line order so the
/// writer can restore that order regardless of which worker finishes it first.
struct Job {
    id: u64,
    record: PositionRecord,
}

/// Feeds one out-of-order arrival into the reorder buffer and returns every record that became
/// writable as a result, in order. Kept as a pure function, separate from the threading and I/O
/// around it, so the ordering logic itself can be unit-tested without a real label pipeline.
fn reorder_push(
    pending: &mut BTreeMap<u64, PositionRecord>,
    next_id: &mut u64,
    id: u64,
    record: PositionRecord,
) -> Vec<PositionRecord> {
    pending.insert(id, record);
    let mut ready = Vec::new();
    while let Some(record) = pending.remove(next_id) {
        ready.push(record);
        *next_id += 1;
    }
    ready
}

fn cmd_label(args: LabelArgs) -> Result<()> {
    // Bounded-pipeline changes (worker count, cache usage, ordering) can't be judged without
    // measuring their actual effect on throughput -- this run's wall-clock start, used for
    // `records_per_sec` below.
    let run_start = std::time::Instant::now();
    let depths: Vec<u32> = args
        .depths
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if depths.is_empty() {
        anyhow::bail!("--depths must contain at least one valid integer, e.g. '4,6,8'");
    }

    let engine_path = args.engine.clone();
    let engine_name = args.engine_name.clone().unwrap_or_default();
    let timeout_ms = args.timeout_ms;
    let jobs = args.jobs.max(1);
    let mut engine_options: Vec<(String, String)> = args
        .engine_options
        .iter()
        .filter_map(|s| {
            let (k, v) = s.split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
        .collect();
    if args.multipv > 1 {
        engine_options.push(("MultiPV".to_string(), args.multipv.to_string()));
    }
    let existing_policy = if args.skip_existing {
        ExistingPolicy::Skip
    } else if args.replace_existing {
        ExistingPolicy::Replace
    } else {
        ExistingPolicy::Append
    };
    let engine_fingerprint_mode = EngineFingerprintMode::parse(&args.engine_fingerprint_mode);
    let cache: Option<Arc<LabelCache>> = match &args.cache_dir {
        Some(dir) => {
            // Read once, before any workers spawn -- negligible next to actually running search
            // across a dataset, and every worker shares this same value via `LabelCache`.
            let engine_fingerprint =
                compute_engine_fingerprint(engine_fingerprint_mode, &engine_path);
            Some(Arc::new(LabelCache {
                dir: dir.clone(),
                engine_options_hash: engine_options_hash(&engine_options),
                multipv: args.multipv,
                engine_fingerprint,
                engine_fingerprint_mode,
                hits: Arc::new(AtomicUsize::new(0)),
                misses: Arc::new(AtomicUsize::new(0)),
            }))
        }
        None => None,
    };

    // Verify the engine launches before committing to any pipeline work.
    let probe = UsiEngine::launch(
        &engine_path,
        engine_name.clone(),
        timeout_ms,
        &engine_options,
    )
    .with_context(|| format!("failed to launch engine {engine_path:?}"))?;
    let engine_display_name = probe.engine_name.clone();
    drop(probe); // cleanly quits via Drop

    info!(jobs, "labeling started");

    // Why a permit-based dispatch window, not just a bounded reader→worker queue: once a job is
    // pulled off that queue by a worker, the queue no longer holds it -- but the job isn't
    // *retired* until the writer durably outputs it. If job 0 is slow and jobs 1..N are fast,
    // workers race through 1..N and hand them to the writer, whose reorder buffer (waiting on
    // job 0) would grow without bound. A fixed pool of permits, released only once the writer
    // durably outputs a job, bounds "dispatched but not yet written" to a constant regardless of
    // which job finishes last -- a slow job 0 now stalls the *reader* (out of permits), not the
    // writer's memory.
    let queue_depth = jobs * 4;
    let (permit_tx, permit_rx) = mpsc::sync_channel::<()>(queue_depth);
    for _ in 0..queue_depth {
        permit_tx.send(()).expect("permit channel just created");
    }
    let writer_permit_tx = permit_tx.clone();
    drop(permit_tx);

    // Small hand-off queue from the reader to the worker pool -- its capacity only smooths
    // scheduling jitter, since the permit scheme above is what actually bounds memory.
    let (job_tx, job_rx) = mpsc::sync_channel::<Job>(jobs);
    let job_rx = Arc::new(Mutex::new(job_rx));
    let (result_tx, result_rx) = mpsc::channel::<Job>();

    let engine_launch_failures = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicUsize::new(0));

    // Reader: streams the input line-by-line so the whole dataset is never resident in memory --
    // only ever `queue_depth` records at a time, enforced by the permit acquire below. It also
    // accumulates the manifest's `input_hash` over the same lines it's already reading -- calling
    // `hash_file` afterward, like every other manifest-producing command does, would mean
    // re-opening and re-reading the whole input a second time purely to hash it, defeating the
    // point of streaming in the first place.
    let input_path = args.input.clone();
    let track_input_hash = args.manifest.is_some();
    let reader_handle = std::thread::spawn(move || -> Result<(u64, usize, String)> {
        let mut input_hasher = blake3::Hasher::new();
        let file =
            File::open(&input_path).with_context(|| format!("cannot open {input_path:?}"))?;
        let reader = BufReader::new(file);
        let mut job_id = 0u64;
        let mut skipped = 0usize;
        for (i, line) in reader.lines().enumerate() {
            let line = line.with_context(|| format!("cannot read {input_path:?}"))?;
            if track_input_hash {
                // Hash every line, including blanks, before the empty-line skip below -- this has
                // to match `hash_file`'s behavior exactly so the same input produces the same
                // `input_hash` whether `label` or `sample`/`balance`/`pack`/`filter` computed it.
                input_hasher.update(line.as_bytes());
                input_hasher.update(b"\n");
            }
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<PositionRecord>(&line) {
                Ok(record) if Sfen::parse(&record.sfen).is_ok() => {
                    // Blocks here once `queue_depth` jobs are dispatched-but-unwritten.
                    if permit_rx.recv().is_err() || job_tx.send(Job { id: job_id, record }).is_err()
                    {
                        break; // writer/workers gone -- don't hang trying to feed a dead pipeline
                    }
                    job_id += 1;
                }
                Ok(_) => {
                    tracing::warn!(line = i + 1, "invalid SFEN, skipping");
                    skipped += 1;
                }
                Err(e) => {
                    tracing::warn!(line = i + 1, "JSON parse error: {e}");
                    skipped += 1;
                }
            }
        }
        Ok((
            job_id,
            skipped,
            input_hasher.finalize().to_hex().to_string(),
        ))
    });

    // Workers: each owns one long-lived USI engine for its whole lifetime, launched lazily on
    // its first job and reused across every job it picks up after that -- spawning a fresh
    // engine per position would hide true search cost behind repeated process-startup overhead.
    let worker_handles: Vec<_> = (0..jobs)
        .map(|_| {
            let job_rx = Arc::clone(&job_rx);
            let result_tx = result_tx.clone();
            let engine_path = engine_path.clone();
            let engine_name = engine_name.clone();
            let engine_options = engine_options.clone();
            let engine_launch_failures = Arc::clone(&engine_launch_failures);
            let done = Arc::clone(&done);
            let depths = depths.clone();
            let cache = cache.clone();
            std::thread::spawn(move || {
                let mut engine: Option<UsiEngine> = None;
                loop {
                    let job = {
                        let rx = job_rx.lock().expect("job queue mutex poisoned");
                        rx.recv()
                    };
                    let Ok(Job { id, mut record }) = job else {
                        break;
                    };
                    if engine.is_none()
                        && let Ok(e) = UsiEngine::launch(
                            &engine_path,
                            engine_name.clone(),
                            timeout_ms,
                            &engine_options,
                        )
                    {
                        engine = Some(e);
                    }
                    if let Some(eng) = engine.as_mut() {
                        analyze_record(
                            &mut record,
                            eng,
                            &depths,
                            timeout_ms,
                            existing_policy,
                            cache.as_deref(),
                        );
                    } else {
                        engine_launch_failures.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(sfen = %record.sfen, "engine unavailable, position left unlabeled");
                    }
                    let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if n.is_multiple_of(200) {
                        eprint!("\r  {n} done");
                    }
                    if result_tx.send(Job { id, record }).is_err() {
                        break;
                    }
                }
            })
        })
        .collect();
    drop(result_tx); // the loop below ends once every worker's clone is also dropped

    // Writer: runs on this thread. Default mode writes each job on arrival, in whatever order
    // workers finish -- interrupt-safe, since nothing is ever held unwritten waiting on a
    // straggler. `--preserve-order` instead buffers out-of-order arrivals in `pending` (bounded to
    // `queue_depth` entries by the permit scheme above) and only flushes the next contiguous
    // job_id, so output order matches input order -- at the cost of losing every already-finished
    // job still buffered behind a slow one if the process is killed before it catches up.
    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut out_writer = BufWriter::new(out_file);
    let mut manifest = args
        .manifest
        .as_ref()
        .map(|_| RunManifest::new("label", &args.input));
    let mut next_id = 0u64;
    let mut pending: BTreeMap<u64, PositionRecord> = BTreeMap::new();
    let mut written = 0usize;
    // Accumulated here (not a second pass) since write_one already sees every written record's
    // observations once, right before writing.
    let mut time_ms_sum = 0u64;
    let mut time_ms_count = 0usize;

    let mut write_one =
        |record: &PositionRecord, manifest: &mut Option<RunManifest>| -> Result<()> {
            serde_json::to_writer(&mut out_writer, record)?;
            out_writer.write_all(b"\n")?;
            // Flush per record (not just once at the very end of the whole run): engine search
            // time dominates I/O time by orders of magnitude here, so this costs nothing
            // measurable, but it shrinks the window where a killed process loses writes still
            // sitting in this BufWriter -- independent of (and much smaller than) the
            // preserve_order/reorder-buffer loss window below.
            out_writer.flush()?;
            if let Some(m) = manifest.as_mut() {
                accumulate_coverage(m, std::slice::from_ref(record));
            }
            for obs in &record.observations {
                if let Some(t) = obs.time_ms {
                    time_ms_sum += t;
                    time_ms_count += 1;
                }
            }
            // Release the permit only now that the record is durably written -- this is the other
            // half of the dispatch window: it caps how far ahead of the writer the pipeline can get.
            let _ = writer_permit_tx.send(());
            Ok(())
        };

    for job in result_rx {
        if args.preserve_order {
            for record in reorder_push(&mut pending, &mut next_id, job.id, job.record) {
                write_one(&record, &mut manifest)?;
                written += 1;
            }
            // Guards the permit scheme's whole reason for existing: without it, `pending` could
            // in principle grow past `queue_depth` and this bounded pipeline would silently
            // revert to the unbounded memory use it replaced.
            debug_assert!(pending.len() <= queue_depth);
        } else {
            write_one(&job.record, &mut manifest)?;
            written += 1;
        }
    }
    out_writer.flush()?;
    eprintln!();

    let (total, skipped, input_hash) = reader_handle
        .join()
        .map_err(|_| anyhow::anyhow!("label reader thread panicked"))??;
    for handle in worker_handles {
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("label worker thread panicked"))?;
    }
    let engine_launch_failures = engine_launch_failures.load(Ordering::Relaxed);
    let cache_counts = cache.as_ref().map(|c| {
        (
            c.hits.load(Ordering::Relaxed),
            c.misses.load(Ordering::Relaxed),
        )
    });

    let elapsed_secs = run_start.elapsed().as_secs_f64();
    let records_per_sec = (elapsed_secs > 0.0).then(|| written as f64 / elapsed_secs);
    let cache_hit_rate = cache_counts.and_then(|(hits, misses)| {
        (hits + misses > 0).then(|| hits as f64 / (hits + misses) as f64)
    });
    let average_engine_time_ms =
        (time_ms_count > 0).then(|| time_ms_sum as f64 / time_ms_count as f64);

    let cache_suffix = cache_counts
        .map(|(hits, misses)| format!(", {hits} cache hits, {misses} cache misses"))
        .unwrap_or_default();
    let throughput_suffix = records_per_sec
        .map(|r| format!(", {r:.1} rec/s"))
        .unwrap_or_default();
    eprintln!(
        "done [{engine_display_name}, jobs={jobs}]: {total} in, {written} labeled, {skipped} skipped, {engine_launch_failures} engine launch failures{cache_suffix}{throughput_suffix} → {:?}",
        args.out
    );
    if let Some(mut manifest) = manifest {
        manifest.input_hash = input_hash;
        manifest.records_read = total as usize;
        manifest.records_kept = written;
        manifest.records_dropped = skipped;
        manifest.engine_name = Some(engine_display_name);
        manifest.depths = Some(depths);
        manifest.multipv = (args.multipv > 1).then_some(args.multipv);
        manifest.engine_options = Some(args.engine_options.clone());
        manifest.jobs = Some(jobs);
        manifest.engine_launch_failures = Some(engine_launch_failures);
        manifest.records_per_sec = records_per_sec;
        manifest.average_engine_time_ms = average_engine_time_ms;
        manifest.preserve_order = Some(args.preserve_order);
        if let Some((hits, misses)) = cache_counts {
            manifest.cache_hits = Some(hits);
            manifest.cache_hit_rate = cache_hit_rate;
            manifest.cache_misses = Some(misses);
            manifest.engine_fingerprint_mode = Some(engine_fingerprint_mode.as_str());
        }
        write_manifest(args.manifest.as_ref().unwrap(), &manifest)?;
    }
    Ok(())
}

fn cmd_stability(args: StabilityArgs) -> Result<()> {
    let reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);

    let mut total = 0usize;
    let mut enriched = 0usize;
    let mut skipped = 0usize;

    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        total += 1;
        let mut rec: PositionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(line = i + 1, "JSON parse error: {e}");
                skipped += 1;
                continue;
            }
        };
        rec.fill_stability();
        if rec.stability.is_some() {
            enriched += 1;
        }
        serde_json::to_writer(&mut writer, &rec)?;
        writer.write_all(b"\n")?;
    }

    writer.flush()?;
    eprintln!(
        "done: {total} read, {enriched} enriched with stability, {skipped} skipped → {:?}",
        args.out
    );
    Ok(())
}

/// One resident file handle in `WriterPool`, tagged with the tick it was last written at so the
/// pool can find its least-recently-used entry in a bounded linear scan (fine at ≤`max_open`
/// entries -- a dedicated LRU crate would be over-engineering for this bound).
struct WriterPoolEntry {
    writer: BufWriter<File>,
    last_used: u64,
}

/// Bounded pool of per-source output file handles for `split --by-source`, so a corpus with far
/// more distinct source games than a process's FD limit doesn't exhaust it. On a miss at
/// capacity, evicts (and explicitly flushes -- so a write error surfaces here, not silently
/// swallowed by `BufWriter`'s `Drop`) whichever resident file was written to longest ago, then
/// opens the target. `opened_this_run` is tracked separately from the (evictable) `writers` map:
/// a source's *first* touch this run truncates (matching this command's previous fresh-file
/// behavior), but every later open -- whether still resident or evicted-and-reopened -- must
/// append, or a re-open after eviction would silently discard everything written before it.
struct WriterPool {
    max_open: usize,
    writers: HashMap<String, WriterPoolEntry>,
    opened_this_run: HashSet<String>,
    tick: u64,
}

impl WriterPool {
    fn new(max_open: usize) -> Self {
        Self {
            max_open,
            writers: HashMap::new(),
            opened_this_run: HashSet::new(),
            tick: 0,
        }
    }

    fn write_line(&mut self, out_path: &Path, key: &str, rec: &PositionRecord) -> Result<()> {
        self.tick += 1;
        let tick = self.tick;
        if let Some(entry) = self.writers.get_mut(key) {
            serde_json::to_writer(&mut entry.writer, rec)?;
            entry.writer.write_all(b"\n")?;
            entry.last_used = tick;
            return Ok(());
        }

        if self.writers.len() >= self.max_open {
            let lru_key = self
                .writers
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone())
                .expect("writers is non-empty when at capacity");
            let mut evicted = self.writers.remove(&lru_key).unwrap();
            evicted.writer.flush()?;
        }

        let first_touch = self.opened_this_run.insert(key.to_string());
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(first_touch)
            .append(!first_touch)
            .open(out_path)
            .with_context(|| format!("cannot open {out_path:?}"))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, rec)?;
        writer.write_all(b"\n")?;
        self.writers.insert(
            key.to_string(),
            WriterPoolEntry {
                writer,
                last_used: tick,
            },
        );
        Ok(())
    }

    fn flush_all(&mut self) -> Result<()> {
        for entry in self.writers.values_mut() {
            entry.writer.flush()?;
        }
        Ok(())
    }
}

fn cmd_split(args: SplitArgs) -> Result<()> {
    if args.train.is_some() || args.valid.is_some() || args.test.is_some() {
        return cmd_split_train_valid_test(args);
    }

    if !args.by_source {
        anyhow::bail!("either --by-source --out-dir, or --train/--valid/--test, is required");
    }
    let out_dir = args
        .out_dir
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("--out-dir is required with --by-source"))?;
    fs::create_dir_all(out_dir).with_context(|| format!("cannot create {out_dir:?}"))?;

    let reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );

    // Group records by source path, writing into per-source output files via a bounded LRU pool
    let mut pool = WriterPool::new(args.max_open_writers.max(1));
    let mut file_names: HashMap<String, String> = HashMap::new();
    let mut file_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;

    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: PositionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(line = i + 1, "JSON parse error: {e}");
                continue;
            }
        };

        let key = rec.source.path.clone();
        let file_name = file_names
            .entry(key.clone())
            .or_insert_with(|| {
                let safe: String = key
                    .chars()
                    .map(|c| {
                        if c.is_alphanumeric() || c == '.' || c == '-' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                format!("{safe}.jsonl")
            })
            .clone();
        let out_path = out_dir.join(&file_name);
        pool.write_line(&out_path, &key, &rec)?;
        *file_counts.entry(file_name).or_default() += 1;
        total += 1;
    }
    pool.flush_all()?;

    let manifest = serde_json::json!({
        "shogiesa_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION,
        "input": args.input,
        "by_source": args.by_source,
        "total_positions": total,
        "files": file_counts,
    });
    let manifest_path = out_dir.join("manifest.json");
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)
        .with_context(|| format!("cannot write {manifest_path:?}"))?;

    eprintln!(
        "done: {total} positions split into {} files → {:?}",
        file_counts.len(),
        out_dir
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SplitBucket {
    Test,
    Valid,
    Train,
}

/// Strip a KIF variation suffix (`#varN@ply`, from shogiesa-kif's `extract_from_str`) so a
/// variation's positions hash into the same split bucket as the mainline game it branched
/// from. A variation only emits moves from its branch ply onward, so what's at stake isn't
/// identical positions landing in different splits -- it's *correlation* between siblings:
/// mainline-ply-N and variation-ply-N share the exact same parent position and near-identical
/// context. Splitting correlated siblings across train/valid/test lets the model be evaluated
/// on positions correlated with ones it trained on, inflating validation results.
fn split_root_path(source_path: &str) -> &str {
    source_path.split("#var").next().unwrap_or(source_path)
}

/// The grouping key used to keep a game's mainline and its variations in the same split bucket.
/// Prefers `source.root_id` (set by extractors that produce variations, e.g. shogiesa-kif) since
/// it doesn't depend on parsing a string convention back out of `path`; falls back to
/// `split_root_path` for JSONL/extractors (CSA, or anything predating this field) that never set
/// `root_id`, preserving old behavior exactly for that data.
fn split_root_key(source: &SourceInfo) -> &str {
    source
        .root_id
        .as_deref()
        .unwrap_or_else(|| split_root_path(&source.path))
}

fn assign_split_bucket(seed: u64, root_key: &str, valid_frac: f64, test_frac: f64) -> SplitBucket {
    let seed_bytes = seed.to_le_bytes();
    let unit = hash_parts_u64(&[&seed_bytes, root_key.as_bytes()]) as f64 / u64::MAX as f64;
    if unit < test_frac {
        SplitBucket::Test
    } else if unit < test_frac + valid_frac {
        SplitBucket::Valid
    } else {
        SplitBucket::Train
    }
}

fn cmd_split_train_valid_test(args: SplitArgs) -> Result<()> {
    let (Some(train_path), Some(valid_path), Some(test_path)) =
        (&args.train, &args.valid, &args.test)
    else {
        anyhow::bail!("--train, --valid, and --test must all be provided together");
    };
    if args.valid_frac + args.test_frac >= 1.0 {
        anyhow::bail!("--valid-frac + --test-frac must be < 1.0");
    }

    let reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );

    let mut train_writer = BufWriter::new(
        File::create(train_path).with_context(|| format!("cannot create {train_path:?}"))?,
    );
    let mut valid_writer = BufWriter::new(
        File::create(valid_path).with_context(|| format!("cannot create {valid_path:?}"))?,
    );
    let mut test_writer = BufWriter::new(
        File::create(test_path).with_context(|| format!("cannot create {test_path:?}"))?,
    );

    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut sources: HashMap<&'static str, HashSet<String>> = HashMap::new();
    let mut total = 0usize;

    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: PositionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(line = i + 1, "JSON parse error: {e}");
                continue;
            }
        };

        let bucket = assign_split_bucket(
            args.seed,
            split_root_key(&rec.source),
            args.valid_frac,
            args.test_frac,
        );
        let (name, writer) = match bucket {
            SplitBucket::Train => ("train", &mut train_writer),
            SplitBucket::Valid => ("valid", &mut valid_writer),
            SplitBucket::Test => ("test", &mut test_writer),
        };
        serde_json::to_writer(&mut *writer, &rec)?;
        writer.write_all(b"\n")?;
        *counts.entry(name).or_default() += 1;
        sources.entry(name).or_default().insert(rec.source.path);
        total += 1;
    }

    train_writer.flush()?;
    valid_writer.flush()?;
    test_writer.flush()?;

    let split_manifest = |name: &str, path: &PathBuf| {
        serde_json::json!({
            "path": path,
            "positions": counts.get(name).copied().unwrap_or(0),
            "sources": sources.get(name).map(HashSet::len).unwrap_or(0),
        })
    };
    let manifest = serde_json::json!({
        "shogiesa_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION,
        "input": args.input,
        "seed": args.seed,
        "requested": { "valid_frac": args.valid_frac, "test_frac": args.test_frac },
        "total_positions": total,
        "splits": {
            "train": split_manifest("train", train_path),
            "valid": split_manifest("valid", valid_path),
            "test": split_manifest("test", test_path),
        },
    });
    let manifest_path = train_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("manifest.json");
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)
        .with_context(|| format!("cannot write {manifest_path:?}"))?;

    eprintln!(
        "done: {total} positions split (seed={}) → train={}, valid={}, test={}",
        args.seed,
        counts.get("train").copied().unwrap_or(0),
        counts.get("valid").copied().unwrap_or(0),
        counts.get("test").copied().unwrap_or(0),
    );
    Ok(())
}

/// Opt-in run provenance, written when `--manifest PATH` is given. `input_hash` is a blake3
/// digest (same mechanism used everywhere else a persistent fingerprint is needed — see
/// `hash_parts`) — this is a "did the input change between runs" marker, not an integrity check
/// against untrusted input, but it still needs to be stable across Rust toolchain upgrades, which
/// is exactly what blake3 (and not `std::collections::hash_map::DefaultHasher`) guarantees.
/// `fingerprint_algorithm` records which algorithm produced `input_hash`, so a manifest from
/// before this field existed (and thus hashed with the old, toolchain-unstable `DefaultHasher`)
/// stays distinguishable from one produced after, rather than the two silently looking comparable.
#[derive(serde::Serialize)]
struct RunManifest {
    shogiesa_version: &'static str,
    git_sha: &'static str,
    schema_version: u32,
    pack_format_version: u16,
    command: &'static str,
    args: Vec<String>,
    input_path: String,
    input_hash: String,
    fingerprint_algorithm: &'static str,
    records_read: usize,
    records_kept: usize,
    /// Meaning is command-specific: parse-skips for `pack`, not-selected for `sample`/
    /// `balance`, gate-failures for `filter`. Disambiguate using `command`.
    records_dropped: usize,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    drop_reasons: BTreeMap<&'static str, usize>,
    labeled_records: usize,
    unlabeled_records: usize,
    observations_with_candidates: usize,
    observations_total: usize,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    score_bound_distribution: BTreeMap<&'static str, usize>,
    /// How many observations recorded a `requested_depth`, and how many of those fell short of
    /// it (excluding mate) — surfaces "how often does this engine/depth config under-deliver"
    /// across a whole run, not just per-record.
    requested_depth_total: usize,
    requested_depth_underreach: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter_config: Option<QualityConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    engine_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    depths: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    multipv: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    engine_options: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    jobs: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    engine_launch_failures: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_hits: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_misses: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    engine_fingerprint_mode: Option<&'static str>,
    /// `label`-only throughput diagnostics -- bounded pipeline changes (worker count, cache
    /// usage, ordering) can't be judged without measuring their actual effect, so this round
    /// surfaces the measurements instead of guessing. `jobs` above already *is* the worker count
    /// -- no separate field for that, to avoid two fields that could drift out of sync.
    #[serde(skip_serializing_if = "Option::is_none")]
    records_per_sec: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_hit_rate: Option<f64>,
    /// Average `Observation.time_ms` across every observation in each written record. Under
    /// `--skip-existing`/`--replace-existing`/the default `Append` policy, a record re-labeled on
    /// top of prior observations includes THOSE observations' `time_ms` too, not only this
    /// invocation's own engine calls -- getting that fully precise would mean threading "which
    /// observations this call actually added" back through `Job`/the worker/writer channels, not
    /// justified for a diagnostic average. Use `records_per_sec` to judge this run's actual
    /// throughput.
    #[serde(skip_serializing_if = "Option::is_none")]
    average_engine_time_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preserve_order: Option<bool>,
}

impl RunManifest {
    fn new(command: &'static str, input_path: &Path) -> Self {
        Self {
            shogiesa_version: env!("CARGO_PKG_VERSION"),
            git_sha: env!("SHOGIESA_GIT_SHA"),
            schema_version: SCHEMA_VERSION,
            pack_format_version: pack::FORMAT_VERSION,
            command,
            args: std::env::args().collect(),
            input_path: input_path.display().to_string(),
            input_hash: String::new(),
            fingerprint_algorithm: "blake3",
            records_read: 0,
            records_kept: 0,
            records_dropped: 0,
            drop_reasons: BTreeMap::new(),
            labeled_records: 0,
            unlabeled_records: 0,
            observations_with_candidates: 0,
            observations_total: 0,
            score_bound_distribution: BTreeMap::new(),
            requested_depth_total: 0,
            requested_depth_underreach: 0,
            filter_config: None,
            engine_name: None,
            depths: None,
            multipv: None,
            engine_options: None,
            jobs: None,
            engine_launch_failures: None,
            cache_hits: None,
            cache_misses: None,
            engine_fingerprint_mode: None,
            records_per_sec: None,
            cache_hit_rate: None,
            average_engine_time_ms: None,
            preserve_order: None,
        }
    }
}

fn write_manifest(path: &Path, manifest: &RunManifest) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(manifest)?)
        .with_context(|| format!("cannot write {path:?}"))
}

/// Hashes a file's lines (content + `\n` per line) with blake3. Only called when `--manifest` is
/// given, on commands that already fully materialize their input (`sample`/`balance`) — an extra
/// read is acceptable there since they aren't streaming to begin with (`label` accumulates its
/// own input hash while streaming instead of calling this, to avoid a redundant second read).
/// Hashes line-by-line to match `cmd_pack`/`cmd_filter`'s inline streaming hash exactly — so the
/// same input file gets the same `input_hash` in a manifest regardless of which command produced
/// it.
fn hash_file(path: &Path) -> Result<String> {
    let reader = BufReader::new(File::open(path).with_context(|| format!("cannot open {path:?}"))?);
    let mut h = blake3::Hasher::new();
    for line in reader.lines() {
        h.update(line?.as_bytes());
        h.update(b"\n");
    }
    Ok(h.finalize().to_hex().to_string())
}

/// String form of `ScoreBound`, shared by every stat-reporting call site (`report`, `calibrate`,
/// `audit`, `accumulate_candidate_coverage` below) that prints a score-bound distribution or a
/// single bound's label, so the string mapping can't drift between them.
fn score_bound_str(bound: shogiesa_core::ScoreBound) -> &'static str {
    match bound {
        shogiesa_core::ScoreBound::Exact => "exact",
        shogiesa_core::ScoreBound::Lowerbound => "lowerbound",
        shogiesa_core::ScoreBound::Upperbound => "upperbound",
    }
}

/// Tally labeled/unlabeled records, MultiPV candidate coverage, and score-bound distribution
/// across a batch of records — the same descriptive stats `report` computes ad hoc, shared here
/// for `RunManifest`. Not a quality *decision* (no pass/fail judgment), so it lives in the CLI
/// rather than `shogiesa_core::evaluate_quality`.
/// Tally MultiPV-candidate coverage and score-bound distribution across a batch of
/// observations. Shared by `accumulate_coverage` (manifests) and `cmd_report` (stdout) so the
/// `match c.score_bound { ... }` logic isn't duplicated.
fn accumulate_candidate_coverage(
    rec: &PositionRecord,
    with_candidates: &mut usize,
    total: &mut usize,
    score_bound_distribution: &mut BTreeMap<&'static str, usize>,
) {
    for obs in &rec.observations {
        *total += 1;
        if !obs.candidates.is_empty() {
            *with_candidates += 1;
            for c in &obs.candidates {
                let key = score_bound_str(c.score_bound);
                *score_bound_distribution.entry(key).or_default() += 1;
            }
        }
    }
}

fn candidate_coverage_stats(
    records: &[PositionRecord],
) -> (usize, usize, BTreeMap<&'static str, usize>) {
    let mut with_candidates = 0;
    let mut total = 0;
    let mut score_bound_distribution: BTreeMap<&'static str, usize> = BTreeMap::new();
    for rec in records {
        accumulate_candidate_coverage(
            rec,
            &mut with_candidates,
            &mut total,
            &mut score_bound_distribution,
        );
    }
    (with_candidates, total, score_bound_distribution)
}

/// Tally how many observations recorded a `requested_depth`, and how many of those under-reached
/// it (achieved `depth` below `requested_depth`, non-mate — mirrors
/// `evaluate_quality`'s `require_requested_depth_reached` gate). Shared by `report` and
/// manifests for the same reason `candidate_coverage_stats` is: one implementation, not two.
fn accumulate_requested_depth(
    rec: &PositionRecord,
    total_with_requested: &mut usize,
    underreach: &mut usize,
) {
    for obs in &rec.observations {
        if obs.requested_depth.is_some() {
            *total_with_requested += 1;
            if requested_depth_underreached(obs) {
                *underreach += 1;
            }
        }
    }
}

fn requested_depth_stats(records: &[PositionRecord]) -> (usize, usize) {
    let mut total_with_requested = 0usize;
    let mut underreach = 0usize;
    for rec in records {
        accumulate_requested_depth(rec, &mut total_with_requested, &mut underreach);
    }
    (total_with_requested, underreach)
}

fn accumulate_coverage(manifest: &mut RunManifest, records: &[PositionRecord]) {
    for rec in records {
        if rec.observations.is_empty() {
            manifest.unlabeled_records += 1;
        } else {
            manifest.labeled_records += 1;
        }
    }
    let (with_candidates, total, distribution) = candidate_coverage_stats(records);
    manifest.observations_with_candidates += with_candidates;
    manifest.observations_total += total;
    for (key, count) in distribution {
        *manifest.score_bound_distribution.entry(key).or_default() += count;
    }
    let (requested_total, underreach) = requested_depth_stats(records);
    manifest.requested_depth_total += requested_total;
    manifest.requested_depth_underreach += underreach;
}

/// Deterministic hash of `(seed, s)` — the same tie-breaking/spreading mechanism used by
/// `sample` (to pick which positions) and `select` (to break ties within a rank), so "pick N
/// deterministically" behaves identically wherever it's used.
fn seeded_hash(seed: u64, s: &str) -> u64 {
    let seed_bytes = seed.to_le_bytes();
    hash_parts_u64(&[&seed_bytes, s.as_bytes()])
}

/// One candidate in a bounded top-K stream. `key` carries every tie-break the equivalent
/// full-materialize-then-`sort_by` code applied (e.g. `(rank, seeded_hash)`); `index` is always
/// the final tiebreak layer, reproducing `sort_by`'s stability -- which a heap has no notion of on
/// its own, since two records can otherwise agree on every field `key` compares.
struct HeapEntry<K: Ord> {
    key: K,
    index: usize,
    record: PositionRecord,
}

impl<K: Ord> PartialEq for HeapEntry<K> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.index == other.index
    }
}
impl<K: Ord> Eq for HeapEntry<K> {}
impl<K: Ord> PartialOrd for HeapEntry<K> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<K: Ord> Ord for HeapEntry<K> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key
            .cmp(&other.key)
            .then_with(|| self.index.cmp(&other.index))
    }
}

/// Keeps the `count` smallest `HeapEntry`s seen so far -- the standard bounded-heap top-K
/// algorithm: push while under capacity, otherwise evict the current worst-kept entry if `entry`
/// ranks ahead of it. Provably identical final set (and, via `BinaryHeap::into_sorted_vec`,
/// identical order) to "collect everything, sort ascending by the same key, truncate" -- at
/// O(count) memory instead of O(n).
fn push_bounded<K: Ord>(heap: &mut BinaryHeap<HeapEntry<K>>, count: usize, entry: HeapEntry<K>) {
    if heap.len() < count {
        heap.push(entry);
    } else if heap.peek().is_some_and(|worst| entry.cmp(worst).is_lt()) {
        heap.pop();
        heap.push(entry);
    }
}

fn cmd_sample(args: SampleArgs) -> Result<()> {
    let seed = args.seed;
    let count = args.count;

    let reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    let mut total = 0usize;
    let mut heap: BinaryHeap<HeapEntry<u64>> = BinaryHeap::with_capacity(count.saturating_add(1));
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record: PositionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(line = i + 1, "parse error: {e}");
                continue;
            }
        };
        let key = seeded_hash(seed, &record.sfen);
        let index = total;
        total += 1;
        push_bounded(&mut heap, count, HeapEntry { key, index, record });
    }

    // Restore original file order (unlike `select`, which outputs in ranked order)
    let mut kept: Vec<HeapEntry<u64>> = heap.into_vec();
    kept.sort_by_key(|e| e.index);

    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);
    for entry in &kept {
        serde_json::to_writer(&mut writer, &entry.record)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    eprintln!(
        "done: {}/{total} sampled (seed={seed}) → {:?}",
        kept.len(),
        args.out
    );
    if let Some(manifest_path) = &args.manifest {
        let mut manifest = RunManifest::new("sample", &args.input);
        manifest.input_hash = hash_file(&args.input)?;
        manifest.records_read = total;
        manifest.records_kept = kept.len();
        manifest.records_dropped = total - kept.len();
        let kept_records: Vec<PositionRecord> = kept.iter().map(|e| e.record.clone()).collect();
        accumulate_coverage(&mut manifest, &kept_records);
        write_manifest(manifest_path, &manifest)?;
    }
    Ok(())
}

/// Black-perspective cp of a record's deepest observation. Thin wrapper over
/// `shogiesa_core::cp_from_black_perspective` -- kept here (rather than inlined at each call
/// site) since "pick the deepest observation, then convert" is a record-level operation the core
/// utility itself doesn't know about.
fn eval_black(rec: &PositionRecord) -> Option<i32> {
    rec.observations
        .iter()
        .max_by_key(|o| o.depth)
        .and_then(|o| match o.score {
            Score::Cp { value } => Some(cp_from_black_perspective(
                value,
                o.score_perspective,
                rec.tags.side_to_move,
            )),
            Score::Mate { .. } => None,
        })
}

/// Indices within `blunder_window` plies of a large eval swing (per source game, in ply order),
/// restricted to labeled positions. Shared by `mine` (its original purpose) and
/// `select --strategy hard` (one of several "worth a closer look" signals there), so the two
/// commands' definition of "blunder-adjacent" can't drift apart.
///
/// ponytail: both callers still fully materialize their input (`load_records`), unlike
/// `sample`/`select --strategy uncertain|coverage`/`balance`/`report`'s bounded-memory streaming.
/// This needs a whole game's positions grouped together, which isn't safe to stream without
/// assuming the input is contiguously grouped by source -- an assumption this codebase doesn't
/// guarantee elsewhere. Upgrade path if this specific command is shown to actually hit a memory
/// limit: require/verify source-contiguous input, then stream per-game windows instead.
fn blunder_adjacent_indices(
    records: &[PositionRecord],
    blunder_threshold: i32,
    blunder_window: usize,
) -> HashSet<usize> {
    let mut by_game: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, rec) in records.iter().enumerate() {
        by_game.entry(rec.source.path.clone()).or_default().push(i);
    }
    for indices in by_game.values_mut() {
        indices.sort_by_key(|&i| records[i].source.ply);
    }

    let mut keep = HashSet::<usize>::new();
    for indices in by_game.values() {
        let evals: Vec<Option<i32>> = indices.iter().map(|&i| eval_black(&records[i])).collect();
        for j in 1..indices.len() {
            if let (Some(e0), Some(e1)) = (evals[j - 1], evals[j])
                && (e1 - e0).abs() >= blunder_threshold
            {
                let lo = j.saturating_sub(blunder_window);
                let hi = (j + blunder_window + 1).min(indices.len());
                for k in lo..hi {
                    if !records[indices[k]].observations.is_empty() {
                        keep.insert(indices[k]);
                    }
                }
            }
        }
    }
    keep
}

fn cmd_mine(args: MineArgs) -> Result<()> {
    let (records, _) = load_records(&args.input)?;
    let total = records.len();

    let mut keep = blunder_adjacent_indices(&records, args.blunder_threshold, args.blunder_window);

    // Group indices by source game path, then sort each group by ply -- only needed below for
    // the losing-threshold pass, which blunder_adjacent_indices doesn't cover.
    let mut by_game: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, rec) in records.iter().enumerate() {
        by_game.entry(rec.source.path.clone()).or_default().push(i);
    }
    for indices in by_game.values_mut() {
        indices.sort_by_key(|&i| records[i].source.ply);
    }

    for indices in by_game.values() {
        let evals: Vec<Option<i32>> = indices.iter().map(|&i| eval_black(&records[i])).collect();

        // Losing positions: side to move's eval worse than -threshold
        if let Some(threshold) = args.losing_threshold {
            for (j, &idx) in indices.iter().enumerate() {
                if let Some(eval) = evals[j] {
                    let side_eval = match records[idx].tags.side_to_move {
                        SideToMove::Black => eval,
                        SideToMove::White => -eval,
                    };
                    if side_eval < -threshold && !records[idx].observations.is_empty() {
                        keep.insert(idx);
                    }
                }
            }
        }
    }

    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);
    let mut mined = 0usize;
    for (i, rec) in records.iter().enumerate() {
        if keep.contains(&i) {
            serde_json::to_writer(&mut writer, rec)?;
            writer.write_all(b"\n")?;
            mined += 1;
        }
    }
    writer.flush()?;
    eprintln!(
        "done: {mined}/{total} hard positions mined → {:?}",
        args.out
    );
    Ok(())
}

/// Composite phase/side/eval-bucket key for one record. Shared by `balance` (equal-count
/// rebalancing) and `select --strategy coverage` (thin-bucket prioritization) so the two
/// commands' notion of "bucket" can never drift apart.
fn bucket_key(rec: &PositionRecord, by_phase: bool, by_side: bool, by_eval: bool) -> String {
    let mut key = String::new();
    if by_phase {
        key.push_str(&format!("{}:", rec.tags.phase));
    }
    if by_side {
        key.push_str(&format!("{}:", rec.tags.side_to_move));
    }
    if by_eval {
        let bucket_str = rec
            .observations
            .iter()
            .max_by_key(|o| o.depth)
            .map(|o| match o.score {
                Score::Cp { value } => {
                    let black_value = cp_from_black_perspective(
                        value,
                        o.score_perspective,
                        rec.tags.side_to_move,
                    );
                    format!("{}:", (black_value.div_euclid(200)) * 200)
                }
                Score::Mate { .. } => "mate:".to_string(),
            })
            .unwrap_or_else(|| "_none_:".to_string());
        key.push_str(&bucket_str);
    }
    key
}

fn cmd_balance(args: BalanceArgs) -> Result<()> {
    if args.by.is_empty() {
        anyhow::bail!("--by requires at least one of: phase, side, eval-bucket");
    }
    let by_phase = args.by.iter().any(|s| s == "phase");
    let by_side = args.by.iter().any(|s| s == "side");
    let by_eval = args.by.iter().any(|s| s == "eval-bucket");

    // Pass 1: tally each bucket's size -- needed before any record can be ranked, since `target`
    // defaults to the smallest bucket's size, which can't be known until every bucket has been
    // seen at least once.
    let mut bucket_sizes: HashMap<String, usize> = HashMap::new();
    let pass1 = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    for line in pass1.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        // parse errors are warned once, in pass 2 below -- not duplicated here
        if let Ok(record) = serde_json::from_str::<PositionRecord>(&line) {
            *bucket_sizes
                .entry(bucket_key(&record, by_phase, by_side, by_eval))
                .or_default() += 1;
        }
    }
    let min_size = bucket_sizes.values().copied().min().unwrap_or(0);
    let target = args.target.unwrap_or(min_size);

    // Pass 2: re-stream, keeping a bounded top-`target` heap per bucket, keyed by SFEN (matching
    // today's "sort by SFEN, take first N" exactly) -- memory O(bucket count x target) instead of
    // O(n).
    let pass2 = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    let mut total = 0usize;
    let mut heaps: HashMap<String, BinaryHeap<HeapEntry<String>>> = HashMap::new();
    for (i, line) in pass2.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record: PositionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(line = i + 1, "parse error: {e}");
                continue;
            }
        };
        let bucket = bucket_key(&record, by_phase, by_side, by_eval);
        let key = record.sfen.clone();
        let index = total;
        total += 1;
        push_bounded(
            heaps.entry(bucket).or_default(),
            target,
            HeapEntry { key, index, record },
        );
    }

    // Restore original file order (unlike `select`, which outputs in ranked order)
    let mut kept: Vec<HeapEntry<String>> = heaps.into_values().flat_map(|h| h.into_vec()).collect();
    kept.sort_by_key(|e| e.index);

    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);
    for entry in &kept {
        serde_json::to_writer(&mut writer, &entry.record)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    eprintln!(
        "done: {}/{total} selected (target {target}/bucket, {} buckets) → {:?}",
        kept.len(),
        bucket_sizes.len(),
        args.out
    );
    if let Some(manifest_path) = &args.manifest {
        let mut manifest = RunManifest::new("balance", &args.input);
        manifest.input_hash = hash_file(&args.input)?;
        manifest.records_read = total;
        manifest.records_kept = kept.len();
        manifest.records_dropped = total - kept.len();
        let kept_records: Vec<PositionRecord> = kept.iter().map(|e| e.record.clone()).collect();
        accumulate_coverage(&mut manifest, &kept_records);
        write_manifest(manifest_path, &manifest)?;
    }
    Ok(())
}

// Why `select` exists at all: re-labeling an entire dataset at higher depth to chase accuracy
// costs the same whether 1% or 100% of it is actually wrong or uninformative. Each strategy
// below ranks positions by a signal that predicts "worth a second look" and reuses the exact
// judgment logic `filter`/`mine`/`balance` already have (evaluate_quality, blunder-window
// detection, bucket keys) rather than re-deriving what "uncertain"/"hard"/"thin" means.
/// `f32` wrapper with a total order (via `total_cmp`), scoped to `select --strategy uncertain`'s
/// heap key -- `evaluate_quality`'s score is a plain 0..=1 fraction, never NaN in practice, but
/// `f32` has no `Ord` impl at all, so a small local wrapper is needed regardless of that.
#[derive(PartialEq)]
struct TotalF32(f32);
impl Eq for TotalF32 {}
impl PartialOrd for TotalF32 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TotalF32 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}

/// `select --strategy uncertain`: streams the input once, keeping a bounded top-K heap of the
/// `count` worst `evaluate_quality` scores. Key = `(score, seeded_hash)`, matching the
/// full-materialize-then-`sort_by` ordering this replaces exactly (ascending: worst first).
fn select_uncertain_streaming(args: &SelectArgs) -> Result<(usize, Vec<PositionRecord>)> {
    // No arbitrary thresholds: every gate here is either a plain existence check or, for depth,
    // requested_depth-vs-achieved (self-referential, no floor to pick) -- require_engine_agreement
    // stands in for the spec's "engine_disagreement" signal. --min-policy-margin-cp is the one
    // optional, user-supplied threshold, mirroring `filter`'s flag of the same name instead of
    // inventing a default.
    let config = QualityConfig {
        require_exact_score: true,
        require_policy_margin: true,
        require_requested_depth_reached: true,
        require_engine_agreement: true,
        min_policy_margin_cp: args.min_policy_margin_cp,
        ..Default::default()
    };
    let count = args.count;
    let seed = args.seed;
    let reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    let mut total = 0usize;
    let mut heap: BinaryHeap<HeapEntry<(TotalF32, u64)>> =
        BinaryHeap::with_capacity(count.saturating_add(1));
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record: PositionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(line = i + 1, "parse error: {e}");
                continue;
            }
        };
        // decision.score is evaluate_quality's own "fraction of gates passed" -- reused directly
        // as the ranking key instead of re-deriving a severity score from reasons.
        let score = evaluate_quality(&record, &config).score;
        let key = (TotalF32(score), seeded_hash(seed, &record.sfen));
        let index = total;
        total += 1;
        push_bounded(&mut heap, count, HeapEntry { key, index, record });
    }
    let ranked_records = heap
        .into_sorted_vec()
        .into_iter()
        .map(|e| e.record)
        .collect();
    Ok((total, ranked_records))
}

/// `select --strategy coverage`: two streaming passes over the input -- pass 1 tallies
/// `bucket_key → count` (needed before any record can be ranked, since the signal is "how common
/// is this record's bucket"); pass 2 streams again, keeping a bounded top-K heap keyed by
/// `(bucket_count, seeded_hash)` (ascending: thinnest bucket first). 2x I/O, O(bucket count +
/// `count`) memory, in exchange for not materializing the whole dataset.
fn select_coverage_streaming(args: &SelectArgs) -> Result<(usize, Vec<PositionRecord>)> {
    let count = args.count;
    let seed = args.seed;

    let mut bucket_counts: HashMap<String, usize> = HashMap::new();
    let pass1 = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    for line in pass1.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        // parse errors are warned once, in pass 2 below -- not duplicated here
        if let Ok(record) = serde_json::from_str::<PositionRecord>(&line) {
            *bucket_counts
                .entry(bucket_key(&record, true, true, true))
                .or_default() += 1;
        }
    }

    let pass2 = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    let mut total = 0usize;
    let mut heap: BinaryHeap<HeapEntry<(usize, u64)>> =
        BinaryHeap::with_capacity(count.saturating_add(1));
    for (i, line) in pass2.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record: PositionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(line = i + 1, "parse error: {e}");
                continue;
            }
        };
        let bucket_count = bucket_counts[&bucket_key(&record, true, true, true)];
        let key = (bucket_count, seeded_hash(seed, &record.sfen));
        let index = total;
        total += 1;
        push_bounded(&mut heap, count, HeapEntry { key, index, record });
    }
    let ranked_records = heap
        .into_sorted_vec()
        .into_iter()
        .map(|e| e.record)
        .collect();
    Ok((total, ranked_records))
}

/// `select --strategy hard`: unlike `uncertain`/`coverage` above, left fully materialized.
/// `blunder_adjacent_indices` fundamentally needs a whole game's positions grouped together, which
/// isn't safe to stream without assuming the input is contiguously grouped by source (an
/// assumption this codebase doesn't guarantee elsewhere) -- so this stays O(n) memory.
/// ponytail: revisit only if `hard` is shown to actually hit a memory limit in practice.
fn select_hard_materialized(args: &SelectArgs) -> Result<(usize, Vec<PositionRecord>)> {
    let (records, _) = load_records(&args.input)?;
    let total = records.len();
    let count = args.count.min(total);
    let seed = args.seed;

    let blunder_set =
        blunder_adjacent_indices(&records, args.blunder_threshold, args.blunder_window);
    let hardness = |i: usize| -> (bool, bool, i32) {
        let rec = &records[i];
        let cp_scores: Vec<i32> = rec
            .observations
            .iter()
            .filter_map(|o| match o.score {
                Score::Cp { value } => Some(value),
                Score::Mate { .. } => None,
            })
            .collect();
        let swing = score_swing(&cp_scores).unwrap_or(0);
        let disagreement = !bestmove_agreement(&rec.observations);
        (blunder_set.contains(&i), disagreement, swing)
    };
    let mut ranked: Vec<usize> = (0..total).collect();
    ranked.sort_by(|&a, &b| {
        hardness(b).cmp(&hardness(a)).then_with(|| {
            seeded_hash(seed, &records[a].sfen).cmp(&seeded_hash(seed, &records[b].sfen))
        })
    });
    ranked.truncate(count);
    let ranked_records = ranked.into_iter().map(|i| records[i].clone()).collect();
    Ok((total, ranked_records))
}

fn cmd_select(args: SelectArgs) -> Result<()> {
    let (total, ranked_records) = match args.strategy.as_str() {
        "uncertain" => select_uncertain_streaming(&args)?,
        "coverage" => select_coverage_streaming(&args)?,
        "hard" => select_hard_materialized(&args)?,
        other => anyhow::bail!("unknown --strategy {other:?} (expected uncertain/hard/coverage)"),
    };

    // Output in ranked order (most-worth-a-look first), unlike `sample`/`balance` which restore
    // input order -- a re-labeling queue is more useful read top-to-bottom by priority than by
    // original file position.
    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);
    for rec in &ranked_records {
        serde_json::to_writer(&mut writer, rec)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;
    eprintln!(
        "done: {}/{total} selected (strategy={}, seed={}) → {:?}",
        ranked_records.len(),
        args.strategy,
        args.seed,
        args.out
    );
    Ok(())
}

fn cmd_pack(args: PackArgs) -> Result<()> {
    let reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);

    pack::write_header(&mut writer)?;

    let mut total = 0usize;
    let mut skipped = 0usize;
    let mut manifest = args
        .manifest
        .is_some()
        .then(|| RunManifest::new("pack", &args.input));
    let mut input_hasher = blake3::Hasher::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if manifest.is_some() {
            input_hasher.update(line.as_bytes());
            input_hasher.update(b"\n");
        }
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<PositionRecord>(&line) {
            Ok(rec) => {
                pack::encode_record(&rec, &mut writer)?;
                total += 1;
                if let Some(m) = &mut manifest {
                    accumulate_coverage(m, std::slice::from_ref(&rec));
                }
            }
            Err(e) => {
                tracing::warn!(line = i + 1, "JSON parse error: {e}");
                skipped += 1;
            }
        }
    }
    writer.flush()?;
    eprintln!("done: {total} packed, {skipped} skipped → {:?}", args.out);
    if let (Some(mut m), Some(manifest_path)) = (manifest, &args.manifest) {
        m.input_hash = input_hasher.finalize().to_hex().to_string();
        m.records_read = total + skipped;
        m.records_kept = total;
        m.records_dropped = skipped;
        write_manifest(manifest_path, &m)?;
    }
    Ok(())
}

fn cmd_unpack(args: UnpackArgs) -> Result<()> {
    let mut reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);

    pack::read_header(&mut reader)?;

    let mut total = 0usize;
    loop {
        match pack::decode_record(&mut reader) {
            Ok(rec) => {
                serde_json::to_writer(&mut writer, &rec)?;
                writer.write_all(b"\n")?;
                total += 1;
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
    }
    writer.flush()?;
    eprintln!("done: {total} unpacked → {:?}", args.out);
    Ok(())
}

/// Parses `--phase`'s comma-separated `opening,middlegame,endgame` list into `QualityConfig`'s
/// `allowed_phases` -- shared by `filter` and `calibrate` so both interpret the same flag
/// identically instead of keeping two copies of the same match arms.
fn parse_allowed_phases(phase: Option<&str>) -> Option<Vec<GamePhase>> {
    phase.map(|s| {
        s.split(',')
            .filter_map(|p| match p.trim() {
                "opening" => Some(GamePhase::Opening),
                "middlegame" => Some(GamePhase::Middlegame),
                "endgame" => Some(GamePhase::Endgame),
                other => {
                    tracing::warn!("unknown phase {other:?}, ignoring");
                    None
                }
            })
            .collect()
    })
}

fn build_quality_config(
    args: &FilterArgs,
    allowed_phases: Option<Vec<GamePhase>>,
) -> QualityConfig {
    QualityConfig {
        min_observations: args.min_observations,
        allowed_phases,
        exclude_mate: args.exclude_mate,
        exclude_in_check: args.exclude_in_check,
        exclude_capture: args.exclude_capture,
        eval_min: args.eval_min,
        eval_max: args.eval_max,
        max_score_swing_cp: args.max_score_swing_cp,
        min_policy_margin_cp: args.min_policy_margin_cp,
        require_bestmove_agreement: args.require_bestmove_agreement,
        require_engine_agreement: args.require_engine_agreement,
        max_engine_score_swing_cp: args.max_engine_score_swing_cp,
        require_exact_score: args.require_exact_score,
        require_policy_margin: args.require_policy_margin,
        min_depth_reached: args.min_depth_reached,
        require_requested_depth_reached: args.require_requested_depth_reached,
    }
}

/// Loads one named candidate's `QualityConfig` out of a `tune --preset-out` JSON file.
/// `rsplit_once(':')` splits at the *last* colon, which correctly handles a Windows-style
/// `C:\...` path (the drive letter's colon is never the last one when a label follows).
fn load_quality_config_preset(spec: &str) -> Result<QualityConfig> {
    let (path_str, label) = spec
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("--preset must be FILE.json:label, got {spec:?}"))?;
    let text = fs::read_to_string(path_str)
        .with_context(|| format!("cannot read preset file {path_str:?}"))?;
    let preset_file: TunePresetFile = serde_json::from_str(&text)
        .with_context(|| format!("cannot parse preset file {path_str:?}"))?;
    let candidate = preset_file.presets.get(label).ok_or_else(|| {
        anyhow::anyhow!(
            "preset {label:?} not found in {path_str:?}; available: {:?}",
            preset_file.presets.keys().collect::<Vec<_>>()
        )
    })?;
    Ok(candidate.config.clone())
}

fn cmd_filter(args: FilterArgs) -> Result<()> {
    let config = match &args.preset {
        Some(spec) => load_quality_config_preset(spec)?,
        None => {
            let allowed_phases = parse_allowed_phases(args.phase.as_deref());
            build_quality_config(&args, allowed_phases)
        }
    };

    let reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    let mut writer = match &args.out {
        Some(out) => Some(BufWriter::new(
            File::create(out).with_context(|| format!("cannot create {out:?}"))?,
        )),
        None => None,
    };
    let mut explain_writer = match &args.explain_out {
        Some(path) => Some(BufWriter::new(
            File::create(path).with_context(|| format!("cannot create {path:?}"))?,
        )),
        None => None,
    };

    let mut total = 0usize;
    let mut passed = 0usize;
    let mut skipped = 0usize;
    let mut drop_reasons: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut manifest = args
        .manifest
        .is_some()
        .then(|| RunManifest::new("filter", &args.input));
    let mut input_hasher = blake3::Hasher::new();

    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if manifest.is_some() {
            input_hasher.update(line.as_bytes());
            input_hasher.update(b"\n");
        }
        if line.trim().is_empty() {
            continue;
        }
        total += 1;

        let rec: PositionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(line = i + 1, "JSON parse error: {e}");
                skipped += 1;
                *drop_reasons.entry("parse_error").or_default() += 1;
                continue;
            }
        };

        let decision = evaluate_quality(&rec, &config);
        match decision.reasons.first() {
            None => {
                if let Some(w) = &mut writer {
                    serde_json::to_writer(&mut *w, &rec)?;
                    w.write_all(b"\n")?;
                }
                passed += 1;
                if let Some(m) = &mut manifest {
                    accumulate_coverage(m, std::slice::from_ref(&rec));
                }
            }
            Some(reason) => {
                skipped += 1;
                *drop_reasons.entry(reason.as_str()).or_default() += 1;
                if let Some(w) = &mut explain_writer {
                    serde_json::to_writer(
                        &mut *w,
                        &serde_json::json!({"record": &rec, "quality": &decision}),
                    )?;
                    w.write_all(b"\n")?;
                }
            }
        }
    }

    if let Some(w) = &mut writer {
        w.flush()?;
    }
    if let Some(w) = &mut explain_writer {
        w.flush()?;
    }
    match &args.out {
        Some(out) => eprintln!("done: {total} read, {passed} passed, {skipped} filtered → {out:?}"),
        None => eprintln!("done (dry run): {total} read, {passed} passed, {skipped} filtered"),
    }
    if !drop_reasons.is_empty() {
        eprintln!("drop reasons:");
        for (reason, count) in &drop_reasons {
            eprintln!("  {reason:<24} {count}");
        }
    }
    if let (Some(mut m), Some(manifest_path)) = (manifest, &args.manifest) {
        m.input_hash = input_hasher.finalize().to_hex().to_string();
        m.records_read = total;
        m.records_kept = passed;
        m.records_dropped = skipped;
        m.drop_reasons = drop_reasons;
        m.filter_config = Some(config);
        write_manifest(manifest_path, &m)?;
    }
    Ok(())
}

/// Tallies how many records pass/fail `evaluate_quality` under one `QualityConfig` -- shared by
/// `SweepRow` (one swept threshold value, `calibrate`) and `TuneCell` (one grid cell of two
/// combined thresholds, `tune`) so neither command reimplements the same three-line bookkeeping.
#[derive(Default)]
struct CoverageTally {
    total: usize,
    kept: usize,
    dropped: usize,
    /// First-failing-reason-only, matching `filter`'s and core's own documented convention (see
    /// `evaluate_quality`'s doc comment) -- keeps `drop_reasons` values summing to `dropped`.
    drop_reasons: BTreeMap<&'static str, usize>,
}

impl CoverageTally {
    fn record(&mut self, decision: &QualityDecision) {
        self.total += 1;
        match decision.reasons.first() {
            None => self.kept += 1,
            Some(reason) => {
                self.dropped += 1;
                *self.drop_reasons.entry(reason.as_str()).or_default() += 1;
            }
        }
    }

    fn coverage_pct(&self) -> f64 {
        if self.total > 0 {
            self.kept as f64 / self.total as f64 * 100.0
        } else {
            0.0
        }
    }

    /// `;`-joined `reason=count` pairs, deterministic order (`BTreeMap`) -- the CSV cell format
    /// both `calibrate` and `tune` write.
    fn drop_reasons_csv_cell(&self) -> String {
        self.drop_reasons
            .iter()
            .map(|(r, c)| format!("{r}={c}"))
            .collect::<Vec<_>>()
            .join(";")
    }
}

/// One swept threshold value's outcome under `evaluate_quality` -- the base config plus this one
/// field overridden to `value`, everything else held fixed. Lets `calibrate` show "what does
/// raising this specific gate do to coverage" in isolation, reusing `evaluate_quality` itself
/// rather than re-deriving its judgment logic.
struct SweepRow {
    param: &'static str,
    value: i32,
    coverage: CoverageTally,
}

impl SweepRow {
    fn new(param: &'static str, value: i32) -> Self {
        Self {
            param,
            value,
            coverage: CoverageTally::default(),
        }
    }
}

fn parse_int_list(s: &str) -> Result<Vec<i32>> {
    s.split(',')
        .map(|p| {
            p.trim()
                .parse::<i32>()
                .with_context(|| format!("invalid integer {p:?} in sweep list"))
        })
        .collect()
}

/// Dataset-wide diagnostics independent of any swept/gridded threshold -- accumulated once per
/// record regardless of how many `QualityConfig` variants are being evaluated against it (not
/// duplicated per sweep row/grid cell, which would wrongly imply they vary by threshold), then
/// printed once. Shared by `calibrate` and `tune` so this bookkeeping can't diverge between them.
#[derive(Default)]
struct DatasetDiagnostics {
    labeled: usize,
    special_bestmove: usize,
    // 50cp buckets, same convention as `report`'s swing/eval histograms -- bucket-and-count, not
    // percentiles.
    margin_buckets: BTreeMap<i32, usize>,
    swing_buckets: BTreeMap<i32, usize>,
    obs_score_bound_counts: BTreeMap<&'static str, usize>,
    requested_depth_total: usize,
    requested_depth_underreach: usize,
}

impl DatasetDiagnostics {
    fn record(&mut self, rec: &PositionRecord) {
        if rec.observations.is_empty() {
            return;
        }
        self.labeled += 1;
        if has_special_bestmove(&rec.observations) {
            self.special_bestmove += 1;
        }
        let mut cp_scores = Vec::new();
        for obs in &rec.observations {
            if let Some(margin) = obs.policy_margin_cp {
                *self
                    .margin_buckets
                    .entry((margin.div_euclid(50)) * 50)
                    .or_default() += 1;
            }
            if let Score::Cp { value } = obs.score {
                cp_scores.push(value);
            }
            let bound_key = score_bound_str(obs.score_bound);
            *self.obs_score_bound_counts.entry(bound_key).or_default() += 1;
        }
        if let Some(swing) = score_swing(&cp_scores) {
            *self
                .swing_buckets
                .entry((swing.div_euclid(50)) * 50)
                .or_default() += 1;
        }
        accumulate_requested_depth(
            rec,
            &mut self.requested_depth_total,
            &mut self.requested_depth_underreach,
        );
    }

    fn print(&self) {
        // Destructured (not `self.field` inline) so the `{name:>6}`-style inline-capture format
        // strings below are byte-identical to before this was extracted out of `cmd_calibrate`.
        let DatasetDiagnostics {
            labeled,
            special_bestmove,
            margin_buckets,
            swing_buckets,
            obs_score_bound_counts,
            requested_depth_total,
            requested_depth_underreach,
        } = self;
        eprintln!("dataset-wide diagnostics (independent of swept thresholds):");
        eprintln!(
            "  special bestmove: {special_bestmove:>6}  ({:.1}% of labeled)",
            *special_bestmove as f64 / (*labeled).max(1) as f64 * 100.0
        );
        if *requested_depth_total > 0 {
            eprintln!(
                "  requested-depth underreach: {requested_depth_underreach:>6}  ({:.1}% of {requested_depth_total} observations with a requested_depth)",
                *requested_depth_underreach as f64 / *requested_depth_total as f64 * 100.0
            );
        }
        if !obs_score_bound_counts.is_empty() {
            eprintln!("  score bound (observations):");
            for (bound, count) in obs_score_bound_counts {
                eprintln!("    {bound:<10} : {count:>6}");
            }
        }
        if !margin_buckets.is_empty() {
            eprintln!("  policy_margin_cp distribution (50cp buckets):");
            for (&key, &count) in margin_buckets {
                eprintln!("    {key:>5}..{:<4}: {count:>6}", key + 49);
            }
        }
        if !swing_buckets.is_empty() {
            eprintln!("  score_swing_cp distribution (50cp buckets, per record):");
            for (&key, &count) in swing_buckets {
                eprintln!("    {key:>5}..{:<4}: {count:>6}", key + 49);
            }
        }
    }
}

fn cmd_calibrate(args: CalibrateArgs) -> Result<()> {
    if args.sweep_policy_margin.is_none() && args.sweep_score_swing.is_none() {
        anyhow::bail!(
            "calibrate requires at least one of --sweep-policy-margin/--sweep-score-swing"
        );
    }

    let allowed_phases = parse_allowed_phases(args.phase.as_deref());
    // Why the held values feed straight into the base config: `--sweep-x`/its `--min-x`(or
    // `--max-x`) hold counterpart are a clap `conflicts_with` pair, so at most one of "sweep this
    // field" and "hold this field at a fixed value" is ever active for a given field -- the base
    // config's value for a field currently being swept is always `None` here (overridden per
    // sweep value in the loop below), and `Some` only when that field is instead held fixed while
    // the OTHER dimension sweeps.
    let base_config = QualityConfig {
        min_observations: args.min_observations,
        allowed_phases,
        exclude_mate: args.exclude_mate,
        exclude_in_check: args.exclude_in_check,
        exclude_capture: args.exclude_capture,
        eval_min: args.eval_min,
        eval_max: args.eval_max,
        max_score_swing_cp: args.max_score_swing_cp,
        min_policy_margin_cp: args.min_policy_margin_cp,
        require_bestmove_agreement: args.require_bestmove_agreement,
        require_engine_agreement: args.require_engine_agreement,
        max_engine_score_swing_cp: args.max_engine_score_swing_cp,
        require_exact_score: args.require_exact_score,
        require_policy_margin: args.require_policy_margin,
        min_depth_reached: args.min_depth_reached,
        require_requested_depth_reached: args.require_requested_depth_reached,
    };

    let mut policy_margin_rows: Vec<SweepRow> = match &args.sweep_policy_margin {
        Some(s) => parse_int_list(s)?
            .into_iter()
            .map(|v| SweepRow::new("policy_margin", v))
            .collect(),
        None => Vec::new(),
    };
    let mut score_swing_rows: Vec<SweepRow> = match &args.sweep_score_swing {
        Some(s) => parse_int_list(s)?
            .into_iter()
            .map(|v| SweepRow::new("score_swing", v))
            .collect(),
        None => Vec::new(),
    };

    let reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );

    let mut total = 0usize;
    let mut diagnostics = DatasetDiagnostics::default();

    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: PositionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(line = i + 1, "JSON parse error: {e}");
                continue;
            }
        };
        total += 1;

        // Each swept value gets its own evaluate_quality call, overriding only the one field this
        // sweep varies -- everything else (including the OTHER dimension, if held via --min-x/
        // --max-x) stays at the base config.
        for row in &mut policy_margin_rows {
            let config = QualityConfig {
                min_policy_margin_cp: Some(row.value),
                ..base_config.clone()
            };
            row.coverage.record(&evaluate_quality(&rec, &config));
        }
        for row in &mut score_swing_rows {
            let config = QualityConfig {
                max_score_swing_cp: Some(row.value),
                ..base_config.clone()
            };
            row.coverage.record(&evaluate_quality(&rec, &config));
        }

        // Sweep-independent, so accumulated once per record regardless of threshold, not
        // duplicated per sweep row (which would wrongly imply it varies by threshold).
        diagnostics.record(&rec);
    }

    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);
    writeln!(
        writer,
        "sweep_param,sweep_value,total,kept,dropped,coverage_pct,drop_reasons"
    )?;
    for row in policy_margin_rows.iter().chain(score_swing_rows.iter()) {
        writeln!(
            writer,
            "{},{},{},{},{},{:.2},{}",
            row.param,
            row.value,
            row.coverage.total,
            row.coverage.kept,
            row.coverage.dropped,
            row.coverage.coverage_pct(),
            row.coverage.drop_reasons_csv_cell()
        )?;
    }
    writer.flush()?;

    eprintln!(
        "done: {total} read, {} labeled → {:?}",
        diagnostics.labeled, args.out
    );
    diagnostics.print();

    Ok(())
}

/// Finds the observation (already filtered to one engine) matching `target_depth` -- primary
/// match on `requested_depth` (what `label` was actually asked to reach), falling back to the
/// achieved `depth` for legacy pre-schema-v6 data (or any observation whose recorded
/// `requested_depth` simply doesn't match `target_depth`).
fn find_at_depth<'a>(
    observations: &[&'a Observation],
    target_depth: u32,
) -> Option<&'a Observation> {
    observations
        .iter()
        .find(|o| o.requested_depth == Some(target_depth))
        .or_else(|| observations.iter().find(|o| o.depth == target_depth))
        .copied()
}

fn parse_u32_list(s: &str) -> Result<Vec<u32>> {
    s.split(',')
        .map(|p| {
            p.trim()
                .parse::<u32>()
                .with_context(|| format!("invalid unsigned integer {p:?} in depth list"))
        })
        .collect()
}

/// Aggregate stats for one student depth (or overall, across every student depth) -- shared
/// accumulation so `cmd_audit`'s per-depth and overall summaries can't diverge.
#[derive(Default)]
struct AuditStats {
    pairs: usize,
    bestmove_mismatches: usize,
    abs_score_error_sum: f64,
    abs_score_error_count: usize,
    abs_score_error_max: i32,
    teacher_non_exact: usize,
    student_non_exact: usize,
    teacher_underreach: usize,
    student_underreach: usize,
    teacher_special_bestmove: usize,
    student_special_bestmove: usize,
}

impl AuditStats {
    fn record(
        &mut self,
        bestmove_match: bool,
        score_error_cp: Option<i32>,
        teacher: &Observation,
        student: &Observation,
    ) {
        self.pairs += 1;
        if !bestmove_match {
            self.bestmove_mismatches += 1;
        }
        if let Some(err) = score_error_cp {
            let abs_err = err.abs();
            self.abs_score_error_sum += abs_err as f64;
            self.abs_score_error_count += 1;
            self.abs_score_error_max = self.abs_score_error_max.max(abs_err);
        }
        if teacher.score_bound != shogiesa_core::ScoreBound::Exact {
            self.teacher_non_exact += 1;
        }
        if student.score_bound != shogiesa_core::ScoreBound::Exact {
            self.student_non_exact += 1;
        }
        if requested_depth_underreached(teacher) {
            self.teacher_underreach += 1;
        }
        if requested_depth_underreached(student) {
            self.student_underreach += 1;
        }
        if effective_bestmove_kind(teacher).is_some() {
            self.teacher_special_bestmove += 1;
        }
        if effective_bestmove_kind(student).is_some() {
            self.student_special_bestmove += 1;
        }
    }

    fn print(&self, label: &str) {
        if self.pairs == 0 {
            return;
        }
        eprintln!("  {label}:");
        eprintln!("    pairs compared      : {:>6}", self.pairs);
        eprintln!(
            "    bestmove mismatch   : {:>6}  ({:.1}%)",
            self.bestmove_mismatches,
            self.bestmove_mismatches as f64 / self.pairs as f64 * 100.0
        );
        if self.abs_score_error_count > 0 {
            eprintln!(
                "    avg |score error|   : {:.1}cp  (max {}cp, over {} mate-free pairs)",
                self.abs_score_error_sum / self.abs_score_error_count as f64,
                self.abs_score_error_max,
                self.abs_score_error_count
            );
        }
        eprintln!(
            "    teacher non-exact   : {:>6}  ({:.1}%)",
            self.teacher_non_exact,
            self.teacher_non_exact as f64 / self.pairs as f64 * 100.0
        );
        eprintln!(
            "    student non-exact   : {:>6}  ({:.1}%)",
            self.student_non_exact,
            self.student_non_exact as f64 / self.pairs as f64 * 100.0
        );
        eprintln!(
            "    teacher underreach  : {:>6}  ({:.1}%)",
            self.teacher_underreach,
            self.teacher_underreach as f64 / self.pairs as f64 * 100.0
        );
        eprintln!(
            "    student underreach  : {:>6}  ({:.1}%)",
            self.student_underreach,
            self.student_underreach as f64 / self.pairs as f64 * 100.0
        );
        eprintln!(
            "    teacher special bm  : {:>6}  ({:.1}%)",
            self.teacher_special_bestmove,
            self.teacher_special_bestmove as f64 / self.pairs as f64 * 100.0
        );
        eprintln!(
            "    student special bm  : {:>6}  ({:.1}%)",
            self.student_special_bestmove,
            self.student_special_bestmove as f64 / self.pairs as f64 * 100.0
        );
    }
}

/// Every (student_depth, bestmove_match, score_error_cp, teacher, student) comparison this
/// record's engine-grouped observations produce against `teacher_depth` -- the same
/// engine-grouping (a dataset labeled by 2+ engines must never compare engine A's shallow
/// observation against engine B's deep one), `find_at_depth` matching, and
/// `cp_from_black_perspective` normalization `cmd_audit` always used, extracted so `cmd_tune` can
/// fold the SAME comparisons into every grid cell without recomputing them per cell (the
/// comparison itself doesn't depend on any quality-gate threshold; only which cells count it
/// toward "kept" does).
fn compute_audit_pairs<'a>(
    rec: &'a PositionRecord,
    teacher_depth: u32,
    student_depths: &[u32],
) -> Vec<(u32, bool, Option<i32>, &'a Observation, &'a Observation)> {
    let mut pairs = Vec::new();
    let mut by_engine: HashMap<&str, Vec<&Observation>> = HashMap::new();
    for obs in &rec.observations {
        by_engine.entry(obs.engine.as_str()).or_default().push(obs);
    }

    for observations in by_engine.values() {
        let Some(teacher) = find_at_depth(observations, teacher_depth) else {
            continue;
        };
        for &student_depth in student_depths {
            // Comparing the teacher depth against itself (if it's also listed as a student depth)
            // is degenerate -- always a "match" with zero error, not a real comparison.
            if student_depth == teacher_depth {
                continue;
            }
            let Some(student) = find_at_depth(observations, student_depth) else {
                continue;
            };

            let bestmove_match = bestmove_agreement(&[teacher.clone(), student.clone()]);
            // Normalize both sides through the record's shared side_to_move rather than assuming
            // they already share ScorePerspective::SideToMove -- correct by construction, not by
            // coincidence. None (not compared) if either side is mate.
            let score_error_cp = match (&teacher.score, &student.score) {
                (Score::Cp { value: t }, Score::Cp { value: s }) => {
                    let t_black = cp_from_black_perspective(
                        *t,
                        teacher.score_perspective,
                        rec.tags.side_to_move,
                    );
                    let s_black = cp_from_black_perspective(
                        *s,
                        student.score_perspective,
                        rec.tags.side_to_move,
                    );
                    Some(t_black - s_black)
                }
                _ => None,
            };
            pairs.push((
                student_depth,
                bestmove_match,
                score_error_cp,
                teacher,
                student,
            ));
        }
    }
    pairs
}

fn cmd_audit(args: AuditArgs) -> Result<()> {
    let student_depths = parse_u32_list(&args.student_depths)?;

    let reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);

    let mut total_records = 0usize;
    let mut overall = AuditStats::default();
    let mut per_depth: BTreeMap<u32, AuditStats> = BTreeMap::new();

    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: PositionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(line = i + 1, "parse error: {e}");
                continue;
            }
        };
        total_records += 1;

        for (student_depth, bestmove_match, score_error_cp, teacher, student) in
            compute_audit_pairs(&rec, args.teacher_depth, &student_depths)
        {
            overall.record(bestmove_match, score_error_cp, teacher, student);
            per_depth.entry(student_depth).or_default().record(
                bestmove_match,
                score_error_cp,
                teacher,
                student,
            );

            serde_json::to_writer(
                &mut writer,
                &serde_json::json!({
                    "sfen": &rec.sfen,
                    "source": &rec.source,
                    "engine": &teacher.engine,
                    "teacher_requested_depth": teacher.requested_depth,
                    "teacher_depth": teacher.depth,
                    "teacher_score_bound": score_bound_str(teacher.score_bound),
                    "teacher_underreach": requested_depth_underreached(teacher),
                    "teacher_bestmove_kind": effective_bestmove_kind(teacher),
                    "student_requested_depth": student.requested_depth,
                    "student_depth": student.depth,
                    "student_score_bound": score_bound_str(student.score_bound),
                    "student_underreach": requested_depth_underreached(student),
                    "student_bestmove_kind": effective_bestmove_kind(student),
                    "bestmove_match": bestmove_match,
                    "score_error_cp": score_error_cp,
                }),
            )?;
            writer.write_all(b"\n")?;
        }
    }

    writer.flush()?;
    eprintln!(
        "done: {total_records} records read, {} pairs compared → {:?}",
        overall.pairs, args.out
    );
    if overall.pairs == 0 {
        eprintln!(
            "(no (engine, student_depth) pair had both a teacher_depth={} and a listed student depth observation present)",
            args.teacher_depth
        );
        return Ok(());
    }
    eprintln!("per student depth:");
    for (&depth, stats) in &per_depth {
        stats.print(&format!("student_depth={depth}"));
    }
    overall.print("overall");

    Ok(())
}

/// One (policy_margin, score_swing) grid cell's outcome: coverage over the WHOLE dataset under
/// that combined `QualityConfig` (same denominator as `calibrate`'s sweep rows), plus, for the
/// SUBSET of records this cell keeps, the pooled teacher-vs-student audit signal across every
/// requested student depth and engine (`AuditStats`, unchanged -- reused exactly as `cmd_audit`
/// uses it). `coverage.kept` and `audit.pairs` are expected to differ: coverage answers "how much
/// data survives this gate," audit answers "of the survivors we could check against a teacher, how
/// much do we actually trust them" -- the whole point of pairing the two.
struct TuneCell {
    policy_margin: Option<i32>,
    score_swing: Option<i32>,
    coverage: CoverageTally,
    audit: AuditStats,
}

impl TuneCell {
    fn new(policy_margin: Option<i32>, score_swing: Option<i32>) -> Self {
        Self {
            policy_margin,
            score_swing,
            coverage: CoverageTally::default(),
            audit: AuditStats::default(),
        }
    }

    /// Fraction (not percent) of the dataset this cell keeps -- `0.0` if the dataset was empty.
    fn coverage_fraction(&self) -> f64 {
        if self.coverage.total == 0 {
            0.0
        } else {
            self.coverage.kept as f64 / self.coverage.total as f64
        }
    }

    /// Only meaningful (and only ever called) when `audit.pairs > 0` -- callers must check that
    /// first, matching `AuditStats::print`'s own `if self.pairs == 0 { return; }` guard.
    fn mismatch_rate(&self) -> f64 {
        self.audit.bestmove_mismatches as f64 / self.audit.pairs as f64
    }

    /// `(policy_margin, score_swing)` with an inactive axis sorting first -- used only to break
    /// ties deterministically between cells that are otherwise numerically identical on every
    /// metric, so candidate selection never depends on grid iteration order.
    fn lexicographic_key(&self) -> (i32, i32) {
        (
            self.policy_margin.unwrap_or(i32::MIN),
            self.score_swing.unwrap_or(i32::MIN),
        )
    }

    fn threshold_csv_cells(&self) -> (String, String) {
        (
            self.policy_margin
                .map(|v| v.to_string())
                .unwrap_or_default(),
            self.score_swing.map(|v| v.to_string()).unwrap_or_default(),
        )
    }

    fn to_csv_row(&self) -> String {
        let (policy_margin, score_swing) = self.threshold_csv_cells();
        // Audit-derived columns render empty (not "0.00") when there's no audit pair to compute
        // them from -- a real 0% mismatch rate must never be confused with "no data," mirroring
        // `AuditStats::print`'s own `if pairs == 0 { return }` guard.
        let pct = |n: usize| -> String {
            if self.audit.pairs == 0 {
                String::new()
            } else {
                format!("{:.2}", n as f64 / self.audit.pairs as f64 * 100.0)
            }
        };
        let avg_abs_score_error_cp = if self.audit.abs_score_error_count > 0 {
            format!(
                "{:.2}",
                self.audit.abs_score_error_sum / self.audit.abs_score_error_count as f64
            )
        } else {
            String::new()
        };
        let max_abs_score_error_cp = if self.audit.abs_score_error_count > 0 {
            self.audit.abs_score_error_max.to_string()
        } else {
            String::new()
        };
        let total = self.coverage.total;
        let kept = self.coverage.kept;
        let dropped = self.coverage.dropped;
        let coverage_pct = self.coverage.coverage_pct();
        let drop_reasons = self.coverage.drop_reasons_csv_cell();
        let audit_pairs = self.audit.pairs;
        let teacher_bestmove_mismatch_pct = pct(self.audit.bestmove_mismatches);
        let teacher_non_exact_pct = pct(self.audit.teacher_non_exact);
        let student_non_exact_pct = pct(self.audit.student_non_exact);
        let teacher_underreach_pct = pct(self.audit.teacher_underreach);
        let student_underreach_pct = pct(self.audit.student_underreach);
        let teacher_special_bestmove_pct = pct(self.audit.teacher_special_bestmove);
        let student_special_bestmove_pct = pct(self.audit.student_special_bestmove);
        format!(
            "{policy_margin},{score_swing},{total},{kept},{dropped},{coverage_pct:.2},\
             {drop_reasons},{audit_pairs},{teacher_bestmove_mismatch_pct},\
             {avg_abs_score_error_cp},{max_abs_score_error_cp},{teacher_non_exact_pct},\
             {student_non_exact_pct},{teacher_underreach_pct},{student_underreach_pct},\
             {teacher_special_bestmove_pct},{student_special_bestmove_pct}"
        )
    }
}

/// Whether `a` Pareto-dominates `b` on (coverage, mismatch_rate): at least as good on both axes
/// (higher coverage, lower mismatch), strictly better on at least one. Only called on cells with
/// `audit.pairs > 0` on both sides (the frontier excludes cells with no audit data entirely).
fn dominates(a: &TuneCell, b: &TuneCell) -> bool {
    let (a_cov, b_cov) = (a.coverage_fraction(), b.coverage_fraction());
    let (a_mis, b_mis) = (a.mismatch_rate(), b.mismatch_rate());
    let not_worse = a_cov >= b_cov && a_mis <= b_mis;
    let strictly_better = a_cov > b_cov || a_mis < b_mis;
    not_worse && strictly_better
}

/// Indices into `grid` of cells with `audit.pairs > 0` that are not Pareto-dominated by any other
/// such cell. Plain O(n^2) pairwise comparison -- grid sizes here are always tens of cells, so an
/// O(n log n) skyline sweep would trade a smaller diff for a subtler-to-verify implementation with
/// no real benefit at this scale.
fn pareto_frontier_indices(grid: &[TuneCell]) -> Vec<usize> {
    let candidates: Vec<usize> = (0..grid.len())
        .filter(|&i| grid[i].audit.pairs > 0)
        .collect();
    candidates
        .iter()
        .copied()
        .filter(|&i| {
            !candidates
                .iter()
                .any(|&j| j != i && dominates(&grid[j], &grid[i]))
        })
        .collect()
}

/// Index (into `grid`) of the frontier point with maximum coverage -- the "keep the most data"
/// candidate. Ties broken by minimum mismatch_rate, then by `lexicographic_key` for determinism.
fn pick_broad(grid: &[TuneCell], frontier: &[usize]) -> usize {
    let mut best = frontier[0];
    for &i in &frontier[1..] {
        let better = grid[i].coverage_fraction() > grid[best].coverage_fraction()
            || (grid[i].coverage_fraction() == grid[best].coverage_fraction()
                && grid[i].mismatch_rate() < grid[best].mismatch_rate())
            || (grid[i].coverage_fraction() == grid[best].coverage_fraction()
                && grid[i].mismatch_rate() == grid[best].mismatch_rate()
                && grid[i].lexicographic_key() < grid[best].lexicographic_key());
        if better {
            best = i;
        }
    }
    best
}

/// Index (into `grid`) of the frontier point with minimum mismatch_rate -- the "trust the data
/// most" candidate. Ties broken by maximum coverage, then `lexicographic_key`.
fn pick_strict(grid: &[TuneCell], frontier: &[usize]) -> usize {
    let mut best = frontier[0];
    for &i in &frontier[1..] {
        let better = grid[i].mismatch_rate() < grid[best].mismatch_rate()
            || (grid[i].mismatch_rate() == grid[best].mismatch_rate()
                && grid[i].coverage_fraction() > grid[best].coverage_fraction())
            || (grid[i].mismatch_rate() == grid[best].mismatch_rate()
                && grid[i].coverage_fraction() == grid[best].coverage_fraction()
                && grid[i].lexicographic_key() < grid[best].lexicographic_key());
        if better {
            best = i;
        }
    }
    best
}

/// Index (into `grid`) of the frontier point closest to the ideal corner (coverage=1,
/// mismatch_rate=0) -- the "split the difference" candidate.
///
/// Why: coverage and mismatch_rate are both mathematically in [0,1], but their *dynamic range
/// across a real frontier* is typically wildly different -- coverage might span 0.35..0.95 while
/// mismatch_rate spans only 0.02..0.11. Computing distance on those raw values lets the coverage
/// term dominate regardless of L1 or Euclidean, so "balanced" silently collapses onto "broad" and
/// the three-candidate menu degenerates to two. Min-max normalizing each axis to the frontier's
/// OWN observed range (not a fixed assumed 0..1) fixes this; the distance formula itself
/// (Euclidean here) is a minor detail once that normalization is in place.
fn pick_balanced(grid: &[TuneCell], frontier: &[usize]) -> usize {
    let covs = frontier.iter().map(|&i| grid[i].coverage_fraction());
    let miss = frontier.iter().map(|&i| grid[i].mismatch_rate());
    let cov_min = covs.clone().fold(f64::INFINITY, f64::min);
    let cov_max = covs.fold(f64::NEG_INFINITY, f64::max);
    let mis_min = miss.clone().fold(f64::INFINITY, f64::min);
    let mis_max = miss.fold(f64::NEG_INFINITY, f64::max);
    // A single-valued (degenerate) axis normalizes to 0 for every point rather than dividing by
    // zero -- it contributes nothing to the distance, which is correct: there's no variation on
    // that axis to weigh "balanced" against.
    let normalize = |value: f64, lo: f64, hi: f64| {
        if hi > lo {
            (value - lo) / (hi - lo)
        } else {
            0.0
        }
    };

    let mut best = frontier[0];
    let mut best_dist = f64::INFINITY;
    for &i in frontier {
        let cov_norm = normalize(grid[i].coverage_fraction(), cov_min, cov_max);
        let mis_norm = normalize(grid[i].mismatch_rate(), mis_min, mis_max);
        let dist = ((1.0 - cov_norm).powi(2) + mis_norm.powi(2)).sqrt();
        if dist < best_dist
            || (dist == best_dist && grid[i].lexicographic_key() < grid[best].lexicographic_key())
        {
            best = i;
            best_dist = dist;
        }
    }
    best
}

/// Every candidate label(s) that landed on grid index `idx`, in `broad, balanced, strict` order --
/// a `Vec` (not a single label) because the frontier can have fewer than 3 distinct points, in
/// which case multiple roles coincide on the same cell.
fn candidate_labels(idx: usize, broad: usize, balanced: usize, strict: usize) -> Vec<&'static str> {
    [(broad, "broad"), (balanced, "balanced"), (strict, "strict")]
        .into_iter()
        .filter(|&(i, _)| i == idx)
        .map(|(_, label)| label)
        .collect()
}

/// Writes `--report`'s Pareto-frontier Markdown. A no-op-content-but-still-`Ok`-file when no cell
/// has any audit data (e.g. `--teacher-depth`/`--student-depths` matched nothing) -- mirrors
/// `cmd_audit`'s own "pairs == 0 -> informative message, not an error" convention.
fn write_tune_report(path: &Path, args: &TuneArgs, total: usize, grid: &[TuneCell]) -> Result<()> {
    let mut report = String::new();
    writeln!(report, "# Tuning Report")?;
    writeln!(report)?;
    writeln!(report, "Input: {:?}, {total} records read", args.input)?;
    writeln!(
        report,
        "Teacher depth: {}. Student depths: {}.",
        args.teacher_depth, args.student_depths
    )?;
    writeln!(report, "Grid: {} configurations", grid.len())?;
    writeln!(report)?;

    let frontier = pareto_frontier_indices(grid);
    if frontier.is_empty() {
        writeln!(
            report,
            "No grid cell had a teacher/student comparison pair -- check `--teacher-depth`/\
             `--student-depths` against what's actually in the data. Pareto analysis skipped; \
             see the CSV for coverage-only results."
        )?;
        fs::write(path, report)?;
        return Ok(());
    }

    let broad = pick_broad(grid, &frontier);
    let balanced = pick_balanced(grid, &frontier);
    let strict = pick_strict(grid, &frontier);

    writeln!(report, "## Why three candidates, not one")?;
    writeln!(report)?;
    writeln!(
        report,
        "Training-data needs vary by whether this round wants quantity or reliability -- \
         shogiesa doesn't presume to know which, so it hands back the Pareto-optimal menu below \
         instead of picking one \"correct\" threshold for you."
    )?;
    writeln!(report)?;

    writeln!(report, "## Recommended candidates")?;
    writeln!(report)?;
    writeln!(
        report,
        "| candidate | policy_margin | score_swing | coverage | teacher_mismatch_rate | avg \\|score_error\\| cp | max \\|score_error\\| cp | audit pairs |"
    )?;
    writeln!(report, "|---|---|---|---|---|---|---|---|")?;
    let mut seen = Vec::new();
    for &idx in &[broad, balanced, strict] {
        if seen.contains(&idx) {
            continue;
        }
        seen.push(idx);
        let cell = &grid[idx];
        let labels = candidate_labels(idx, broad, balanced, strict).join(", ");
        let (policy_margin, score_swing) = cell.threshold_csv_cells();
        let avg_abs = if cell.audit.abs_score_error_count > 0 {
            format!(
                "{:.1}",
                cell.audit.abs_score_error_sum / cell.audit.abs_score_error_count as f64
            )
        } else {
            "n/a".to_string()
        };
        writeln!(
            report,
            "| {labels} | {policy_margin} | {score_swing} | {:.1}% | {:.1}% | {avg_abs} | {} | {} |",
            cell.coverage_fraction() * 100.0,
            cell.mismatch_rate() * 100.0,
            cell.audit.abs_score_error_max,
            cell.audit.pairs
        )?;
    }
    if seen.len() < 3 {
        writeln!(report)?;
        writeln!(
            report,
            "(The Pareto frontier had fewer than 3 distinct points, so some candidates coincided \
             on the same grid cell above.)"
        )?;
    }
    writeln!(report)?;

    writeln!(report, "## Pareto frontier")?;
    writeln!(report)?;
    let mut sorted_frontier = frontier.clone();
    sorted_frontier.sort_by(|&a, &b| {
        grid[b]
            .coverage_fraction()
            .total_cmp(&grid[a].coverage_fraction())
    });
    writeln!(
        report,
        "| policy_margin | score_swing | coverage | teacher_mismatch_rate | candidate |"
    )?;
    writeln!(report, "|---|---|---|---|---|")?;
    for &idx in &sorted_frontier {
        let cell = &grid[idx];
        let (policy_margin, score_swing) = cell.threshold_csv_cells();
        let labels = candidate_labels(idx, broad, balanced, strict).join(", ");
        writeln!(
            report,
            "| {policy_margin} | {score_swing} | {:.1}% | {:.1}% | {labels} |",
            cell.coverage_fraction() * 100.0,
            cell.mismatch_rate() * 100.0
        )?;
    }
    writeln!(report)?;

    writeln!(report, "## Full grid")?;
    writeln!(report)?;
    if grid.len() <= 50 {
        writeln!(
            report,
            "| policy_margin | score_swing | coverage | kept | dropped | audit pairs | teacher_mismatch_rate |"
        )?;
        writeln!(report, "|---|---|---|---|---|---|---|")?;
        for cell in grid {
            let (policy_margin, score_swing) = cell.threshold_csv_cells();
            let mismatch = if cell.audit.pairs > 0 {
                format!("{:.1}%", cell.mismatch_rate() * 100.0)
            } else {
                "n/a".to_string()
            };
            writeln!(
                report,
                "| {policy_margin} | {score_swing} | {:.1}% | {} | {} | {} | {mismatch} |",
                cell.coverage_fraction() * 100.0,
                cell.coverage.kept,
                cell.coverage.dropped,
                cell.audit.pairs
            )?;
        }
    } else {
        writeln!(
            report,
            "See `{:?}` for the full {}-row grid.",
            args.out,
            grid.len()
        )?;
    }

    fs::write(path, report)?;
    Ok(())
}

/// Versions `--preset-out`'s JSON shape, independent of `SCHEMA_VERSION` (which versions the
/// PositionRecord/pack data model) -- this is a standalone artifact, not a labeled dataset.
const TUNE_PRESET_FORMAT_VERSION: u32 = 1;

/// One named candidate's full resolved `QualityConfig`, ready to hand straight to
/// `filter --preset FILE.json:label` -- carrying the whole config (not just the swept fields)
/// means filter never has to re-derive which base flags this tune run held fixed.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TunePresetCandidate {
    config: QualityConfig,
    coverage_fraction: f64,
    /// `None` only when `audit_pairs == 0` (no teacher/student comparison available for this
    /// cell) -- mirrors `TuneCell::mismatch_rate`'s own precondition.
    #[serde(skip_serializing_if = "Option::is_none")]
    mismatch_rate: Option<f64>,
    audit_pairs: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TunePresetFile {
    preset_format_version: u32,
    tool: String,
    input: String,
    teacher_depth: u32,
    student_depths: String,
    /// Keys: "broad" / "balanced" / "strict". A map (not 3 fixed fields) so the <3-distinct-
    /// frontier-points degeneracy (see `candidate_labels`) is handled the same way --report's
    /// Markdown already handles it: duplicate the full config under each coinciding label, so
    /// `filter --preset x.json:strict` always resolves even when strict and broad landed on the
    /// same cell.
    presets: BTreeMap<String, TunePresetCandidate>,
}

/// Writes `--preset-out`'s machine-readable JSON. An empty `presets` map when no cell has audit
/// data is a valid, non-error output -- mirrors `write_tune_report`'s own degenerate case.
fn write_tune_preset(
    path: &Path,
    args: &TuneArgs,
    base_config: &QualityConfig,
    grid: &[TuneCell],
) -> Result<()> {
    let mut presets = BTreeMap::new();
    let frontier = pareto_frontier_indices(grid);
    if !frontier.is_empty() {
        let broad = pick_broad(grid, &frontier);
        let balanced = pick_balanced(grid, &frontier);
        let strict = pick_strict(grid, &frontier);
        for (label, idx) in [("broad", broad), ("balanced", balanced), ("strict", strict)] {
            let cell = &grid[idx];
            // Same expression cmd_tune's hot loop already builds per cell -- reused here outside
            // the loop for just these (at most) 3 indices, not refactored into a shared helper:
            // the recomputation is O(frontier size), trivially cheap, and duplicating this small
            // expression is safer than reshaping cmd_tune's already-tested hot loop.
            let config = QualityConfig {
                min_policy_margin_cp: cell.policy_margin,
                max_score_swing_cp: cell.score_swing,
                ..base_config.clone()
            };
            presets.insert(
                label.to_string(),
                TunePresetCandidate {
                    config,
                    coverage_fraction: cell.coverage_fraction(),
                    mismatch_rate: (cell.audit.pairs > 0).then(|| cell.mismatch_rate()),
                    audit_pairs: cell.audit.pairs,
                },
            );
        }
    }
    let file = TunePresetFile {
        preset_format_version: TUNE_PRESET_FORMAT_VERSION,
        tool: "shogiesa tune".to_string(),
        input: args.input.display().to_string(),
        teacher_depth: args.teacher_depth,
        student_depths: args.student_depths.clone(),
        presets,
    };
    fs::write(path, serde_json::to_string_pretty(&file)?)?;
    Ok(())
}

fn cmd_tune(args: TuneArgs) -> Result<()> {
    if args.sweep_policy_margin.is_none() && args.sweep_score_swing.is_none() {
        anyhow::bail!("tune requires at least one of --sweep-policy-margin/--sweep-score-swing");
    }
    let student_depths = parse_u32_list(&args.student_depths)?;

    let allowed_phases = parse_allowed_phases(args.phase.as_deref());
    // Same convention as calibrate: a field currently being swept is always None here (overridden
    // per grid cell below); Some only when that field is instead held fixed while the OTHER
    // dimension sweeps (enforced by clap's conflicts_with on the sweep/hold pairs).
    let base_config = QualityConfig {
        min_observations: args.min_observations,
        allowed_phases,
        exclude_mate: args.exclude_mate,
        exclude_in_check: args.exclude_in_check,
        exclude_capture: args.exclude_capture,
        eval_min: args.eval_min,
        eval_max: args.eval_max,
        max_score_swing_cp: args.max_score_swing_cp,
        min_policy_margin_cp: args.min_policy_margin_cp,
        require_bestmove_agreement: args.require_bestmove_agreement,
        require_engine_agreement: args.require_engine_agreement,
        max_engine_score_swing_cp: args.max_engine_score_swing_cp,
        require_exact_score: args.require_exact_score,
        require_policy_margin: args.require_policy_margin,
        min_depth_reached: args.min_depth_reached,
        require_requested_depth_reached: args.require_requested_depth_reached,
    };

    // A held-but-not-swept axis becomes a single-value list (possibly `None`, meaning "gate
    // inactive") -- this makes `tune` a strict superset of `calibrate`'s shape: a 1xN or Nx1 grid
    // degenerates to exactly calibrate's independent-sweep behavior, not a divergent second mode.
    let policy_values: Vec<Option<i32>> = match &args.sweep_policy_margin {
        Some(s) => parse_int_list(s)?.into_iter().map(Some).collect(),
        None => vec![args.min_policy_margin_cp],
    };
    let swing_values: Vec<Option<i32>> = match &args.sweep_score_swing {
        Some(s) => parse_int_list(s)?.into_iter().map(Some).collect(),
        None => vec![args.max_score_swing_cp],
    };

    let mut grid: Vec<TuneCell> = Vec::with_capacity(policy_values.len() * swing_values.len());
    for &policy_margin in &policy_values {
        for &score_swing_value in &swing_values {
            grid.push(TuneCell::new(policy_margin, score_swing_value));
        }
    }

    let reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    let mut total = 0usize;
    let mut diagnostics = DatasetDiagnostics::default();

    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: PositionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(line = i + 1, "JSON parse error: {e}");
                continue;
            }
        };
        total += 1;
        diagnostics.record(&rec);

        // Computed ONCE per record, independent of any grid cell's thresholds -- the comparison
        // itself doesn't depend on a quality gate; only whether a given cell counts it toward
        // "kept" does. Folding these into every cell below, instead of recomputing per cell,
        // is what keeps this a single streaming pass despite an M*N grid.
        let audit_pairs = compute_audit_pairs(&rec, args.teacher_depth, &student_depths);

        // Why this calls evaluate_quality M*N times per record instead of caching one base
        // decision and only re-checking the two swept gates: doing the latter would mean
        // re-deriving evaluate_quality's own pass/fail logic here in the CLI, exactly the
        // parallel judgment logic this project centralizes in shogiesa_core instead. Grid sizes
        // are always small (tens of cells), so this isn't a real performance concern.
        for cell in &mut grid {
            let config = QualityConfig {
                min_policy_margin_cp: cell.policy_margin,
                max_score_swing_cp: cell.score_swing,
                ..base_config.clone()
            };
            let decision = evaluate_quality(&rec, &config);
            let kept = decision.keep;
            cell.coverage.record(&decision);
            if kept {
                for &(_, bestmove_match, score_error_cp, teacher, student) in &audit_pairs {
                    cell.audit
                        .record(bestmove_match, score_error_cp, teacher, student);
                }
            }
        }
    }

    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);
    writeln!(
        writer,
        "policy_margin,score_swing,total,kept,dropped,coverage_pct,drop_reasons,audit_pairs,\
         teacher_bestmove_mismatch_pct,avg_abs_score_error_cp,max_abs_score_error_cp,\
         teacher_non_exact_pct,student_non_exact_pct,teacher_underreach_pct,student_underreach_pct,\
         teacher_special_bestmove_pct,student_special_bestmove_pct"
    )?;
    for cell in &grid {
        writeln!(writer, "{}", cell.to_csv_row())?;
    }
    writer.flush()?;

    eprintln!(
        "done: {total} read, {} grid configurations → {:?}",
        grid.len(),
        args.out
    );
    diagnostics.print();

    if let Some(report_path) = &args.report {
        write_tune_report(report_path, &args, total, &grid)?;
        eprintln!("report → {report_path:?}");
    }

    if let Some(preset_path) = &args.preset_out {
        write_tune_preset(preset_path, &args, &base_config, &grid)?;
        eprintln!("preset → {preset_path:?}");
    }

    Ok(())
}

/// Every `.json` file under a `label --cache-dir`'s two-level shard layout
/// (`cache_dir/xx/<64-hex-hash>.json`, see `label_cache_path`). Filtering strictly on the `.json`
/// extension skips `write_cache_entry_atomically`'s in-flight temp files
/// (`<hash>.json.tmp.<pid>.<tid>`) by construction, so a walk never races a concurrent `label`
/// process sharing the same cache dir.
fn walk_cache_entries(cache_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = Vec::new();
    for shard in fs::read_dir(cache_dir).with_context(|| format!("cannot read {cache_dir:?}"))? {
        let shard_path = shard?.path();
        if !shard_path.is_dir() {
            continue;
        }
        for entry in
            fs::read_dir(&shard_path).with_context(|| format!("cannot read {shard_path:?}"))?
        {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                entries.push(path);
            }
        }
    }
    Ok(entries)
}

fn cmd_cache_stats(args: CacheStatsArgs) -> Result<()> {
    let entries = walk_cache_entries(&args.cache_dir)?;
    let now = std::time::SystemTime::now();
    let mut total_bytes = 0u64;
    let mut oldest: Option<std::time::SystemTime> = None;
    let mut newest: Option<std::time::SystemTime> = None;
    // Parsed straight from each entry's `Observation.engine` field -- present in both v1 and v2
    // payloads, not inferred from the (otherwise opaque, one-way-hashed) cache key/path.
    let mut engine_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut legacy_v1_count = 0usize;
    // These four distributions only exist for v2 entries -- a v1 payload is a bare `Observation`
    // with no schema_version/engine_fingerprint/requested_depth/multipv of its own to report.
    let mut schema_version_counts: BTreeMap<u32, usize> = BTreeMap::new();
    let mut fingerprint_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut requested_depth_counts: BTreeMap<u32, usize> = BTreeMap::new();
    let mut multipv_counts: BTreeMap<u32, usize> = BTreeMap::new();

    for path in &entries {
        if let Ok(meta) = fs::metadata(path) {
            total_bytes += meta.len();
            if let Ok(modified) = meta.modified() {
                oldest = Some(oldest.map_or(modified, |o| o.min(modified)));
                newest = Some(newest.map_or(modified, |n| n.max(modified)));
            }
        }
        if let Some(parsed) = fs::read_to_string(path)
            .ok()
            .and_then(|s| parse_cache_entry(&s))
        {
            *engine_counts
                .entry(parsed.observation().engine.clone())
                .or_default() += 1;
            match &parsed {
                CacheRead::V1(_) => legacy_v1_count += 1,
                CacheRead::V2(entry) => {
                    *schema_version_counts
                        .entry(entry.schema_version)
                        .or_default() += 1;
                    let fingerprint_key = entry
                        .engine_fingerprint
                        .map(|fp| format!("{fp:016x}"))
                        .unwrap_or_else(|| "none".to_string());
                    *fingerprint_counts.entry(fingerprint_key).or_default() += 1;
                    *requested_depth_counts
                        .entry(entry.requested_depth)
                        .or_default() += 1;
                    *multipv_counts.entry(entry.multipv).or_default() += 1;
                }
            }
        }
    }

    println!("cache entries : {}", entries.len());
    println!("total size    : {total_bytes} bytes");
    if let Some(oldest) = oldest {
        let days = now.duration_since(oldest).unwrap_or_default().as_secs() / 86400;
        println!("oldest entry  : {days} days old");
    }
    if let Some(newest) = newest {
        let days = now.duration_since(newest).unwrap_or_default().as_secs() / 86400;
        println!("newest entry  : {days} days old");
    }
    if !engine_counts.is_empty() {
        println!("engine distribution:");
        for (engine, count) in &engine_counts {
            println!("  {engine:<20} {count:>6}");
        }
    }
    if legacy_v1_count > 0 {
        println!("legacy (v1, no metadata): {legacy_v1_count} entries");
    }
    if !schema_version_counts.is_empty() {
        println!("schema_version distribution (v2 entries only):");
        for (v, count) in &schema_version_counts {
            println!("  {v:<20} {count:>6}");
        }
    }
    if !fingerprint_counts.is_empty() {
        println!("engine_fingerprint distribution (v2 entries only):");
        for (fp, count) in &fingerprint_counts {
            println!("  {fp:<20} {count:>6}");
        }
    }
    if !requested_depth_counts.is_empty() {
        println!("requested_depth distribution (v2 entries only):");
        for (d, count) in &requested_depth_counts {
            println!("  {d:<20} {count:>6}");
        }
    }
    if !multipv_counts.is_empty() {
        println!("multipv distribution (v2 entries only):");
        for (m, count) in &multipv_counts {
            println!("  {m:<20} {count:>6}");
        }
    }
    Ok(())
}

fn cmd_cache_verify(args: CacheVerifyArgs) -> Result<()> {
    let entries = walk_cache_entries(&args.cache_dir)?;
    let mut corrupted = 0usize;
    let mut legacy_v1_count = 0usize;
    let mut schema_version_counts: BTreeMap<u32, usize> = BTreeMap::new();

    for path in &entries {
        match fs::read_to_string(path)
            .ok()
            .and_then(|s| parse_cache_entry(&s))
        {
            Some(CacheRead::V1(_)) => legacy_v1_count += 1,
            Some(CacheRead::V2(entry)) => {
                *schema_version_counts
                    .entry(entry.schema_version)
                    .or_default() += 1;
            }
            None => {
                corrupted += 1;
                tracing::warn!(path = %path.display(), "corrupted cache entry");
            }
        }
    }

    println!("cache entries : {}", entries.len());
    println!("corrupted     : {corrupted}");
    if legacy_v1_count > 0 {
        println!("legacy (v1, no metadata): {legacy_v1_count} entries");
    }
    if !schema_version_counts.is_empty() {
        println!("schema_version distribution (v2 entries only):");
        for (v, count) in &schema_version_counts {
            println!("  {v:<20} {count:>6}");
        }
    }
    // Deliberately not claiming a LIVE staleness check ("does this entry match today's engine/
    // schema"): label_cache_path already folds SCHEMA_VERSION and the engine fingerprint into the
    // (one-way) cache key hash, so a schema bump or a different engine binary simply produces a
    // different key -- a stale entry is never wrongly returned as a hit, it's just orphaned dead
    // weight. A true live check would need this command to also take --engine/
    // --engine-fingerprint-mode to recompute today's fingerprint and compare -- a real but
    // separate feature, not built here; v1 legacy entries have no metadata to check against in
    // the first place.
    println!(
        "note: v1 (legacy) entries store no schema_version/engine_fingerprint metadata; v2 \
         entries do (see the distribution above). Neither format supports a live \"does this \
         match today's engine/schema\" check here -- a schema bump or engine change already \
         changes future cache keys by construction (see label_cache_path), so a stale entry is \
         never wrongly reused, just orphaned."
    );
    Ok(())
}

/// The file's age (now minus mtime), or `None` if either the metadata/mtime read fails or the
/// clock has gone backwards -- either way, "unknown age" should never count as "old enough to
/// prune."
fn file_age(path: &Path, now: std::time::SystemTime) -> Option<std::time::Duration> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    now.duration_since(modified).ok()
}

fn cmd_cache_prune(args: CachePruneArgs) -> Result<()> {
    if !args.corrupted_only && !args.legacy_only && args.older_than_days.is_none() {
        anyhow::bail!(
            "cache prune requires --corrupted-only, --legacy-only, and/or --older-than-days"
        );
    }
    let entries = walk_cache_entries(&args.cache_dir)?;
    let now = std::time::SystemTime::now();
    let max_age = args
        .older_than_days
        .map(|days| std::time::Duration::from_secs(days * 86400));

    let mut to_delete = Vec::new();
    for path in &entries {
        let mut matched = false;
        if args.corrupted_only || args.legacy_only {
            let parsed = fs::read_to_string(path)
                .ok()
                .and_then(|s| parse_cache_entry(&s));
            match &parsed {
                None if args.corrupted_only => matched = true,
                Some(CacheRead::V1(_)) if args.legacy_only => matched = true,
                _ => {}
            }
        }
        if let Some(max_age) = max_age
            && file_age(path, now).is_some_and(|age| age >= max_age)
        {
            matched = true;
        }
        if matched {
            to_delete.push(path.clone());
        }
    }

    if args.yes {
        let mut deleted = 0usize;
        for path in &to_delete {
            match fs::remove_file(path) {
                Ok(()) => deleted += 1,
                Err(e) => tracing::warn!(path = %path.display(), "failed to delete: {e}"),
            }
        }
        eprintln!(
            "deleted {deleted}/{} matched entries ({} total)",
            to_delete.len(),
            entries.len()
        );
    } else {
        eprintln!(
            "dry run: {}/{} entries matched and would be deleted (pass --yes to actually delete)",
            to_delete.len(),
            entries.len()
        );
    }
    Ok(())
}

fn collect_game_paths(input: &PathBuf) -> Result<Vec<PathBuf>> {
    if input.is_file() {
        return Ok(vec![input.clone()]);
    }
    let mut paths = Vec::new();
    for entry in
        fs::read_dir(input).with_context(|| format!("cannot read directory {:?}", input))?
    {
        let entry = entry?;
        let p = entry.path();
        if matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("csa" | "kif" | "ki2")
        ) {
            paths.push(p);
        }
    }
    paths.sort();
    Ok(paths)
}

fn load_records(path: &PathBuf) -> Result<(Vec<PositionRecord>, usize)> {
    let content = fs::read_to_string(path).with_context(|| format!("cannot read {:?}", path))?;
    let mut broken = 0usize;
    let records: Vec<PositionRecord> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .filter_map(
            |(i, line)| match serde_json::from_str::<PositionRecord>(line) {
                Ok(rec) => Some(rec),
                Err(e) => {
                    tracing::warn!(line = i + 1, "parse error: {e}");
                    broken += 1;
                    None
                }
            },
        )
        .collect();
    Ok((records, broken))
}

fn cmd_report(args: ReportArgs) -> Result<()> {
    let reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );

    let mut n = 0usize;
    let mut broken = 0usize;
    let mut phases = BTreeMap::<String, usize>::new();
    let mut sides = BTreeMap::<String, usize>::new();
    let mut schema_versions = BTreeMap::<u32, usize>::new();
    let mut ply_sum = 0u64;
    let mut ply_min = u32::MAX;
    let mut ply_max = 0u32;
    let mut sfen_counts: HashMap<String, usize> = HashMap::new();
    let mut tag_mismatches = 0usize;
    let mut invalid_sfens = 0usize;
    let mut labeled = 0usize;
    let mut in_check = 0usize;
    let mut has_capture = 0usize;
    let mut depth_disagree = 0usize;
    let mut multi_engine = 0usize;
    let mut engine_disagree = 0usize;
    let mut special_bestmove = 0usize;
    let mut depth_counts: BTreeMap<u32, usize> = BTreeMap::new();
    // eval buckets: key = floor(cp / 200) * 200; special keys: i32::MIN = unlabeled, i32::MAX = mate
    let mut eval_buckets: BTreeMap<i32, usize> = BTreeMap::new();
    let mut eval_by_phase: BTreeMap<i32, BTreeMap<String, usize>> = BTreeMap::new();
    let mut eval_by_side: BTreeMap<i32, BTreeMap<String, usize>> = BTreeMap::new();
    let mut cp_obs_count = 0usize;
    let mut mate_obs_count = 0usize;
    let mut swing_sum = 0f64;
    let mut swing_count = 0usize;
    // score swing histogram: key = floor(swing / 50) * 50
    let mut swing_buckets: BTreeMap<i32, usize> = BTreeMap::new();
    let mut margin_sum = 0f64;
    let mut margin_count = 0usize;
    let mut obs_score_bound_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut sources = BTreeMap::<String, usize>::new();
    let mut obs_with_candidates = 0usize;
    let mut obs_total = 0usize;
    let mut score_bound_distribution: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut requested_depth_total = 0usize;
    let mut requested_depth_underreach = 0usize;

    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: PositionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(line = i + 1, "parse error: {e}");
                broken += 1;
                continue;
            }
        };
        n += 1;

        *phases.entry(format!("{}", rec.tags.phase)).or_default() += 1;
        *sides
            .entry(format!("{}", rec.tags.side_to_move))
            .or_default() += 1;
        *schema_versions.entry(rec.schema_version).or_default() += 1;
        if rec.tags.in_check {
            in_check += 1;
        }
        if rec.tags.has_capture {
            has_capture += 1;
        }
        let ply = rec.source.ply;
        ply_sum += ply as u64;
        ply_min = ply_min.min(ply);
        ply_max = ply_max.max(ply);
        *sfen_counts.entry(rec.sfen.clone()).or_default() += 1;
        *sources.entry(rec.source.path.clone()).or_default() += 1;

        match Sfen::parse(&rec.sfen) {
            Ok(sfen) => {
                if sfen.side_to_move() != rec.tags.side_to_move {
                    tag_mismatches += 1;
                }
            }
            Err(_) => invalid_sfens += 1,
        }

        // observation stats
        if rec.observations.is_empty() {
            *eval_buckets.entry(i32::MIN).or_default() += 1; // unlabeled sentinel
            *eval_by_phase
                .entry(i32::MIN)
                .or_default()
                .entry(format!("{}", rec.tags.phase))
                .or_default() += 1;
            *eval_by_side
                .entry(i32::MIN)
                .or_default()
                .entry(format!("{}", rec.tags.side_to_move))
                .or_default() += 1;
        } else {
            labeled += 1;
            // bestmove disagreement, excluding resign/win/none tokens (one engine giving up isn't
            // an opinion about which move is best)
            if !bestmove_agreement(&rec.observations) {
                depth_disagree += 1;
            }
            if has_special_bestmove(&rec.observations) {
                special_bestmove += 1;
            }
            if let Some(agree) = engine_bestmove_agreement(&rec.observations) {
                multi_engine += 1;
                if !agree {
                    engine_disagree += 1;
                }
            }
            let mut cp_scores = Vec::new();
            for obs in &rec.observations {
                *depth_counts.entry(obs.depth).or_default() += 1;
                match obs.score {
                    Score::Cp { value } => {
                        cp_obs_count += 1;
                        cp_scores.push(value);
                    }
                    Score::Mate { .. } => mate_obs_count += 1,
                }
                if let Some(margin) = obs.policy_margin_cp {
                    margin_sum += margin as f64;
                    margin_count += 1;
                }
                let bound_key = score_bound_str(obs.score_bound);
                *obs_score_bound_counts.entry(bound_key).or_default() += 1;
            }
            if let Some(swing) = score_swing(&cp_scores) {
                swing_sum += swing as f64;
                swing_count += 1;
                *swing_buckets
                    .entry((swing.div_euclid(50)) * 50)
                    .or_default() += 1;
            }
            // eval bucket from deepest observation, normalized to Black's perspective so the
            // histogram/cross-tabs share one reference frame regardless of whose turn each
            // record's position was -- otherwise "+400" would mean opposite things in different
            // rows of the same table.
            if let Some(deepest) = rec.observations.iter().max_by_key(|o| o.depth) {
                let key = match deepest.score {
                    Score::Cp { value } => {
                        let black_value = cp_from_black_perspective(
                            value,
                            deepest.score_perspective,
                            rec.tags.side_to_move,
                        );
                        // bucket width 200cp; clamp display at ±1400
                        (black_value.div_euclid(200)) * 200
                    }
                    Score::Mate { .. } => i32::MAX, // mate sentinel
                };
                *eval_buckets.entry(key).or_default() += 1;
                *eval_by_phase
                    .entry(key)
                    .or_default()
                    .entry(format!("{}", rec.tags.phase))
                    .or_default() += 1;
                *eval_by_side
                    .entry(key)
                    .or_default()
                    .entry(format!("{}", rec.tags.side_to_move))
                    .or_default() += 1;
            }

            accumulate_candidate_coverage(
                &rec,
                &mut obs_with_candidates,
                &mut obs_total,
                &mut score_bound_distribution,
            );
            accumulate_requested_depth(
                &rec,
                &mut requested_depth_total,
                &mut requested_depth_underreach,
            );
        }
    }

    if n == 0 {
        println!("no valid records in {:?}", args.input);
        return Ok(());
    }

    let duplicate_sfens: usize = sfen_counts
        .values()
        .filter(|&&c| c > 1)
        .map(|&c| c - 1)
        .sum();
    let duplicate_rate = duplicate_sfens as f64 / n as f64 * 100.0;

    let top_source_pct = sources.values().max().copied().unwrap_or(0) as f64 / n as f64 * 100.0;
    let opening_pct = phases.get("opening").copied().unwrap_or(0) as f64 / n as f64 * 100.0;
    let black_count = sides.get("black").copied().unwrap_or(0);
    let white_count = sides.get("white").copied().unwrap_or(0);

    println!("=== shogiesa report ===");
    println!("positions      : {n}");
    println!("broken lines   : {broken}");
    println!(
        "ply range      : {ply_min}–{ply_max} (avg {:.1})",
        ply_sum as f64 / n as f64
    );
    println!("invalid SFENs  : {invalid_sfens}");
    println!("duplicate SFENs: {duplicate_sfens}");
    println!("tag mismatches : {tag_mismatches}  (side_to_move vs SFEN)");
    println!();
    println!("schema versions: {schema_versions:?}");
    println!();
    println!("phase distribution:");
    for (phase, count) in &phases {
        println!(
            "  {phase:<12} {count:>6}  ({:.1}%)",
            *count as f64 / n as f64 * 100.0
        );
    }
    println!();
    println!("side to move:");
    for (side, count) in &sides {
        println!(
            "  {side:<12} {count:>6}  ({:.1}%)",
            *count as f64 / n as f64 * 100.0
        );
    }
    println!();
    println!("tag ratios:");
    println!(
        "  in-check       {in_check:>6}  ({:.1}%)",
        in_check as f64 / n as f64 * 100.0
    );
    println!(
        "  capture        {has_capture:>6}  ({:.1}%)",
        has_capture as f64 / n as f64 * 100.0
    );
    println!();
    println!("source files: {}", sources.len());
    for (path, count) in sources.iter().take(10) {
        println!("  {path}: {count}");
    }
    if sources.len() > 10 {
        println!("  … and {} more", sources.len() - 10);
    }
    println!();
    println!("source dominance:");
    let top_warn = if top_source_pct > 50.0 {
        "WARN: too concentrated"
    } else {
        "OK"
    };
    println!("  top source     : {top_source_pct:.1}%  {top_warn}");
    println!();
    println!("balance warnings:");
    let opening_warn = if opening_pct > 50.0 {
        "WARN: too high"
    } else {
        "OK"
    };
    println!("  opening ratio  : {opening_pct:.1}%  {opening_warn}");
    let (b_pct, w_pct) = (
        black_count as f64 / n as f64 * 100.0,
        white_count as f64 / n as f64 * 100.0,
    );
    let side_warn = if b_pct > 65.0 || w_pct > 65.0 {
        "WARN"
    } else {
        "OK"
    };
    println!("  side imbalance : {b_pct:.1}% / {w_pct:.1}%  {side_warn}");
    let dup_warn = if duplicate_rate > 5.0 {
        "WARN: too high"
    } else {
        "OK"
    };
    println!("  duplicate rate : {duplicate_rate:.1}%  {dup_warn}");

    // --- observation stats (only shown when any record has been labeled) ---
    let unlabeled = n - labeled;
    println!();
    println!("observations:");
    println!(
        "  labeled        : {labeled:>6}  ({:.1}%)",
        labeled as f64 / n as f64 * 100.0
    );
    println!(
        "  unlabeled      : {unlabeled:>6}  ({:.1}%)",
        unlabeled as f64 / n as f64 * 100.0
    );
    if labeled > 0 {
        println!(
            "  depth disagree : {depth_disagree:>6}  ({:.1}% of labeled)",
            depth_disagree as f64 / labeled as f64 * 100.0
        );
        if multi_engine > 0 {
            println!(
                "  engine disagree: {engine_disagree:>6}  ({:.1}% of {multi_engine} multi-engine positions)",
                engine_disagree as f64 / multi_engine as f64 * 100.0
            );
        }
        println!(
            "  special bestmove: {special_bestmove:>5}  ({:.1}% of labeled; resign/win/none)",
            special_bestmove as f64 / labeled as f64 * 100.0
        );
        println!("  depth counts:");
        for (&depth, &count) in &depth_counts {
            println!("    depth {depth:>2}     : {count:>6}");
        }
        let total_obs = cp_obs_count + mate_obs_count;
        println!(
            "  cp/mate ratio  : {cp_obs_count} cp / {mate_obs_count} mate  ({:.1}% mate)",
            mate_obs_count as f64 / total_obs.max(1) as f64 * 100.0
        );
        println!("  score bound (observations):");
        for (bound, count) in &obs_score_bound_counts {
            println!("    {bound:<10} : {count:>6}");
        }
        if swing_count > 0 {
            println!(
                "  avg score swing: {:.1}cp  (over {swing_count} records with \u{2265}2 cp observations)",
                swing_sum / swing_count as f64
            );
        }
        if margin_count > 0 {
            println!(
                "  avg policy margin: {:.1}cp  (over {margin_count} observations)",
                margin_sum / margin_count as f64
            );
        }
        if obs_with_candidates > 0 {
            println!(
                "  multipv coverage: {obs_with_candidates:>6}  ({:.1}% of {obs_total} observations)",
                obs_with_candidates as f64 / obs_total as f64 * 100.0
            );
            println!("  score bound (multipv candidates):");
            for (bound, count) in &score_bound_distribution {
                println!("    {bound:<10} : {count:>6}");
            }
        }
        if requested_depth_total > 0 {
            println!(
                "  requested-depth underreach: {requested_depth_underreach:>6}  ({:.1}% of {requested_depth_total} observations with a requested_depth)",
                requested_depth_underreach as f64 / requested_depth_total as f64 * 100.0
            );
        }
    }

    if !eval_buckets.is_empty() {
        let bar_max = eval_buckets.values().copied().max().unwrap_or(1);
        println!();
        println!("eval distribution (200cp buckets, deepest observation):");
        for (&key, &count) in &eval_buckets {
            let label = if key == i32::MIN {
                "  unlabeled  ".to_string()
            } else if key == i32::MAX {
                "  mate       ".to_string()
            } else {
                format!("  {:+5}..{:+5}", key, key + 199)
            };
            let bar = "█".repeat(count * 20 / bar_max.max(1));
            println!("{label}: {count:>5}  {bar}");
        }
    }

    if !swing_buckets.is_empty() {
        let bar_max = swing_buckets.values().copied().max().unwrap_or(1);
        println!();
        println!("score swing distribution (50cp buckets, per record):");
        for (&key, &count) in &swing_buckets {
            let bar = "█".repeat(count * 20 / bar_max.max(1));
            println!("  {key:>4}..{:<4}: {count:>5}  {bar}", key + 49);
        }
    }

    print_eval_cross_tab("eval bucket x phase", &eval_by_phase);
    print_eval_cross_tab("eval bucket x side", &eval_by_side);

    Ok(())
}

/// Prints a small fixed-width cross-tab of eval bucket (rows, same keying as the main eval
/// distribution histogram) against an arbitrary string dimension (columns, e.g. phase or side).
fn print_eval_cross_tab(title: &str, table: &BTreeMap<i32, BTreeMap<String, usize>>) {
    if table.is_empty() {
        return;
    }
    let columns: BTreeSet<String> = table.values().flat_map(|m| m.keys().cloned()).collect();
    println!();
    println!("{title}:");
    print!("  {:<14}", "");
    for col in &columns {
        print!("{col:>12}");
    }
    println!();
    for (&key, row) in table {
        let label = if key == i32::MIN {
            "unlabeled".to_string()
        } else if key == i32::MAX {
            "mate".to_string()
        } else {
            format!("{:+}..{:+}", key, key + 199)
        };
        print!("  {label:<14}");
        for col in &columns {
            print!("{:>12}", row.get(col).copied().unwrap_or(0));
        }
        println!();
    }
}

fn cmd_validate(args: ValidateArgs) -> Result<()> {
    // Why: validate is meant to run against multi-GB JSONL exports, so the input is read one
    // line at a time instead of buffering the whole file into memory.
    let file = File::open(&args.input).with_context(|| format!("cannot read {:?}", args.input))?;
    let reader = BufReader::new(file);

    let mut total_lines = 0usize;
    let mut valid_json = 0usize;
    let mut valid_records = 0usize;
    let mut tag_mismatches = 0usize;
    let mut invalid_sfens = 0usize;
    let mut schema_versions = BTreeMap::<u32, usize>::new();
    let mut seen_sfens: HashSet<String> = HashSet::new();
    let mut duplicate_sfens = 0usize;

    for line in reader.lines() {
        let line = line.with_context(|| format!("cannot read {:?}", args.input))?;
        if line.trim().is_empty() {
            continue;
        }
        total_lines += 1;

        let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        valid_json += 1;

        let Ok(rec) = serde_json::from_value::<PositionRecord>(val) else {
            continue;
        };
        valid_records += 1;

        *schema_versions.entry(rec.schema_version).or_default() += 1;

        if !seen_sfens.insert(rec.sfen.clone()) {
            duplicate_sfens += 1;
        }

        match Sfen::parse(&rec.sfen) {
            Ok(sfen) => {
                if sfen.side_to_move() != rec.tags.side_to_move {
                    tag_mismatches += 1;
                }
            }
            Err(_) => invalid_sfens += 1,
        }
    }

    let broken = total_lines - valid_json;
    let has_problems = tag_mismatches > 0 || broken > 0 || invalid_sfens > 0;

    println!("=== shogiesa validate ===");
    println!("total lines    : {total_lines}");
    println!("valid JSON     : {valid_json}");
    println!("valid records  : {valid_records}");
    println!("broken lines   : {broken}");
    println!("invalid SFENs  : {invalid_sfens}");
    println!("duplicate SFENs: {duplicate_sfens}");
    println!("tag mismatches : {tag_mismatches}  (side_to_move vs SFEN)");
    println!("schema versions: {schema_versions:?}");

    if has_problems {
        println!();
        if broken > 0 {
            println!("WARN: {broken} broken lines");
        }
        if invalid_sfens > 0 {
            println!("WARN: {invalid_sfens} invalid SFENs");
        }
        if tag_mismatches > 0 {
            println!("WARN: {tag_mismatches} side_to_move tag mismatches");
        }
        if args.strict {
            std::process::exit(1);
        }
    } else {
        println!();
        println!("OK");
    }
    Ok(())
}

#[cfg(test)]
mod label_pipeline_tests {
    use super::*;
    use shogiesa_core::{PositionTags, SourceInfo};

    fn rec(tag: &str) -> PositionRecord {
        PositionRecord::new(
            "startpos".to_string(),
            SourceInfo {
                kind: "test".to_string(),
                path: tag.to_string(),
                ply: 1,
                root_id: None,
                variation_id: None,
                branch_from_ply: None,
            },
            PositionTags {
                phase: GamePhase::Opening,
                side_to_move: SideToMove::Black,
                in_check: false,
                has_capture: false,
            },
        )
    }

    fn tag(record: &PositionRecord) -> &str {
        &record.source.path
    }

    #[test]
    fn reorder_push_holds_back_out_of_order_arrivals() {
        let mut pending = BTreeMap::new();
        let mut next_id = 0u64;

        // job 1 arrives before job 0 -- nothing is writable yet.
        let ready = reorder_push(&mut pending, &mut next_id, 1, rec("b"));
        assert!(ready.is_empty());
        assert_eq!(pending.len(), 1);

        // job 0 arrives -- both 0 and the already-buffered 1 become writable, in order.
        let ready = reorder_push(&mut pending, &mut next_id, 0, rec("a"));
        assert_eq!(ready.iter().map(tag).collect::<Vec<_>>(), vec!["a", "b"]);
        assert!(pending.is_empty());
        assert_eq!(next_id, 2);
    }

    #[test]
    fn reorder_push_restores_order_from_arbitrary_arrival_order() {
        let mut pending = BTreeMap::new();
        let mut next_id = 0u64;
        let arrivals = [(3, "d"), (1, "b"), (0, "a"), (2, "c")];

        let mut output = Vec::new();
        for (id, t) in arrivals {
            output.extend(reorder_push(&mut pending, &mut next_id, id, rec(t)));
        }
        assert_eq!(
            output.iter().map(tag).collect::<Vec<_>>(),
            vec!["a", "b", "c", "d"]
        );
    }

    #[test]
    fn reorder_push_buffer_size_equals_arrivals_ahead_of_next_id() {
        // reorder_push itself has no cap -- it will happily buffer every arrival ahead of
        // next_id, as this test shows by reaching 20. The permit scheme in `cmd_label` is what
        // bounds this in practice, by capping how many jobs can ever be dispatched-but-unwritten
        // at once (see the `debug_assert!` in cmd_label's writer loop). This test exists to
        // pin the exact quantity that scheme has to bound: pending.len() after N out-of-order
        // arrivals ahead of job 0 is N, not less -- so bounding "jobs in flight" really does
        // bound the reorder buffer.
        let mut pending = BTreeMap::new();
        let mut next_id = 0u64;
        let mut max_buffered = 0usize;

        for id in (1..=20u64).rev() {
            let ready = reorder_push(&mut pending, &mut next_id, id, rec("x"));
            assert!(ready.is_empty(), "job 0 hasn't arrived yet");
            max_buffered = max_buffered.max(pending.len());
        }
        assert_eq!(max_buffered, 20);

        let ready = reorder_push(&mut pending, &mut next_id, 0, rec("a"));
        assert_eq!(ready.len(), 21, "job 0 unblocks every buffered successor");
        assert!(pending.is_empty());
    }
}

#[cfg(test)]
mod extract_zobrist_dedup_tests {
    use super::*;
    use shogiesa_core::{PositionTags, SourceInfo};

    fn rec_with_sfen(sfen: &str) -> PositionRecord {
        PositionRecord::new(
            sfen.to_string(),
            SourceInfo {
                kind: "test".to_string(),
                path: "game".to_string(),
                ply: 1,
                root_id: None,
                variation_id: None,
                branch_from_ply: None,
            },
            PositionTags {
                phase: GamePhase::Opening,
                side_to_move: SideToMove::Black,
                in_check: false,
                has_capture: false,
            },
        )
    }

    #[test]
    fn two_distinct_unparseable_sfens_are_both_kept_and_counted_as_skipped() {
        let mut seen_hashes = HashSet::new();
        let mut skipped = 0usize;

        // Neither SFEN is valid, so neither should ever get inserted into seen_hashes --
        // an earlier `unwrap_or(0)` sentinel would have made the second one collide with the
        // first and be silently dropped as a "duplicate".
        let a = rec_with_sfen("not a valid sfen");
        let b = rec_with_sfen("also not valid");
        assert!(zobrist_from_sfen(&a.sfen).is_none());
        assert!(zobrist_from_sfen(&b.sfen).is_none());

        assert!(!zobrist_dedup_keep(&a, &mut seen_hashes, &mut skipped));
        assert!(!zobrist_dedup_keep(&b, &mut seen_hashes, &mut skipped));
        assert_eq!(skipped, 2, "both unparseable records counted as skipped");
        assert!(seen_hashes.is_empty(), "no sentinel hash was ever inserted");
    }

    #[test]
    fn valid_duplicate_sfen_is_deduped_normally() {
        let mut seen_hashes = HashSet::new();
        let mut skipped = 0usize;
        let sfen = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";

        assert!(zobrist_dedup_keep(
            &rec_with_sfen(sfen),
            &mut seen_hashes,
            &mut skipped
        ));
        assert!(!zobrist_dedup_keep(
            &rec_with_sfen(sfen),
            &mut seen_hashes,
            &mut skipped
        ));
        assert_eq!(skipped, 0, "a real duplicate isn't counted as skipped");
    }
}

#[cfg(test)]
mod fingerprint_tests {
    use super::*;

    // Why hard-coded expected values, not just "assert it matches itself": blake3's digest for a
    // fixed input is stable forever by spec (unlike the `DefaultHasher` this replaced, whose docs
    // explicitly disclaim cross-toolchain stability) -- a literal expected value is a real
    // regression guard, catching a future accidental change to the hashing scheme, not just
    // confirming the function is self-consistent.
    #[test]
    fn assign_split_bucket_is_a_stable_golden_value() {
        assert_eq!(
            assign_split_bucket(42, "some-fixed-key", 0.1, 0.1),
            SplitBucket::Train
        );
    }

    #[test]
    fn seeded_hash_is_a_stable_golden_value() {
        assert_eq!(seeded_hash(7, "startpos"), 13402537162744184401);
    }

    #[test]
    fn engine_options_hash_is_a_stable_golden_value() {
        assert_eq!(
            engine_options_hash(&[("MultiPV".to_string(), "4".to_string())]),
            1923589341701319780
        );
    }

    #[test]
    fn label_cache_key_is_a_64_char_hex_digest() {
        // Pins the format change from DefaultHasher's 16-hex-char digest to blake3's full
        // 64-hex-char digest -- catches an accidental revert to the old hasher immediately, even
        // before checking the golden value below.
        let cache = LabelCache {
            dir: PathBuf::from("/tmp/cache"),
            engine_options_hash: engine_options_hash(&[]),
            multipv: 1,
            engine_fingerprint: None,
            engine_fingerprint_mode: EngineFingerprintMode::None,
            hits: Arc::new(AtomicUsize::new(0)),
            misses: Arc::new(AtomicUsize::new(0)),
        };
        let path = label_cache_path(&cache, "startpos", "engine", Some("1.0"), 8);
        let key = path.file_stem().unwrap().to_str().unwrap();
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

#[cfg(test)]
mod label_cache_correctness_tests {
    use super::*;

    fn cache_with_fingerprint(fp: Option<u64>) -> LabelCache {
        LabelCache {
            dir: PathBuf::from("/tmp/cache"),
            engine_options_hash: engine_options_hash(&[]),
            multipv: 1,
            engine_fingerprint: fp,
            engine_fingerprint_mode: if fp.is_some() {
                EngineFingerprintMode::Content
            } else {
                EngineFingerprintMode::None
            },
            hits: Arc::new(AtomicUsize::new(0)),
            misses: Arc::new(AtomicUsize::new(0)),
        }
    }

    #[test]
    fn atomic_cache_write_never_leaves_a_torn_file_under_concurrent_writers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared_key.json");

        // Several threads race to write the same content-addressed key -- exactly the
        // documented "sharing a labeling budget across datasets" scenario, just compressed onto
        // one machine. None of them should ever leave a partially-written file visible.
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    write_cache_entry_atomically(&path, &format!(r#"{{"n":{i}}}"#))
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap().unwrap();
        }

        let content = fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content)
            .expect("file must always be complete, valid JSON, never a torn partial write");
        assert!(parsed["n"].is_number());

        let leftover_temp_files = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .count();
        assert_eq!(leftover_temp_files, 0, "no temp files should remain behind");
    }

    #[test]
    fn engine_fingerprint_content_mode_differs_for_different_binaries() {
        let dir = tempfile::tempdir().unwrap();
        let path_a = dir.path().join("engine_a");
        let path_b = dir.path().join("engine_b");
        fs::write(&path_a, b"binary A content").unwrap();
        fs::write(&path_b, b"binary B content").unwrap();

        let fp_a = compute_engine_fingerprint(EngineFingerprintMode::Content, &path_a);
        let fp_b = compute_engine_fingerprint(EngineFingerprintMode::Content, &path_b);
        assert!(fp_a.is_some());
        assert_ne!(
            fp_a, fp_b,
            "two different binaries must not silently share a cache identity"
        );
    }

    #[test]
    fn engine_fingerprint_none_mode_produces_no_fingerprint() {
        // The escape hatch: identical to today's original behavior of relying solely on the
        // USI-reported id name/version, regardless of what the binary on disk actually contains.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("engine");
        fs::write(&path, b"anything").unwrap();
        assert_eq!(
            compute_engine_fingerprint(EngineFingerprintMode::None, &path),
            None
        );
    }

    #[test]
    fn engine_fingerprint_falls_back_to_none_when_path_is_unreadable() {
        // A bare engine name (e.g. "sekirei") is resolved via PATH by std::process::Command at
        // spawn time, but fs::read/fs::canonicalize have no PATH search -- they'd fail on that
        // same bare name. Fingerprinting must degrade gracefully (not error out label entirely)
        // for a case that worked fine before fingerprinting existed.
        let unreadable = PathBuf::from("this-path-almost-certainly-does-not-exist-anywhere");
        assert_eq!(
            compute_engine_fingerprint(EngineFingerprintMode::Content, &unreadable),
            None
        );
        assert_eq!(
            compute_engine_fingerprint(EngineFingerprintMode::Metadata, &unreadable),
            None
        );
    }

    #[test]
    fn label_cache_path_differs_when_engine_fingerprint_differs() {
        let path_a = label_cache_path(
            &cache_with_fingerprint(Some(111)),
            "startpos",
            "engine",
            Some("1.0"),
            8,
        );
        let path_b = label_cache_path(
            &cache_with_fingerprint(Some(222)),
            "startpos",
            "engine",
            Some("1.0"),
            8,
        );
        assert_ne!(
            path_a, path_b,
            "a rebuilt engine binary must not collide with a stale cache entry"
        );
    }
}

#[cfg(test)]
mod calibrate_helper_tests {
    use super::*;
    use shogiesa_core::QualityReason;

    fn decision(reasons: Vec<QualityReason>) -> QualityDecision {
        QualityDecision {
            keep: reasons.is_empty(),
            score: 1.0,
            reasons,
            score_swing_cp: None,
            bestmove_agreement: true,
            cp_count: 0,
            mate_count: 0,
        }
    }

    #[test]
    fn coverage_tally_records_kept_and_first_reason_only() {
        let mut tally = CoverageTally::default();
        tally.record(&decision(vec![]));
        tally.record(&decision(vec![
            QualityReason::PolicyMargin,
            QualityReason::ScoreSwing,
        ]));
        assert_eq!(tally.total, 2);
        assert_eq!(tally.kept, 1);
        assert_eq!(tally.dropped, 1);
        // Only the first reason counts, matching evaluate_quality's own documented convention --
        // ScoreSwing must not also appear even though this decision failed both gates.
        assert_eq!(tally.drop_reasons_csv_cell(), "policy_margin=1");
        assert_eq!(tally.coverage_pct(), 50.0);
    }

    #[test]
    fn dataset_diagnostics_skips_unlabeled_records() {
        let rec: PositionRecord = serde_json::from_str(
            r#"{"schema_version":8,"sfen":"x","source":{"kind":"csa","path":"g.csa","ply":1},
                "tags":{"phase":"opening","side_to_move":"black","in_check":false,"has_capture":false},
                "observations":[]}"#,
        )
        .unwrap();
        let mut diagnostics = DatasetDiagnostics::default();
        diagnostics.record(&rec);
        assert_eq!(
            diagnostics.labeled, 0,
            "empty observations must not count as labeled"
        );
    }

    #[test]
    fn dataset_diagnostics_buckets_a_labeled_record() {
        let rec: PositionRecord = serde_json::from_str(
            r#"{"schema_version":8,"sfen":"x","source":{"kind":"csa","path":"g.csa","ply":1},
                "tags":{"phase":"opening","side_to_move":"black","in_check":false,"has_capture":false},
                "observations":[{"engine":"e","engine_version":null,"depth":4,"score":{"kind":"cp","value":50},
                "bestmove":"7g7f","nodes":null,"time_ms":null,"pv":null,"policy_margin_cp":80}]}"#,
        )
        .unwrap();
        let mut diagnostics = DatasetDiagnostics::default();
        diagnostics.record(&rec);
        assert_eq!(diagnostics.labeled, 1);
        assert_eq!(diagnostics.margin_buckets.get(&50), Some(&1)); // 80.div_euclid(50)*50 == 50
        assert_eq!(diagnostics.obs_score_bound_counts.get("exact"), Some(&1));
    }
}

#[cfg(test)]
mod tune_pareto_tests {
    use super::*;

    fn cell_with(id: i32, kept_pct: usize, mismatch_pct: usize) -> TuneCell {
        let mut cell = TuneCell::new(Some(id), None);
        cell.coverage.total = 100;
        cell.coverage.kept = kept_pct;
        cell.audit.pairs = 100;
        cell.audit.bestmove_mismatches = mismatch_pct;
        cell
    }

    #[test]
    fn pick_balanced_range_normalizes_before_computing_distance() {
        // Coverage spans a much wider range (20%..95%) across this frontier than mismatch_rate
        // (2%..8%) does. On RAW (non-normalized) values, distance-to-the-ideal-corner is
        // dominated by the coverage term regardless of L1/Euclidean, so an unnormalized
        // "balanced" pick collapses onto "broad" (highest coverage) -- this is the exact bug
        // caught in this feature's design review before it shipped. Range-normalizing each axis
        // to the frontier's own min/max first fixes it: the middle cell (60% coverage, 4%
        // mismatch) is the actual best trade-off, not the edge with the most data.
        let cells = vec![
            cell_with(0, 95, 8),
            cell_with(1, 60, 4),
            cell_with(2, 20, 2),
        ];
        let frontier = pareto_frontier_indices(&cells);
        assert_eq!(
            frontier,
            vec![0, 1, 2],
            "all three are mutually non-dominated: coverage and mismatch both decrease together"
        );
        assert_eq!(
            pick_balanced(&cells, &frontier),
            1,
            "the middle trade-off point, not the highest-coverage one"
        );
    }

    #[test]
    fn pareto_frontier_excludes_a_strictly_dominated_point() {
        let cells = vec![
            cell_with(0, 70, 0), // dominates cell 1: same mismatch, higher coverage
            cell_with(1, 30, 0),
        ];
        assert_eq!(pareto_frontier_indices(&cells), vec![0]);
    }

    #[test]
    fn pick_broad_and_strict_pick_opposite_ends() {
        let cells = vec![cell_with(0, 90, 10), cell_with(1, 40, 2)];
        let frontier = pareto_frontier_indices(&cells);
        assert_eq!(pick_broad(&cells, &frontier), 0);
        assert_eq!(pick_strict(&cells, &frontier), 1);
    }
}

#[cfg(test)]
mod merge_observations_tests {
    use super::*;

    fn obs(engine: &str, depth: u32, requested_depth: Option<u32>, bestmove: &str) -> Observation {
        Observation {
            engine: engine.to_string(),
            engine_version: None,
            depth,
            requested_depth,
            score: Score::Cp { value: 0 },
            score_perspective: ScorePerspective::default(),
            score_bound: shogiesa_core::ScoreBound::default(),
            bestmove: bestmove.to_string(),
            bestmove_kind: None,
            nodes: None,
            time_ms: None,
            pv: None,
            policy_margin_cp: None,
            candidates: Vec::new(),
        }
    }

    #[test]
    fn no_collision_appends_both() {
        let mut base = vec![obs("e1", 4, Some(4), "7g7f")];
        let incoming = vec![obs("e2", 4, Some(4), "3c3d")];
        let collisions =
            merge_observations_into(&mut base, incoming, MergeObservationPolicy::KeepBoth);
        assert_eq!(collisions, 0);
        assert_eq!(base.len(), 2);
    }

    #[test]
    fn keep_both_on_collision_keeps_both() {
        let mut base = vec![obs("e1", 4, Some(4), "7g7f")];
        let incoming = vec![obs("e1", 4, Some(4), "3c3d")]; // same (engine, depth, requested_depth)
        let collisions =
            merge_observations_into(&mut base, incoming, MergeObservationPolicy::KeepBoth);
        assert_eq!(collisions, 0); // KeepBoth never even checks for a collision
        assert_eq!(base.len(), 2);
    }

    #[test]
    fn prefer_primary_on_collision_drops_secondary() {
        let mut base = vec![obs("e1", 4, Some(4), "7g7f")];
        let incoming = vec![obs("e1", 4, Some(4), "3c3d")];
        let collisions =
            merge_observations_into(&mut base, incoming, MergeObservationPolicy::PreferPrimary);
        assert_eq!(collisions, 1);
        assert_eq!(base.len(), 1);
        assert_eq!(base[0].bestmove, "7g7f"); // primary's survives
    }

    #[test]
    fn prefer_secondary_on_collision_replaces_primary() {
        let mut base = vec![obs("e1", 4, Some(4), "7g7f")];
        let incoming = vec![obs("e1", 4, Some(4), "3c3d")];
        let collisions =
            merge_observations_into(&mut base, incoming, MergeObservationPolicy::PreferSecondary);
        assert_eq!(collisions, 1);
        assert_eq!(base.len(), 1);
        assert_eq!(base[0].bestmove, "3c3d"); // secondary's replaces primary's
    }

    #[test]
    fn multiple_simultaneous_collisions_all_resolved() {
        let mut base = vec![obs("e1", 4, Some(4), "7g7f"), obs("e2", 6, Some(6), "3c3d")];
        let incoming = vec![
            obs("e1", 4, Some(4), "aaaa"), // collides with base[0]
            obs("e2", 6, Some(6), "bbbb"), // collides with base[1]
            obs("e3", 8, Some(8), "cccc"), // no collision
        ];
        let collisions =
            merge_observations_into(&mut base, incoming, MergeObservationPolicy::PreferSecondary);
        assert_eq!(collisions, 2);
        assert_eq!(base.len(), 3);
        assert!(base.iter().any(|o| o.bestmove == "aaaa"));
        assert!(base.iter().any(|o| o.bestmove == "bbbb"));
        assert!(base.iter().any(|o| o.bestmove == "cccc"));
    }

    #[test]
    fn engine_version_mismatch_is_not_a_collision() {
        // merge-observations' key includes engine_version (unlike label's own narrower in-place
        // dedup key) precisely so two different engine versions at the same nominal depth are
        // never silently conflated.
        let mut base = vec![obs("e1", 4, Some(4), "7g7f")];
        let mut versioned = obs("e1", 4, Some(4), "3c3d");
        versioned.engine_version = Some("2.0".to_string());
        let collisions = merge_observations_into(
            &mut base,
            vec![versioned],
            MergeObservationPolicy::PreferPrimary,
        );
        assert_eq!(collisions, 0);
        assert_eq!(base.len(), 2);
    }

    #[test]
    fn shallow_and_deep_same_engine_never_collide_under_any_policy() {
        // The flagship use case this command exists for -- a shallow pass (depth 4) plus a
        // deeper relabel (depth 12) of the same engine -- has DIFFERENT depths, so it never
        // collides on (engine, engine_version, depth, requested_depth) no matter which
        // --on-collision policy is chosen: both observations always survive. `--on-collision`
        // only ever matters when two passes produced the exact same (engine, engine_version,
        // depth, requested_depth) tuple (e.g. a flaky re-run at the identical depth) -- it is
        // NOT a "the deeper one wins" switch, since depth is part of the key that determines
        // whether there's a collision to resolve in the first place.
        for policy in [
            MergeObservationPolicy::KeepBoth,
            MergeObservationPolicy::PreferPrimary,
            MergeObservationPolicy::PreferSecondary,
        ] {
            let mut base = vec![obs("e1", 4, Some(4), "7g7f")];
            let deep = vec![obs("e1", 12, Some(12), "3c3d")];
            let collisions = merge_observations_into(&mut base, deep, policy);
            assert_eq!(collisions, 0);
            assert_eq!(base.len(), 2, "both depths must survive under every policy");
        }
    }
}
