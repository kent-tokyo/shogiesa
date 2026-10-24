use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet};
use std::fmt::{Display, Write as _}; // writeln! into a String, for cmd_tune's Markdown report
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use shogiesa_core::{
    Board, GameOutcome, GamePhase, GameResultInfo, Observation, PieceType, PositionRecord,
    PositionTags, QualityConfig, QualityDecision, SCHEMA_VERSION, Score, ScorePerspective,
    SearchLimitKind, SideToMove, SourceInfo, UsiMove, bestmove_agreement,
    cp_from_black_perspective_i64, effective_bestmove_kind, engine_bestmove_agreement,
    evaluate_quality, has_special_bestmove,
    manifest::RunManifestV1 as RunManifest,
    parse_usi_move, phase_from_ply,
    quality_report::{
        QUALITY_REPORT_SCHEMA_VERSION, QualityInputIdentityV1, QualityManifestIdentityV1,
        QualityReportV1, QualityShardV1, QualitySummaryBuilder,
    },
    requested_depth_underreached,
    schema::{
        PositionRecordJsonlReader, PositionRecordLine, PositionRecordLineError,
        PositionRecordParseError, parse_json_line,
    },
    score_swing_i64,
    sfen::Sfen,
    zobrist_from_sfen,
};
use shogiesa_pack as pack;
use shogiesa_stratify::{EvalBucket, bucket_key, eval_bucket_of};
use shogiesa_usi::{SearchLimit, UsiEngine, UsiError};
use stratifykit_core::{
    BucketStatus, HeapEntry, QuotaSpec, TotalF32, bucket_floor, classify_bucket, group_aware_fill,
    mean_of, push_bounded, reservoir_sample, seeded_hash,
};
use tracing::info;

mod commands;
mod io_support;
mod recipe;

use io_support::{
    AtomicWriter, Sha256Reader, Sha256Writer, atomic_write_bytes, finish_hashed_position_jsonl,
    hash_file_sha256, open_hashed_position_jsonl, sha256_hex, validate_path_roles,
};

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
    /// Extract positions from CSA, KIF, or KI2 game records
    Extract(ExtractArgs),
    /// Label positions with engine evaluations
    Label(Box<LabelArgs>),
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
    /// Balance dataset distribution by phase / side / eval-bucket / wdl
    Balance(BalanceArgs),
    /// Quota-based sampling to a per-bucket target distribution (unlike `balance`'s single
    /// uniform target), group-aware so one source game/root can't dominate a bucket's fill.
    /// `--write-template` observes current bucket counts as a hand-editable starting point;
    /// `--quota` applies a (possibly hand-edited) quota file
    Stratify(StratifyArgs),
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
    /// Report disagreements between decisive game outcomes and teacher CP sign
    ConflictReport(ConflictReportArgs),
    /// Report fixed-size contiguous source-root blocks for diagnostic analysis
    BlockReport(BlockReportArgs),
    /// Report statistics about a positions dataset
    Report(ReportArgs),
    /// Emit a versioned machine-readable quality report for JSONL or pack inputs
    QualityReport(QualityReportArgs),
    /// Compare two datasets semantically, independent of JSONL input order
    DatasetDiff(DatasetDiffArgs),
    /// Plan, run, resume, or verify a typed multi-stage dataset recipe
    Recipe(recipe::RecipeArgs),
    /// Report phase/side/eval-bucket/ply/source-root distribution, explicitly flagging bucket
    /// combinations that have zero records within the observed range -- distinct from `select
    /// --strategy coverage` (which *ranks* existing records by thin-bucket membership for
    /// re-labeling) and from the unrelated MultiPV/quality-gate "coverage" used by `report`/
    /// `calibrate`/`audit`/`tune`. Named `distribution`, not `coverage`, to avoid colliding with
    /// either.
    Distribution(DistributionArgs),
    /// Validate data integrity of a positions dataset
    Validate(ValidateArgs),
    /// Inspect/maintain a `label --cache-dir` cache
    Cache(CacheArgs),
    /// Extract positions from a match runner's per-game USI kifu text files. This command only
    /// extracts; feed its output through `label`, `select`, or `filter` as needed.
    FromMatch(FromMatchArgs),
    /// Merge two labeled positions JSONL files' observations (e.g. a shallow pass and a deeper
    /// relabel pass), with configurable duplicate-observation resolution
    MergeObservations(MergeObservationsArgs),
    /// Build a diverse, root-aware opening suite (plain SFEN-per-line, no schema wrapper) from a
    /// positions dataset for an external match runner. Input need not be labeled because only
    /// `sfen` and `source` are read
    MakeGateOpenings(MakeGateOpeningsArgs),
    /// Deterministically shuffle a positions dataset into training order, optionally writing a
    /// sidecar `--order-manifest` (training_position -> sample_id/game_id/outcome/teacher_cp/
    /// .../block_id) plus an `order_hash` so a later run can be verified to have produced the
    /// identical sequence, not just "used the same seed" -- the first command in this codebase
    /// that actually permutes output order (`sample`/`select`/`stratify`/`split` all preserve
    /// input-line or rank order)
    Shuffle(ShuffleArgs),
    /// Nested `lineprior` subcommands -- currently only `export`, for offline dogfooding of the
    /// external `lineprior` tool (not part of this repo). Measurement-only: does not integrate
    /// into Sekirei search.
    #[command(name = "lineprior")]
    LinePrior(LinePriorArgs),
}

#[derive(clap::Args)]
struct ExtractArgs {
    /// Input game-record file or directory (directory traversal is shallow by default)
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
    /// Recursively discover .csa, .kif, and .ki2 files; symlinks are not followed
    #[arg(long)]
    recursive: bool,
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
struct QualityReportArgs {
    /// Input JSONL or .shgpk file; repeat for deterministic multi-shard aggregation
    #[arg(short, long, required = true)]
    input: Vec<PathBuf>,
    /// Output JSON report
    #[arg(short, long)]
    out: PathBuf,
    /// Input format; auto recognizes .shgpk and the pack magic header
    #[arg(long, default_value = "auto", value_parser = ["auto", "jsonl", "pack"])]
    input_format: String,
    /// Optional run manifest paired by position with each --input; repeat once per input
    #[arg(long)]
    manifest: Vec<PathBuf>,
    /// Include a semantic summary for each input shard in addition to the aggregate
    #[arg(long)]
    include_shards: bool,
}

#[derive(clap::Args)]
struct DatasetDiffArgs {
    /// Reference dataset JSONL
    #[arg(long)]
    baseline: PathBuf,
    /// Dataset JSONL to compare against the baseline
    #[arg(long)]
    candidate: PathBuf,
    /// Record identity: occurrence uses sfen+source path+ply; position uses sfen only
    #[arg(long, default_value = "occurrence", value_parser = ["occurrence", "position"])]
    match_by: String,
    /// Write the complete deterministic comparison report as JSON
    #[arg(long)]
    json_out: Option<PathBuf>,
}

#[derive(clap::Args)]
struct ConflictReportArgs {
    /// Input JSONL file
    #[arg(short, long)]
    input: PathBuf,
    /// Ignore CP values whose absolute value is at or below this deadband
    #[arg(long, default_value = "0")]
    min_abs_cp: i32,
}

#[derive(clap::Args)]
struct BlockReportArgs {
    /// Input JSONL file
    #[arg(short, long)]
    input: PathBuf,
    /// Positions per block; a source-root change also flushes the current partial block
    #[arg(long, default_value = "32")]
    block_size: usize,
}

#[derive(clap::Args)]
struct DistributionArgs {
    /// Input JSONL file
    #[arg(short, long)]
    input: PathBuf,
    /// Ply histogram bucket width
    #[arg(long, default_value = "20")]
    ply_bucket_size: u32,
    /// Flag a bucket as underrepresented when its count is below this fraction of the mean count
    /// (computed over buckets that have >=1 record, within the same distribution)
    #[arg(long, default_value = "0.5")]
    under_ratio: f64,
    /// Flag a bucket as overrepresented when its count is above this multiple of the mean count
    #[arg(long, default_value = "2.0")]
    over_ratio: f64,
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
    /// Comma-separated search depths, e.g. "4,6,8". Exactly one of --depths/--nodes is required.
    #[arg(long, required_unless_present = "nodes", conflicts_with = "nodes")]
    depths: Option<String>,
    /// Comma-separated fixed node-count search limits, e.g. "50000,200000" -- alternative to
    /// --depths for comparing different engines/versions, where "depth" doesn't mean a
    /// comparable amount of search work across them. Sends `go nodes N` instead of `go depth N`;
    /// populates search_limit_kind="nodes" and requested_nodes -- the existing depth/nodes
    /// fields still record whatever the engine actually reported reaching.
    #[arg(long, required_unless_present = "depths", conflicts_with = "depths")]
    nodes: Option<String>,
    /// Per-depth (or per-node-budget) timeout in milliseconds
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
    /// Merge in already-labeled observations from a previous (possibly interrupted) run's --out
    /// file, matched on (sfen, source.path, source.ply) -- the same alignment key
    /// `merge-observations` uses. Positions already covered to the requested depth are then
    /// skipped automatically (same effect as --skip-existing) unless --replace-existing is also
    /// given. The path doesn't need to exist yet -- a first, not-yet-interrupted run just no-ops.
    /// Must not be the same path as --out. Unlike `merge-observations`, this is not a union: only
    /// --input is iterated, so --input must be the full original corpus (a superset of whatever
    /// --resume-from contains) -- a record present only in --resume-from is silently dropped, not
    /// carried through.
    #[arg(long)]
    resume_from: Option<PathBuf>,
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
    /// SHA-256 of this file is recorded as weight_sha256 on every observation this run produces,
    /// and in --manifest if given -- e.g. the NNUE weight file the engine is configured to use
    /// via --engine-option EvalFile=... shogiesa does not read or interpret the file's contents
    /// beyond hashing it. Must be a single file, not a directory.
    #[arg(long)]
    weight_file: Option<PathBuf>,
    /// Opaque tag echoed into shogiesa's own --manifest verbatim; shogiesa doesn't interpret or
    /// validate it. Downstream adapters may use it as a join key, but it is not a shared schema.
    #[arg(long)]
    experiment_id: Option<String>,
    /// Opaque tag echoed into --manifest verbatim; shogiesa doesn't interpret or validate it.
    #[arg(long)]
    candidate_id: Option<String>,
    /// Opaque tag echoed into --manifest verbatim; shogiesa doesn't interpret or validate it.
    #[arg(long)]
    baseline_id: Option<String>,
    /// Opaque tag echoed into --manifest verbatim; shogiesa doesn't interpret or validate it.
    #[arg(long)]
    lineage_id: Option<String>,
    /// Opaque tag echoed into --manifest verbatim; shogiesa doesn't interpret or validate it --
    /// identifies an upstream teacher-manifest this run's input was derived from, if any.
    #[arg(long)]
    teacher_manifest_sha256: Option<String>,
    /// Opaque value echoed into --manifest verbatim; shogiesa doesn't interpret or validate it.
    #[arg(long)]
    init_seed: Option<u64>,
    /// Opaque value echoed into --manifest verbatim; shogiesa doesn't interpret or validate it.
    #[arg(long)]
    split_seed: Option<u64>,
    /// Opaque tag echoed into --manifest verbatim; shogiesa doesn't interpret or validate it --
    /// shogiesa can't itself tell whether --input is a split, so this is passthrough only.
    #[arg(long)]
    split_sha256: Option<String>,
    /// Opaque tag echoed into --manifest verbatim; shogiesa doesn't interpret or validate it.
    #[arg(long)]
    validity: Option<String>,
    /// Detect USI protocol desync: a bestmove with no matching `go`, a duplicate bestmove for
    /// the same `go`, a bestmove that arrives after its search was already timeout-salvaged, or
    /// a bestmove that isn't a legal move in the position it was given. Without this flag, such
    /// responses are silently trusted -- the real-world bug this guards against is a desynced or
    /// half-dead engine process producing plausible-looking but wrong bestmoves for the rest of
    /// an unattended run. Detected violations are counted in --manifest regardless of whether
    /// --restart-on-protocol-error is also given.
    #[arg(long)]
    usi_strict: bool,
    /// Relaunch the engine process after every N searches (`go` calls), regardless of whether
    /// anything went wrong -- a periodic hedge against slow resource growth in long unattended
    /// runs, independent of --usi-strict/--restart-on-protocol-error.
    #[arg(long)]
    restart_engine_every: Option<u32>,
    /// Relaunch the engine after a search error that isn't proof the process itself is dead (a
    /// timeout, an unparseable response, or -- with --usi-strict -- a detected protocol
    /// violation). An I/O error (broken pipe, exited process) always triggers a relaunch
    /// regardless of this flag, since there's nothing left to reuse. Without this flag, a
    /// position that errors for a non-I/O reason is left unlabeled and the same (possibly still
    /// confused) engine process is reused for the next position.
    #[arg(long)]
    restart_on_protocol_error: bool,
    /// On any search error -- a timeout, an I/O failure, or (with --usi-strict) a detected
    /// protocol violation -- write the raw USI exchange since the last clean `go` to a file
    /// under this directory (`{nanos}_worker{id}_{kind}.log`), for post-mortem debugging of an
    /// unattended run. Independent of --usi-strict: works the same without it, just without the
    /// extra violation kinds strict mode can detect.
    #[arg(long)]
    transcript_on_error: Option<PathBuf>,
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
struct ShuffleArgs {
    /// Input positions JSONL
    #[arg(short, long)]
    input: PathBuf,
    /// Output JSONL file -- same schema as input, records re-emitted in shuffled order
    #[arg(short, long)]
    out: PathBuf,
    /// Seed for the deterministic shuffle
    #[arg(long, default_value = "0")]
    seed: u64,
    /// Positions per block, for --order-manifest's block_id column
    #[arg(long, default_value = "32")]
    block_size: u64,
    /// Write a training-order manifest (JSONL, one line per output position) to this path
    #[arg(long = "order-manifest")]
    order_manifest: Option<PathBuf>,
    /// Engine whose observation is treated as the teacher for --order-manifest's teacher_cp/
    /// teacher_depth columns. Requires --teacher-depth too; omit both to leave those columns null
    #[arg(long, requires = "teacher_depth")]
    teacher_engine: Option<String>,
    /// Depth to select the teacher observation at (see --teacher-engine)
    #[arg(long, requires = "teacher_engine")]
    teacher_depth: Option<u32>,
    /// Opaque split label echoed into every --order-manifest line's split_id column -- this
    /// command doesn't detect which split a file is, pass what you already know (e.g. "train")
    #[arg(long)]
    split_id: Option<String>,
    /// Opaque tag echoed into --manifest, for correlating this run with an external pipeline run
    #[arg(long)]
    experiment_id: Option<String>,
    /// Write a run manifest (git sha, input hash, counts, order_hash) to this path
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
    /// Dimension(s) to balance by: phase, side, eval-bucket, wdl (repeatable)
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
struct StratifyArgs {
    /// Input positions JSONL
    #[arg(short, long)]
    input: PathBuf,
    /// Write an observed-bucket-count quota template to this JSON path and exit, instead of
    /// running the quota-fill. Requires --by. Hand-edit the written "quotas" counts down to the
    /// desired per-bucket targets, then feed the file to --quota.
    #[arg(long, conflicts_with_all = ["quota", "out", "manifest"])]
    write_template: Option<PathBuf>,
    /// Dimension(s) to bucket by when writing a template: phase, side, eval-bucket, wdl (repeatable,
    /// same tokens as `balance --by`). Write-template-only -- `--quota` reads this from the quota
    /// file's own "by" field instead, so a stale/mismatched --by can never be passed at apply
    /// time.
    #[arg(long = "by", value_name = "DIMENSION", conflicts_with = "quota")]
    by: Vec<String>,
    /// Apply a --write-template-produced (and hand-edited) quota file to --input, writing the
    /// quota-shaped, group-aware subset to --out.
    #[arg(long, conflicts_with = "write_template")]
    quota: Option<PathBuf>,
    /// Output JSONL file (required with --quota)
    #[arg(short, long)]
    out: Option<PathBuf>,
    /// Seed for deterministic root/tie-break ordering within an over-quota bucket (--quota mode)
    #[arg(long, default_value = "0")]
    seed: u64,
    /// Write a run manifest (git sha, input hash, counts, drop reasons, coverage, root-diversity
    /// stats) to this path (--quota mode only)
    #[arg(long)]
    manifest: Option<PathBuf>,
}

#[derive(clap::Args)]
struct MakeGateOpeningsArgs {
    /// Input positions JSONL (extract/from-match output; labeled or not, observations are never
    /// inspected)
    #[arg(short, long)]
    input: PathBuf,
    /// Output plain-text file, one SFEN per line -- ready for an external match-runner's own
    /// opening-book flag (e.g. Sekirei's `--positions FILE`). Line order is deterministic
    /// input-encounter order (not seed-shuffled), stable across repeated runs for a fixed
    /// `--input`/`--seed`/filter/`--count` combination -- `--seed` only breaks ties over which
    /// records win a `--count` quota, it doesn't reorder the output. Changing `--count` changes
    /// which records survive the quota, so a line's position in the file is not a stable
    /// cross-run join key on its own.
    #[arg(short, long)]
    out: PathBuf,
    /// Target suite size (quota)
    #[arg(long)]
    count: usize,
    /// Drop positions below this ply (a very early position offers little opening variety)
    #[arg(long, default_value = "8")]
    min_ply: u32,
    /// Drop positions above this ply (unbounded by default -- pair with `filter`/`mine` first if
    /// the input is a late-game-heavy corpus, e.g. loss-mined data, so "opening" stays accurate)
    #[arg(long)]
    max_ply: Option<u32>,
    /// Seed for deterministic root tie-breaking (default 0), same convention as sample/stratify
    #[arg(long, default_value = "0")]
    seed: u64,
    /// Write a run manifest (git sha, input hash, counts, drop reasons, root-diversity stats) to
    /// this path
    #[arg(long)]
    manifest: Option<PathBuf>,
    /// Keep positions where the side to move has zero legal moves (checkmate/stalemate-adjacent
    /// terminal positions) instead of dropping them. Off by default.
    #[arg(long)]
    allow_unplayable: bool,
}

#[derive(clap::Args)]
struct LinePriorArgs {
    #[command(subcommand)]
    action: LinePriorAction,
}

#[derive(clap::Subcommand)]
enum LinePriorAction {
    /// Export CSA/KIF game records into lineprior-compatible JSONL observations (one line per
    /// move played), for offline dogfooding of the external `lineprior` tool. Measurement-only
    /// -- does not integrate into Sekirei search.
    Export(LinePriorExportArgs),
}

#[derive(clap::Args)]
struct LinePriorExportArgs {
    /// Input file or directory of CSA/KIF game records
    #[arg(long)]
    input: PathBuf,
    /// Output JSONL file (one lineprior observation per move played)
    #[arg(long)]
    out: PathBuf,
    /// State representation -- only "sfen" is supported (placeholder for forward compat)
    #[arg(long, default_value = "sfen", value_parser = ["sfen"])]
    state_format: String,
    /// Action representation -- only "usi" is supported (placeholder for forward compat)
    #[arg(long, default_value = "usi", value_parser = ["usi"])]
    action_format: String,
    /// Maximum ply to export per game (outcome is still resolved from the whole game even when
    /// this truncates emission)
    #[arg(long)]
    max_ply: Option<u32>,
    /// Arbitrary label written to every observation's `source` field
    #[arg(long)]
    source: String,
    /// How `outcome` is derived -- only "game-result" is supported (placeholder for forward
    /// compat)
    #[arg(long, default_value = "game-result", value_parser = ["game-result"])]
    outcome_mode: String,
    /// How `score` is derived -- only "none" is supported (v1 has no engine-eval scoring)
    #[arg(long, default_value = "none", value_parser = ["none"])]
    score_mode: String,
    /// Write a run manifest (git sha, input hash, record/sequence counts, outcome and phase-tag
    /// distributions) to this path
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
    /// (hard strategy) assume `--input` is already contiguously grouped by source game (e.g.
    /// `extract`'s own output, or anything already run through `split`) -- lets `hard` bound
    /// memory by one game's positions plus the top-`--count` heap instead of materializing the
    /// whole dataset. Off by default: an incorrectly-set flag on genuinely ungrouped/interleaved
    /// input silently computes wrong blunder windows (a game split across two non-adjacent
    /// buffered chunks looks like two separate one-position "games" to the blunder detector).
    #[arg(long)]
    assume_grouped_by_source: bool,
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
    /// Exclude positions where any observation is a timeout-salvaged, degraded-but-real result
    /// (label's --timeout-ms elapsed before bestmove arrived) rather than a full completion or a
    /// genuine engine-initiated early stop
    #[arg(long)]
    exclude_timeout_salvaged: bool,
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
    /// A timeout-salvaged observation carrying a mate score does not get
    /// --require-requested-depth-reached's usual mate exemption by default -- it's less
    /// trustworthy than a genuine early-stop-on-forced-mate. Pass this to restore the blanket
    /// exemption for salvaged mates too. No effect unless --require-requested-depth-reached is
    /// also set.
    #[arg(long)]
    allow_timeout_salvaged_mate: bool,
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
        "exclude_timeout_salvaged", "max_score_swing_cp", "eval_min", "eval_max",
        "min_policy_margin_cp", "require_exact_score", "require_policy_margin",
        "min_depth_reached", "require_requested_depth_reached", "allow_timeout_salvaged_mate",
        "require_engine_agreement", "max_engine_score_swing_cp", "require_bestmove_agreement",
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
    /// Write input/output hashes, command arguments, schema, and counts to this manifest
    #[arg(long)]
    manifest: Option<PathBuf>,
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
    #[arg(long)]
    exclude_timeout_salvaged: bool,
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
    #[arg(long)]
    allow_timeout_salvaged_mate: bool,
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
    /// Write input/output hashes, command arguments, schema, and counts to this manifest
    #[arg(long)]
    manifest: Option<PathBuf>,
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
    #[arg(long)]
    exclude_timeout_salvaged: bool,
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
    #[arg(long)]
    allow_timeout_salvaged_mate: bool,
    /// Output CSV: one row per (policy_margin, score_swing) grid configuration
    #[arg(short, long)]
    out: PathBuf,
    /// Write input/output hashes, command arguments, schema, and counts to this manifest
    #[arg(long)]
    manifest: Option<PathBuf>,
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

/// Central path-safety gate for every command that writes a caller-selected file. Keeping this
/// next to dispatch makes it harder for a new command to accidentally truncate an input before
/// its own parsing/validation has a chance to run.
fn validate_command_paths(command: &Commands) -> Result<()> {
    match command {
        Commands::Extract(args) => {
            validate_path_roles(&[("--input", &args.input)], &[("--out", &args.out)])
        }
        Commands::FromMatch(args) => {
            validate_path_roles(&[("--input", &args.input)], &[("--out", &args.out)])
        }
        Commands::MergeObservations(args) => validate_path_roles(
            &[
                ("--primary", &args.primary),
                ("--secondary", &args.secondary),
            ],
            &[("--out", &args.out)],
        ),
        Commands::Label(args) => {
            let mut reads = vec![
                ("--input", args.input.as_path()),
                ("--engine", args.engine.as_path()),
            ];
            if let Some(path) = &args.resume_from {
                reads.push(("--resume-from", path));
            }
            if let Some(path) = &args.weight_file {
                reads.push(("--weight-file", path));
            }
            let mut writes = vec![("--out", args.out.as_path())];
            if let Some(path) = &args.manifest {
                writes.push(("--manifest", path));
            }
            validate_path_roles(&reads, &writes)
        }
        Commands::Stability(args) => {
            validate_path_roles(&[("--input", &args.input)], &[("--out", &args.out)])
        }
        Commands::Split(args) => {
            if let (Some(train), Some(valid), Some(test)) = (&args.train, &args.valid, &args.test) {
                let manifest = train
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join("manifest.json");
                validate_path_roles(
                    &[("--input", &args.input)],
                    &[
                        ("--train", train),
                        ("--valid", valid),
                        ("--test", test),
                        ("split manifest", &manifest),
                    ],
                )
            } else if let Some(out_dir) = &args.out_dir {
                let manifest = out_dir.join("manifest.json");
                validate_path_roles(
                    &[("--input", &args.input)],
                    &[("split manifest", &manifest)],
                )
            } else {
                Ok(())
            }
        }
        Commands::Sample(args) => {
            let mut writes = vec![("--out", args.out.as_path())];
            if let Some(path) = &args.manifest {
                writes.push(("--manifest", path));
            }
            validate_path_roles(&[("--input", &args.input)], &writes)
        }
        Commands::Shuffle(args) => {
            let mut writes = vec![("--out", args.out.as_path())];
            if let Some(path) = &args.order_manifest {
                writes.push(("--order-manifest", path));
            }
            if let Some(path) = &args.manifest {
                writes.push(("--manifest", path));
            }
            validate_path_roles(&[("--input", &args.input)], &writes)
        }
        Commands::Mine(args) => {
            validate_path_roles(&[("--input", &args.input)], &[("--out", &args.out)])
        }
        Commands::Balance(args) => {
            let mut writes = vec![("--out", args.out.as_path())];
            if let Some(path) = &args.manifest {
                writes.push(("--manifest", path));
            }
            validate_path_roles(&[("--input", &args.input)], &writes)
        }
        Commands::Stratify(args) => {
            let mut reads = vec![("--input", args.input.as_path())];
            if let Some(path) = &args.quota {
                reads.push(("--quota", path));
            }
            let mut writes = Vec::new();
            if let Some(path) = &args.write_template {
                writes.push(("--write-template", path.as_path()));
            }
            if let Some(path) = &args.out {
                writes.push(("--out", path));
            }
            if let Some(path) = &args.manifest {
                writes.push(("--manifest", path));
            }
            validate_path_roles(&reads, &writes)
        }
        Commands::MakeGateOpenings(args) => {
            let mut writes = vec![("--out", args.out.as_path())];
            if let Some(path) = &args.manifest {
                writes.push(("--manifest", path));
            }
            validate_path_roles(&[("--input", &args.input)], &writes)
        }
        Commands::Select(args) => {
            validate_path_roles(&[("--input", &args.input)], &[("--out", &args.out)])
        }
        Commands::Pack(args) => {
            let mut writes = vec![("--out", args.out.as_path())];
            if let Some(path) = &args.manifest {
                writes.push(("--manifest", path));
            }
            validate_path_roles(&[("--input", &args.input)], &writes)
        }
        Commands::Unpack(args) => {
            validate_path_roles(&[("--input", &args.input)], &[("--out", &args.out)])
        }
        Commands::QualityReport(args) => {
            let mut reads = args
                .input
                .iter()
                .map(|path| ("--input", path.as_path()))
                .collect::<Vec<_>>();
            reads.extend(
                args.manifest
                    .iter()
                    .map(|path| ("--manifest", path.as_path())),
            );
            validate_path_roles(&reads, &[("--out", &args.out)])
        }
        Commands::Filter(args) => {
            let mut reads = vec![("--input", args.input.as_path())];
            if let Some((preset_path, _)) = args
                .preset
                .as_deref()
                .and_then(|spec| spec.rsplit_once(':'))
            {
                reads.push(("--preset", Path::new(preset_path)));
            }
            let mut writes = Vec::new();
            if let Some(path) = &args.out {
                writes.push(("--out", path.as_path()));
            }
            if let Some(path) = &args.explain_out {
                writes.push(("--explain-out", path));
            }
            if let Some(path) = &args.manifest {
                writes.push(("--manifest", path));
            }
            validate_path_roles(&reads, &writes)
        }
        Commands::Calibrate(args) => {
            let mut writes = vec![("--out", args.out.as_path())];
            if let Some(path) = &args.manifest {
                writes.push(("--manifest", path));
            }
            validate_path_roles(&[("--input", &args.input)], &writes)
        }
        Commands::Audit(args) => {
            let mut writes = vec![("--out", args.out.as_path())];
            if let Some(path) = &args.manifest {
                writes.push(("--manifest", path));
            }
            validate_path_roles(&[("--input", &args.input)], &writes)
        }
        Commands::Tune(args) => {
            let mut writes = vec![("--out", args.out.as_path())];
            if let Some(path) = &args.manifest {
                writes.push(("--manifest", path));
            }
            if let Some(path) = &args.report {
                writes.push(("--report", path));
            }
            if let Some(path) = &args.preset_out {
                writes.push(("--preset-out", path));
            }
            validate_path_roles(&[("--input", &args.input)], &writes)
        }
        Commands::DatasetDiff(args) => {
            let writes = args
                .json_out
                .as_deref()
                .map(|path| vec![("--json-out", path)])
                .unwrap_or_default();
            validate_path_roles(
                &[
                    ("--baseline", &args.baseline),
                    ("--candidate", &args.candidate),
                ],
                &writes,
            )
        }
        Commands::LinePrior(args) => match &args.action {
            LinePriorAction::Export(args) => {
                let mut writes = vec![("--out", args.out.as_path())];
                if let Some(path) = &args.manifest {
                    writes.push(("--manifest", path));
                }
                validate_path_roles(&[("--input", &args.input)], &writes)
            }
        },
        Commands::ConflictReport(_)
        | Commands::BlockReport(_)
        | Commands::Report(_)
        | Commands::Recipe(_)
        | Commands::Distribution(_)
        | Commands::Validate(_)
        | Commands::Cache(_) => Ok(()),
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    validate_command_paths(&cli.command)?;
    match cli.command {
        Commands::Extract(args) => commands::cmd_extract(args),
        Commands::Label(args) => cmd_label(*args),
        Commands::Stability(args) => cmd_stability(args),
        Commands::Split(args) => cmd_split(args),
        Commands::Sample(args) => cmd_sample(args),
        Commands::Mine(args) => cmd_mine(args),
        Commands::Balance(args) => cmd_balance(args),
        Commands::Stratify(args) => cmd_stratify(args),
        Commands::Select(args) => cmd_select(args),
        Commands::Pack(args) => commands::cmd_pack(args),
        Commands::Unpack(args) => commands::cmd_unpack(args),
        Commands::Filter(args) => commands::cmd_filter(args),
        Commands::Calibrate(args) => commands::cmd_calibrate(args),
        Commands::Audit(args) => commands::cmd_audit(args),
        Commands::Tune(args) => commands::cmd_tune(args),
        Commands::ConflictReport(args) => commands::cmd_conflict_report(args),
        Commands::BlockReport(args) => commands::cmd_block_report(args),
        Commands::Report(args) => commands::cmd_report(args),
        Commands::QualityReport(args) => commands::cmd_quality_report(args),
        Commands::DatasetDiff(args) => commands::cmd_dataset_diff(args),
        Commands::Recipe(args) => recipe::run(args),
        Commands::Distribution(args) => commands::cmd_distribution(args),
        Commands::Validate(args) => commands::cmd_validate(args),
        Commands::Cache(args) => match args.action {
            CacheAction::Stats(a) => commands::cmd_cache_stats(a),
            CacheAction::Verify(a) => commands::cmd_cache_verify(a),
            CacheAction::Prune(a) => commands::cmd_cache_prune(a),
        },
        Commands::FromMatch(args) => commands::cmd_from_match(args),
        Commands::MergeObservations(args) => commands::cmd_merge_observations(args),
        Commands::MakeGateOpenings(args) => cmd_make_gate_openings(args),
        Commands::Shuffle(args) => cmd_shuffle(args),
        Commands::LinePrior(args) => match args.action {
            LinePriorAction::Export(a) => cmd_lineprior_export(a),
        },
    }
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

/// Sorts (and borrows) engine option pairs -- shared by `engine_options_hash` (u64, label-cache
/// key) and `engine_options_hash_hex` (full hex digest, cross-repo Observation/RunManifest use)
/// so the two never drift on what counts as "the same options" (sort order, and whether the
/// CLI-synthesized MultiPV entry is included -- it is, since it's already in `engine_options` by
/// the time either function is called).
fn sorted_engine_option_bytes(options: &[(String, String)]) -> Vec<&[u8]> {
    let mut sorted: Vec<&(String, String)> = options.iter().collect();
    sorted.sort();
    let mut parts: Vec<&[u8]> = Vec::with_capacity(sorted.len() * 2);
    for (k, v) in &sorted {
        parts.push(k.as_bytes());
        parts.push(v.as_bytes());
    }
    parts
}

/// Hash of the resolved USI engine options, sorted so option order doesn't change the label
/// cache key below.
fn engine_options_hash(options: &[(String, String)]) -> u64 {
    hash_parts_u64(&sorted_engine_option_bytes(options))
}

/// Full (untruncated) blake3 hex digest of the same input `engine_options_hash` hashes.
/// `engine_options_hash` (u64) stays truncated for the on-disk cache key/`CacheEntry`, which
/// never leaves this process; this hex form is for `Observation.engine_options_hash`/
/// `RunManifest`, which do leave it (shogiesa -> quietset -> lineprior -> veridict) -- a
/// truncated u64 exceeds 2^53 almost always and a JS/TS consumer's `JSON.parse` would silently
/// mangle it as an IEEE-754 double.
fn engine_options_hash_hex(options: &[(String, String)]) -> String {
    hash_parts(&sorted_engine_option_bytes(options))
        .to_hex()
        .to_string()
}

/// SHA-256 of `--weight-file`'s bytes, for `weight_sha256` -- errors clearly if given a
/// directory, since the envelope field is defined over one file's bytes.
fn compute_weight_sha256(path: &Path) -> Result<String> {
    if path.is_dir() {
        anyhow::bail!("weight_sha256 requires a single file, got a directory: {path:?}");
    }
    hash_file_sha256(path)
}

/// SHA-256 of the engine binary's raw bytes, for `RunManifest.binary_sha256`. Deliberately a
/// separate `fs::read` from `compute_engine_fingerprint`'s own Content-mode read (a different,
/// blake3-truncated value for an unrelated purpose -- the label cache key) rather than threading
/// bytes out of that function: engine binaries are small and this runs once per `label`
/// invocation, not per position, so the extra read is negligible next to actual engine search
/// cost. Same graceful warn-and-None fallback as `compute_engine_fingerprint` for an unreadable
/// (e.g. bare PATH-resolved) path.
fn compute_binary_sha256(engine_path: &Path) -> Option<String> {
    match fs::read(engine_path) {
        Ok(bytes) => Some(sha256_hex(&bytes)),
        Err(e) => {
            tracing::warn!(
                engine = %engine_path.display(),
                "cannot read engine binary for binary_sha256 ({e})"
            );
            None
        }
    }
}

/// Case-insensitive lookup of one USI engine option's value among `--engine-option` pairs,
/// parsed as `u32` -- for `RunManifest.engine_threads`/`engine_hash_mb`. `Threads`/`Hash` aren't
/// dedicated CLI flags in this codebase; they're conventionally passed through the generic
/// `--engine-option Threads=N --engine-option Hash=M` mechanism like any other USI option.
fn engine_option_u32(options: &[(String, String)], key: &str) -> Option<u32> {
    options
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .and_then(|(_, v)| v.parse().ok())
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
    /// Additive since the fixed-node-search round, not a rename of `requested_depth` (which
    /// stays `0` for a node-limited entry, same as before this field existed) -- a rename would
    /// make every pre-existing v2 cache entry fail `CacheEntry`'s parse and get misreported as
    /// corrupted by `cache verify` rather than merely predating this field.
    #[serde(default)]
    search_limit_kind: SearchLimitKind,
    /// The depth (as u64) or node count this entry's search was limited to, mirroring whichever
    /// of `requested_depth`/`search_limit_kind` applies. `0` for pre-existing entries (defaults
    /// to `Depth`/`0`, exactly what they always were).
    #[serde(default)]
    search_limit_value: u64,
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
    limit: SearchLimit,
) -> PathBuf {
    // Why an explicit discriminant byte for `engine_version`, not a sentinel string: `None` must
    // never collide with `Some("")` (or any other literal an engine could plausibly report).
    let (version_tag, version_bytes): (&[u8], &[u8]) = match engine_version {
        Some(v) => (&[1], v.as_bytes()),
        None => (&[0], &[]),
    };
    let engine_options_hash_bytes = cache.engine_options_hash.to_le_bytes();
    // A kind discriminant byte, not just the numeric value -- without it, `--depths 8` and
    // `--nodes 8` would hash to the same cache key despite meaning completely different searches.
    let (limit_kind_tag, limit_value_bytes): (&[u8], [u8; 8]) = match limit {
        SearchLimit::Depth(d) => (&[0], (d as u64).to_le_bytes()),
        SearchLimit::Nodes(n) => (&[1], n.to_le_bytes()),
    };
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
        limit_kind_tag,
        &limit_value_bytes,
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

/// `--usi-strict`/`--restart-engine-every`/`--restart-on-protocol-error`/`--transcript-on-error`,
/// bundled since they're all per-run-constant USI-lifecycle config a worker needs on every call --
/// same bundling rationale as `LabelContext`, which this is embedded into.
#[derive(Clone, Default)]
struct UsiStrictConfig {
    strict: bool,
    restart_on_protocol_error: bool,
    restart_engine_every: Option<u32>,
    transcript_dir: Option<PathBuf>,
}

/// Per-run values every worker needs but that never change across positions -- bundled so
/// `analyze_record`'s signature doesn't grow one parameter per envelope field.
#[derive(Clone)]
struct LabelContext {
    weight_sha256: Option<String>,
    engine_options_hash_hex: String,
    // Bundled here (rather than as separate analyze_record parameters) partly for the same
    // per-run-constant reasoning as the two fields above, and partly to keep analyze_record's
    // own parameter count under clippy's too_many_arguments threshold.
    timeout_ms: u64,
    multipv: u32,
    existing_policy: ExistingPolicy,
    strict_cfg: UsiStrictConfig,
}

/// Per-worker mutable state that survives across jobs on the same thread (unlike `LabelContext`,
/// which is per-run-constant) -- `worker_id` for transcript filenames/logging, and a counter
/// `--restart-engine-every` compares against. Reset to 0 whenever the worker actually relaunches
/// its engine (including the very first launch).
struct WorkerState {
    worker_id: usize,
    searches_since_restart: u32,
}

/// Shared atomics every worker increments as it processes jobs -- bundled so `analyze_record`'s
/// parameter count doesn't grow one atomic per counter.
struct RunCounters {
    timeout_salvaged: AtomicUsize,
    protocol_violations: AtomicUsize,
    engine_restarts: AtomicUsize,
}

/// Short tag identifying which `UsiError` variant fired, used only for transcript filenames and
/// isn't meant to duplicate `UsiError`'s own `Display` (which is a full sentence).
fn error_kind_label(e: &UsiError) -> &'static str {
    match e {
        UsiError::Io(_) => "io",
        UsiError::Timeout => "timeout",
        UsiError::InvalidResponse => "invalid_response",
        UsiError::NoBestmove => "no_bestmove",
        UsiError::BestmoveWithoutGo => "bestmove_without_go",
        UsiError::DuplicateBestmove => "duplicate_bestmove",
        UsiError::DelayedBestmoveAfterTimeout => "delayed_bestmove_after_timeout",
        UsiError::IllegalBestmove { .. } => "illegal_bestmove",
    }
}

/// Dumps `engine`'s captured transcript (raw USI lines since the last clean exchange) to
/// `{nanos}_worker{worker_id}_{kind}.log` under `dir` -- best-effort, since a transcript is a
/// debugging aid, not something a failed write should itself escalate into a run failure.
fn dump_transcript(dir: &Path, engine: &UsiEngine, worker_id: usize, e: &UsiError) {
    // Nanoseconds, not milliseconds: a burst of same-kind violations on one worker (exactly the
    // case a post-mortem most cares about) can otherwise collide on the same filename within a
    // millisecond and silently overwrite each other.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let kind = error_kind_label(e);
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(
        dir.join(format!("{nanos}_worker{worker_id}_{kind}.log")),
        engine.transcript().join("\n"),
    );
}

/// Handles one search error (either a real `UsiError` from `engine.analyse()`, or a synthesized
/// `UsiError::IllegalBestmove` for an otherwise-`Ok` result whose bestmove failed
/// `is_legal_bestmove_lite`) -- logs it, counts it as a protocol violation (excluding `Io`/
/// `Timeout`/`InvalidResponse`/`NoBestmove`, which predate --usi-strict and aren't protocol
/// violations in that sense), dumps a transcript if `--transcript-on-error` is set, and reports
/// whether the caller must discard the current engine and relaunch: unconditional for `Io` (the
/// pipe itself is broken, nothing to reuse), otherwise only under `--restart-on-protocol-error`
/// (a `Timeout` alone can just mean `--timeout-ms` is too tight, not proof of a dead engine).
fn handle_analyse_error(
    e: &UsiError,
    limit: SearchLimit,
    engine: &UsiEngine,
    strict_cfg: &UsiStrictConfig,
    worker_id: usize,
    counters: &RunCounters,
) -> bool {
    tracing::warn!(worker_id, ?limit, "analysis error: {e}");
    let dead = matches!(e, UsiError::Io(_));
    let is_protocol_violation = matches!(
        e,
        UsiError::BestmoveWithoutGo
            | UsiError::DuplicateBestmove
            | UsiError::DelayedBestmoveAfterTimeout
            | UsiError::IllegalBestmove { .. }
    );
    if is_protocol_violation {
        counters.protocol_violations.fetch_add(1, Ordering::Relaxed);
    }
    if let Some(dir) = &strict_cfg.transcript_dir {
        dump_transcript(dir, engine, worker_id, e);
    }
    dead || strict_cfg.restart_on_protocol_error
}

/// Whether `o` (from `engine_name`) already covers a fresh request for `limit`, i.e. `label
/// --skip-existing` can skip re-running it. Depth mode: achieved depth >= requested (unchanged
/// existing behavior). Nodes mode: achieved `nodes` (the engine's own reported total, since
/// nodes -- like depth -- is "how much work was done") >= requested. Requires matching
/// `search_limit_kind` in both directions: a depth-limited observation never "covers" a
/// node-limited request or vice versa, even if the raw numbers happen to compare favorably --
/// they aren't comparable notions of search thoroughness across modes.
fn observation_multipv(o: &Observation) -> u32 {
    o.candidates
        .iter()
        .map(|candidate| candidate.multipv)
        .max()
        .unwrap_or(1)
}

/// Whether an observation was produced with the requested MultiPV setting. Older JSONL has no
/// explicit request field, so the candidate ranks are the durable evidence: no candidates means
/// ordinary single-PV labeling, while the highest reported rank identifies the requested MultiPV
/// setting. Exact matching is intentional -- a MultiPV=2 result should not silently satisfy a
/// later MultiPV=3 request (or cause a re-label to be treated as the same slot as single-PV).
fn observation_matches_multipv(o: &Observation, multipv: u32) -> bool {
    observation_multipv(o) == multipv.max(1)
}

fn observation_covers_limit(
    o: &Observation,
    engine_name: &str,
    limit: SearchLimit,
    multipv: u32,
) -> bool {
    if o.engine != engine_name {
        return false;
    }
    if !observation_matches_multipv(o, multipv) {
        return false;
    }
    match limit {
        SearchLimit::Depth(d) => o.search_limit_kind == SearchLimitKind::Depth && o.depth >= d,
        SearchLimit::Nodes(n) => {
            o.search_limit_kind == SearchLimitKind::Nodes
                && o.nodes.is_some_and(|actual| actual >= n)
        }
    }
}

/// Whether `o` (an existing observation) and `new` (freshly produced) are "the same request" for
/// dedup/replace purposes. Depth mode: unchanged existing logic (achieved-depth equality +
/// requested_depth match-or-legacy-absent, so "requested 12, reached 8" and "requested 8, reached
/// 8" stay distinct). Nodes mode: exact `requested_nodes` match, not achieved-nodes equality -- an
/// engine's actual node count for a fixed budget can jitter run-to-run in a way achieved depth
/// typically doesn't, so matching on achieved nodes would systematically under-dedupe (never
/// matching) rather than over-dedupe.
fn same_search_request(o: &Observation, new: &Observation) -> bool {
    o.engine == new.engine
        && observation_multipv(o) == observation_multipv(new)
        && o.search_limit_kind == new.search_limit_kind
        && match new.search_limit_kind {
            SearchLimitKind::Depth => {
                o.depth == new.depth
                    && (o.requested_depth.is_none() || o.requested_depth == new.requested_depth)
            }
            SearchLimitKind::Nodes => o.requested_nodes == new.requested_nodes,
        }
}

/// Runs every `limit` in `limits` against `rec` (cache-first, then a live `engine.analyse()`
/// call), same as before --usi-strict existed, plus: counting a `--usi-strict` illegal-bestmove
/// finding as a search failure instead of trusting it, and reporting via the `bool` return
/// whether the caller must discard `engine` and relaunch (see `handle_analyse_error`). Once that
/// becomes true, remaining limits for this record are skipped -- there's no reason to keep
/// feeding a soon-to-be-replaced engine process more work.
fn analyze_record(
    rec: &mut PositionRecord,
    engine: &mut UsiEngine,
    limits: &[SearchLimit],
    cache: Option<&LabelCache>,
    counters: &RunCounters,
    ctx: &LabelContext,
    worker: &mut WorkerState,
) -> bool {
    let mut restart_needed = false;
    for &limit in limits {
        // The engine may stop before reaching `limit` (e.g. a forced mate) — check coverage
        // against what was actually achieved, not the requested limit, before spending a call.
        if matches!(ctx.existing_policy, ExistingPolicy::Skip)
            && rec
                .observations
                .iter()
                .any(|o| observation_covers_limit(o, &engine.engine_name, limit, ctx.multipv))
        {
            continue;
        }

        let cache_path = cache.map(|c| {
            label_cache_path(
                c,
                &rec.sfen,
                &engine.engine_name,
                engine.engine_version.as_deref(),
                limit,
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
            None => {
                worker.searches_since_restart += 1;
                match engine.analyse(&rec.sfen, limit, ctx.timeout_ms) {
                    // `bestmove_kind.is_none()` excludes resign/win/none -- those are special
                    // USI tokens, not board moves, and would otherwise always false-positive as
                    // illegal (they never match any entry in `all_legal_moves_partial`).
                    Ok(result)
                        if ctx.strict_cfg.strict
                            && result.bestmove_kind.is_none()
                            && !is_legal_bestmove_lite(&rec.sfen, &result.bestmove)
                                .unwrap_or(true) =>
                    {
                        let err = UsiError::IllegalBestmove {
                            sfen: rec.sfen.clone(),
                            mv: result.bestmove.clone(),
                        };
                        if handle_analyse_error(
                            &err,
                            limit,
                            engine,
                            &ctx.strict_cfg,
                            worker.worker_id,
                            counters,
                        ) {
                            restart_needed = true;
                        }
                        None
                    }
                    Ok(result) => {
                        if result.timed_out {
                            counters.timeout_salvaged.fetch_add(1, Ordering::Relaxed);
                        }
                        let obs = Observation {
                            engine: engine.engine_name.clone(),
                            engine_version: engine.engine_version.clone(),
                            depth: result.depth,
                            requested_depth: match limit {
                                SearchLimit::Depth(d) => Some(d),
                                SearchLimit::Nodes(_) => None,
                            },
                            requested_nodes: match limit {
                                SearchLimit::Depth(_) => None,
                                SearchLimit::Nodes(n) => Some(n),
                            },
                            search_limit_kind: limit.kind(),
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
                            seldepth: result.seldepth,
                            nps: result.nps,
                            hashfull: result.hashfull,
                            pv: result.pv,
                            policy_margin_cp: result.policy_margin_cp,
                            candidates: result.candidates,
                            engine_options_hash: Some(ctx.engine_options_hash_hex.clone()),
                            weight_sha256: ctx.weight_sha256.clone(),
                            was_timeout_salvaged: result.timed_out,
                        };
                        if let Some(path) = &cache_path {
                            if let Some(parent) = path.parent() {
                                let _ = fs::create_dir_all(parent);
                            }
                            // Cache writes always produce v2 (CacheEntry) -- only the read path needs
                            // to understand v1, for cache dirs populated before this round.
                            if let Some(c) = cache {
                                let (cache_requested_depth, search_limit_value) = match limit {
                                    SearchLimit::Depth(d) => (d, d as u64),
                                    SearchLimit::Nodes(n) => (0, n),
                                };
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
                                    requested_depth: cache_requested_depth,
                                    search_limit_kind: limit.kind(),
                                    search_limit_value,
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
                        if handle_analyse_error(
                            &e,
                            limit,
                            engine,
                            &ctx.strict_cfg,
                            worker.worker_id,
                            counters,
                        ) {
                            restart_needed = true;
                        }
                        None
                    }
                }
            }
        };

        if let Some(obs) = observation {
            if !matches!(ctx.existing_policy, ExistingPolicy::Append) {
                rec.observations.retain(|o| !same_search_request(o, &obs));
            }
            rec.observations.push(obs);
        }

        if restart_needed {
            break;
        }
    }

    if let Some(every) = ctx.strict_cfg.restart_engine_every
        && worker.searches_since_restart >= every
    {
        restart_needed = true;
    }

    restart_needed
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

/// Parse a comma-separated list of positive, distinct numeric limits without silently dropping
/// malformed entries. Search budgets are part of label identity, so accepting `4,bad,8` as
/// `4,8` or running the same budget twice would make the manifest describe a command other than
/// the one that was actually executed.
fn parse_positive_csv<T>(value: &str, flag: &str) -> Result<Vec<T>>
where
    T: Copy + Default + Display + FromStr + Ord,
    T::Err: Display,
{
    let mut parsed = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in value.split(',') {
        let item = raw.trim();
        if item.is_empty() {
            anyhow::bail!("{flag} contains an empty value: {value:?}");
        }
        let number = item
            .parse::<T>()
            .map_err(|error| anyhow::anyhow!("invalid value {item:?} in {flag}: {error}"))?;
        if number <= T::default() {
            anyhow::bail!("{flag} values must be greater than zero, got {number}");
        }
        if !seen.insert(number) {
            anyhow::bail!("{flag} values must be distinct, duplicate {number}");
        }
        parsed.push(number);
    }
    Ok(parsed)
}

/// Parse `--engine-option NAME=VALUE` once, preserving the value verbatim while rejecting input
/// that used to disappear through `filter_map`. MultiPV has a dedicated flag because it also
/// changes observation/cache identity; accepting both controls would let those identities drift.
fn parse_engine_options(values: &[String]) -> Result<Vec<(String, String)>> {
    let mut parsed = Vec::with_capacity(values.len());
    let mut names = BTreeSet::new();
    for raw in values {
        let (name, value) = raw.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("invalid --engine-option {raw:?}; expected NAME=VALUE")
        })?;
        if name.trim().is_empty() {
            anyhow::bail!("invalid --engine-option {raw:?}; NAME must not be empty");
        }
        if name.eq_ignore_ascii_case("MultiPV") {
            anyhow::bail!(
                "MultiPV must be configured with --multipv, not --engine-option, so label identity remains consistent"
            );
        }
        let normalized = name.to_ascii_lowercase();
        if !names.insert(normalized) {
            anyhow::bail!("duplicate --engine-option name {name:?}");
        }
        parsed.push((name.to_string(), value.to_string()));
    }
    Ok(parsed)
}

fn cmd_label(args: LabelArgs) -> Result<()> {
    // Bounded-pipeline changes (worker count, cache usage, ordering) can't be judged without
    // measuring their actual effect on throughput -- this run's wall-clock start, used for
    // `records_per_sec` below.
    let run_start = std::time::Instant::now();
    // clap's required_unless_present/conflicts_with on --depths/--nodes guarantees exactly one
    // of these is Some.
    let limits: Vec<SearchLimit> = if let Some(depths) = &args.depths {
        let depths: Vec<u32> = parse_positive_csv(depths, "--depths")?;
        depths.into_iter().map(SearchLimit::Depth).collect()
    } else {
        let nodes: Vec<u64> = parse_positive_csv(
            args.nodes
                .as_ref()
                .expect("clap guarantees --depths or --nodes"),
            "--nodes",
        )?;
        nodes.into_iter().map(SearchLimit::Nodes).collect()
    };
    if args.timeout_ms == 0 {
        anyhow::bail!("--timeout-ms must be greater than zero");
    }
    if args.jobs == 0 {
        anyhow::bail!("--jobs must be greater than zero");
    }
    if args.multipv == 0 {
        anyhow::bail!("--multipv must be greater than zero");
    }
    let weight_sha256 = args
        .weight_file
        .as_ref()
        .map(|path| compute_weight_sha256(path))
        .transpose()?;
    // See `build_resume_index` for why this is an offset index, not the full-record map
    // `load_records` would give -- a missing path just means "nothing to resume yet" (lets a
    // wrapper script pass --resume-from unconditionally from the very first run), same as before.
    let mut resume_source: Option<(commands::ResumeIndex, File)> = match &args.resume_from {
        Some(path) if path.exists() => {
            let index = commands::build_resume_index(path)?;
            let file = File::open(path).with_context(|| format!("cannot open {path:?}"))?;
            Some((index, file))
        }
        _ => None,
    };

    let engine_path = args.engine.clone();
    let engine_name = args.engine_name.clone().unwrap_or_default();
    let timeout_ms = args.timeout_ms;
    let jobs = args.jobs;
    let mut engine_options = parse_engine_options(&args.engine_options)?;
    if args.multipv > 1 {
        engine_options.push(("MultiPV".to_string(), args.multipv.to_string()));
    }
    // --resume-from implies Skip unless --replace-existing is explicit: without this, forgetting
    // --skip-existing alongside --resume-from would merge in the old observations and *still*
    // redundantly re-run the engine for already-covered depths under the default Append policy --
    // no data loss, but it defeats the entire point of resuming.
    let existing_policy = if args.replace_existing {
        ExistingPolicy::Replace
    } else if args.skip_existing || args.resume_from.is_some() {
        ExistingPolicy::Skip
    } else {
        ExistingPolicy::Append
    };
    let strict_cfg = UsiStrictConfig {
        strict: args.usi_strict,
        restart_on_protocol_error: args.restart_on_protocol_error,
        restart_engine_every: args.restart_engine_every,
        transcript_dir: args.transcript_on_error.clone(),
    };
    let label_ctx = LabelContext {
        weight_sha256: weight_sha256.clone(),
        engine_options_hash_hex: engine_options_hash_hex(&engine_options),
        timeout_ms,
        multipv: args.multipv.max(1),
        existing_policy,
        strict_cfg,
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
    let counters = Arc::new(RunCounters {
        timeout_salvaged: AtomicUsize::new(0),
        protocol_violations: AtomicUsize::new(0),
        engine_restarts: AtomicUsize::new(0),
    });
    let done = Arc::new(AtomicUsize::new(0));

    // Reader: streams the input line-by-line so the whole dataset is never resident in memory --
    // only ever `queue_depth` records at a time, enforced by the permit acquire below. It also
    // accumulates the manifest's `input_hash` over the same lines it's already reading -- calling
    // `hash_file` afterward, like every other manifest-producing command does, would mean
    // re-opening and re-reading the whole input a second time purely to hash it, defeating the
    // point of streaming in the first place.
    let input_path = args.input.clone();
    let reader_handle = std::thread::spawn(move || -> Result<(u64, usize, u64, String, String)> {
        let mut reader = open_hashed_position_jsonl(&input_path)?;
        let mut job_id = 0u64;
        let mut skipped = 0usize;
        let mut resumed_count = 0u64;
        for item in &mut reader {
            match item {
                Ok(PositionRecordLine {
                    line_number: _,
                    mut record,
                }) if Sfen::parse(&record.sfen).is_ok() => {
                    if let Some((index, file)) = resume_source.as_mut()
                        && let Some(&offset) = index.get(&commands::merge_alignment_key(&record))
                    {
                        // Matches `load_records`' tolerance (the full-load path this replaced):
                        // an unreadable/malformed resume line is warned and skipped, not fatal --
                        // this position is simply relabeled from scratch, same as if it had never
                        // been in the resume file at all.
                        match commands::read_resume_observations_at(file, offset) {
                            Ok(resume_obs) => {
                                commands::merge_resume_observations(
                                    &mut record.observations,
                                    resume_obs,
                                );
                                resumed_count += 1;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "resume-from record unreadable at offset {offset}: {e}"
                                );
                            }
                        }
                    }
                    // Blocks here once `queue_depth` jobs are dispatched-but-unwritten.
                    if permit_rx.recv().is_err() || job_tx.send(Job { id: job_id, record }).is_err()
                    {
                        break; // writer/workers gone -- don't hang trying to feed a dead pipeline
                    }
                    job_id += 1;
                }
                Ok(PositionRecordLine { line_number, .. }) => {
                    tracing::warn!(line = line_number, "invalid SFEN, skipping");
                    skipped += 1;
                }
                Err(PositionRecordLineError::Parse {
                    line_number,
                    source,
                }) => {
                    tracing::warn!(line = line_number, "JSON parse error: {source}");
                    skipped += 1;
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("cannot read {input_path:?}"));
                }
            }
        }
        let (input_hash, dataset_sha256) = finish_hashed_position_jsonl(reader);
        Ok((job_id, skipped, resumed_count, input_hash, dataset_sha256))
    });

    // Workers: each owns one long-lived USI engine for its whole lifetime, launched lazily on
    // its first job and reused across every job it picks up after that -- spawning a fresh
    // engine per position would hide true search cost behind repeated process-startup overhead.
    let worker_handles: Vec<_> = (0..jobs)
        .map(|worker_id| {
            let job_rx = Arc::clone(&job_rx);
            let result_tx = result_tx.clone();
            let engine_path = engine_path.clone();
            let engine_name = engine_name.clone();
            let engine_options = engine_options.clone();
            let engine_launch_failures = Arc::clone(&engine_launch_failures);
            let counters = Arc::clone(&counters);
            let done = Arc::clone(&done);
            let limits = limits.clone();
            let label_ctx = label_ctx.clone();
            let cache = cache.clone();
            std::thread::spawn(move || {
                let mut engine: Option<UsiEngine> = None;
                let mut worker = WorkerState {
                    worker_id,
                    searches_since_restart: 0,
                };
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
                        engine = Some(
                            e.with_strict(label_ctx.strict_cfg.strict)
                                .with_transcript_capture(
                                    label_ctx.strict_cfg.transcript_dir.is_some(),
                                ),
                        );
                        worker.searches_since_restart = 0;
                    }
                    if let Some(eng) = engine.as_mut() {
                        let restart_needed = analyze_record(
                            &mut record,
                            eng,
                            &limits,
                            cache.as_deref(),
                            &counters,
                            &label_ctx,
                            &mut worker,
                        );
                        if restart_needed {
                            engine = None;
                            counters.engine_restarts.fetch_add(1, Ordering::Relaxed);
                        }
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
    let mut out_writer = Sha256Writer::new(BufWriter::new(out_file));
    let mut manifest = args
        .manifest
        .as_ref()
        .map(|_| new_run_manifest("label", &args.input));
    let mut next_id = 0u64;
    let mut pending: BTreeMap<u64, PositionRecord> = BTreeMap::new();
    let mut written = 0usize;
    // Accumulated here (not a second pass) since write_one already sees every written record's
    // observations once, right before writing.
    let mut time_ms_sum = 0u64;
    let mut time_ms_count = 0usize;

    {
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
                // Guards the permit scheme's whole reason for existing: without it, `pending`
                // could in principle grow past `queue_depth` and this bounded pipeline would
                // silently revert to the unbounded memory use it replaced.
                debug_assert!(pending.len() <= queue_depth);
            } else {
                write_one(&job.record, &mut manifest)?;
                written += 1;
            }
        }
    }
    out_writer.flush()?;
    let output_sha256 = out_writer.digest_hex();
    eprintln!();

    let (total, skipped, resumed_count, input_hash, dataset_sha256) = reader_handle
        .join()
        .map_err(|_| anyhow::anyhow!("label reader thread panicked"))??;
    for handle in worker_handles {
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("label worker thread panicked"))?;
    }
    let engine_launch_failures = engine_launch_failures.load(Ordering::Relaxed);
    let timeout_salvaged_count = counters.timeout_salvaged.load(Ordering::Relaxed);
    let protocol_violations_count = counters.protocol_violations.load(Ordering::Relaxed);
    let engine_restarts_count = counters.engine_restarts.load(Ordering::Relaxed);
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
    let accepted = usize::try_from(total).context("label record count exceeds this platform")?;
    let records_read = accepted
        .checked_add(skipped)
        .context("label input record count overflow")?;
    eprintln!(
        "done [{engine_display_name}, jobs={jobs}]: {records_read} in, {written} labeled, {skipped} skipped, {engine_launch_failures} engine launch failures{cache_suffix}{throughput_suffix} → {:?}",
        args.out
    );
    if let Some(mut manifest) = manifest {
        manifest.input_hash = input_hash;
        manifest.records_read = records_read;
        manifest.records_kept = written;
        manifest.records_dropped = skipped;
        manifest.engine_name = Some(engine_display_name);
        let depth_values: Vec<u32> = limits
            .iter()
            .filter_map(|l| match l {
                SearchLimit::Depth(d) => Some(*d),
                SearchLimit::Nodes(_) => None,
            })
            .collect();
        let node_values: Vec<u64> = limits
            .iter()
            .filter_map(|l| match l {
                SearchLimit::Nodes(n) => Some(*n),
                SearchLimit::Depth(_) => None,
            })
            .collect();
        manifest.depths = (!depth_values.is_empty()).then_some(depth_values);
        manifest.nodes = (!node_values.is_empty()).then_some(node_values);
        manifest.multipv = (args.multipv > 1).then_some(args.multipv);
        manifest.engine_options = Some(args.engine_options.clone());
        manifest.jobs = Some(jobs);
        manifest.engine_launch_failures = Some(engine_launch_failures);
        manifest.timeout_salvaged_count = Some(timeout_salvaged_count);
        manifest.protocol_violations_count = args.usi_strict.then_some(protocol_violations_count);
        manifest.engine_restarts = Some(engine_restarts_count);
        manifest.records_per_sec = records_per_sec;
        manifest.average_engine_time_ms = average_engine_time_ms;
        manifest.preserve_order = Some(args.preserve_order);
        manifest.resume_from = args.resume_from.clone();
        manifest.resumed_count = args.resume_from.is_some().then_some(resumed_count);
        if let Some((hits, misses)) = cache_counts {
            manifest.cache_hits = Some(hits);
            manifest.cache_hit_rate = cache_hit_rate;
            manifest.cache_misses = Some(misses);
            manifest.engine_fingerprint_mode = Some(engine_fingerprint_mode.as_str().to_string());
        }
        // shogiesa-owned provenance fields: dataset_sha256/binary_sha256/weight_sha256/
        // engine_threads/engine_hash_mb are computed here; everything else is opaque
        // orchestrator-supplied passthrough that shogiesa doesn't interpret.
        manifest.dataset_sha256 = Some(dataset_sha256);
        manifest.output_sha256 = Some(output_sha256);
        manifest.binary_sha256 = compute_binary_sha256(&engine_path);
        manifest.weight_sha256 = weight_sha256;
        manifest.engine_threads = engine_option_u32(&engine_options, "Threads");
        manifest.engine_hash_mb = engine_option_u32(&engine_options, "Hash");
        manifest.experiment_id.clone_from(&args.experiment_id);
        manifest.candidate_id.clone_from(&args.candidate_id);
        manifest.baseline_id.clone_from(&args.baseline_id);
        manifest.lineage_id.clone_from(&args.lineage_id);
        manifest
            .teacher_manifest_sha256
            .clone_from(&args.teacher_manifest_sha256);
        manifest.init_seed = args.init_seed;
        manifest.split_seed = args.split_seed;
        manifest.split_sha256.clone_from(&args.split_sha256);
        manifest.validity.clone_from(&args.validity);
        write_manifest(args.manifest.as_ref().unwrap(), &manifest)?;
    }
    Ok(())
}

fn cmd_stability(args: StabilityArgs) -> Result<()> {
    let mut writer = AtomicWriter::new(&args.out)?;

    let mut total = 0usize;
    let mut enriched = 0usize;
    let mut skipped = 0usize;

    for item in open_position_jsonl(&args.input)? {
        total += 1;
        let Some(mut rec) = permissive_position_record(item, &args.input, &mut skipped)? else {
            continue;
        };
        rec.fill_stability();
        if rec.stability.is_some() {
            enriched += 1;
        }
        serde_json::to_writer(&mut writer, &rec)?;
        writer.write_all(b"\n")?;
    }

    writer.commit()?;
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

struct SplitBundleEntry {
    staged: PathBuf,
    destination: PathBuf,
    prepared_temp: Option<tempfile::TempPath>,
    backup: Option<tempfile::TempPath>,
    committed: bool,
}

fn rollback_split_bundle(entries: &mut [SplitBundleEntry]) -> Vec<String> {
    let mut errors = Vec::new();
    for entry in entries.iter_mut().rev() {
        if entry.committed {
            if let Err(error) = fs::remove_file(&entry.destination) {
                let mut message = format!(
                    "cannot remove partially committed {:?}: {error}",
                    entry.destination
                );
                if let Some(backup) = entry.backup.take() {
                    let backup_path = backup.to_path_buf();
                    match backup.keep() {
                        Ok(path) => {
                            message.push_str(&format!("; previous output retained at {path:?}"));
                        }
                        Err(keep_error) => {
                            message.push_str(&format!(
                                "; could not retain previous output at {backup_path:?}: {keep_error}"
                            ));
                        }
                    }
                }
                errors.push(message);
                continue;
            }
            entry.committed = false;
        }
        if let Some(backup) = entry.backup.take() {
            let backup_path = backup.to_path_buf();
            if let Err(error) = fs::rename(&backup_path, &entry.destination) {
                let retention = match backup.keep() {
                    Ok(path) => format!("previous output retained at {path:?}"),
                    Err(keep_error) => {
                        format!("could not retain previous output at {backup_path:?}: {keep_error}")
                    }
                };
                errors.push(format!(
                    "cannot restore backup for {:?}: {error}; {retention}",
                    entry.destination,
                ));
            }
        }
    }
    errors
}

/// Commit all per-source files and their manifest as one recoverable bundle. Each destination is
/// backed up before replacement; any later failure restores every earlier destination in reverse
/// order. Existing symlinks are resolved so their targets are updated without replacing the link.
fn commit_split_bundle(staged_files: &[(PathBuf, PathBuf)]) -> Result<()> {
    commit_split_bundle_with(staged_files, |_| Ok(()))
}

fn commit_split_bundle_with<F>(
    staged_files: &[(PathBuf, PathBuf)],
    mut before_commit: F,
) -> Result<()>
where
    F: FnMut(usize) -> Result<()>,
{
    let mut entries = Vec::with_capacity(staged_files.len());
    let mut destinations = HashSet::with_capacity(staged_files.len());

    // Complete every fallible preparation step before moving an existing destination. A symlink
    // target may live on another filesystem, so copy that staged file to a temporary neighbor;
    // the final replacement can then use a same-filesystem rename.
    for (staged, requested_destination) in staged_files {
        let destination = match fs::symlink_metadata(requested_destination) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                fs::canonicalize(requested_destination).with_context(|| {
                    format!("cannot resolve split output symlink {requested_destination:?}")
                })?
            }
            Ok(metadata) if metadata.is_dir() => {
                anyhow::bail!("split output destination is a directory: {requested_destination:?}")
            }
            Ok(_) => requested_destination.clone(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                requested_destination.clone()
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("cannot inspect split output {requested_destination:?}")
                });
            }
        };
        if !destinations.insert(destination.clone()) {
            anyhow::bail!("split outputs resolve to the same destination: {destination:?}");
        }

        let mut prepared_temp = None;
        let prepared_staged = if destination != *requested_destination {
            let parent = destination.parent().unwrap_or(Path::new("."));
            let temp = tempfile::NamedTempFile::new_in(parent).with_context(|| {
                format!("cannot stage split output beside symlink target {destination:?}")
            })?;
            fs::copy(staged, temp.path()).with_context(|| {
                format!("cannot stage split output for symlink target {destination:?}")
            })?;
            temp.as_file()
                .sync_all()
                .with_context(|| format!("cannot sync staged split output for {destination:?}"))?;
            let temp = temp.into_temp_path();
            let path = temp.to_path_buf();
            prepared_temp = Some(temp);
            path
        } else {
            // Windows requires a writable handle for `sync_all`/`FlushFileBuffers` even when
            // the file contents are already complete. Opening read-only made bundle commits
            // fail during preparation before rollback behavior could run.
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(staged)
                .with_context(|| format!("cannot open staged split output {staged:?}"))?
                .sync_all()
                .with_context(|| format!("cannot sync staged split output {staged:?}"))?;
            staged.clone()
        };

        entries.push(SplitBundleEntry {
            staged: prepared_staged,
            destination,
            prepared_temp,
            backup: None,
            committed: false,
        });
    }

    for index in 0..entries.len() {
        if let Err(error) = before_commit(index) {
            let rollback_errors = rollback_split_bundle(&mut entries);
            let suffix = if rollback_errors.is_empty() {
                String::new()
            } else {
                format!("; rollback incomplete: {}", rollback_errors.join("; "))
            };
            anyhow::bail!("split bundle commit interrupted: {error}{suffix}");
        }
        let destination = entries[index].destination.clone();
        if destination.exists() {
            let parent = destination.parent().unwrap_or(Path::new("."));
            let backup = match tempfile::NamedTempFile::new_in(parent) {
                Ok(file) => file.into_temp_path(),
                Err(error) => {
                    let rollback_errors = rollback_split_bundle(&mut entries);
                    let suffix = if rollback_errors.is_empty() {
                        String::new()
                    } else {
                        format!("; rollback incomplete: {}", rollback_errors.join("; "))
                    };
                    anyhow::bail!(
                        "cannot create backup beside split output {destination:?}: {error}{suffix}"
                    );
                }
            };
            let backup_path = backup.to_path_buf();
            if let Err(error) =
                fs::remove_file(&backup_path).and_then(|()| fs::rename(&destination, &backup_path))
            {
                let rollback_errors = rollback_split_bundle(&mut entries);
                let suffix = if rollback_errors.is_empty() {
                    String::new()
                } else {
                    format!("; rollback incomplete: {}", rollback_errors.join("; "))
                };
                anyhow::bail!("cannot back up split output {destination:?}: {error}{suffix}");
            }
            entries[index].backup = Some(backup);
        }

        if let Err(error) = fs::rename(&entries[index].staged, &destination) {
            let rollback_errors = rollback_split_bundle(&mut entries);
            let suffix = if rollback_errors.is_empty() {
                String::new()
            } else {
                format!("; rollback incomplete: {}", rollback_errors.join("; "))
            };
            anyhow::bail!("cannot commit split output {destination:?}: {error}{suffix}");
        }
        entries[index].committed = true;
    }

    for entry in &mut entries {
        if let Some(backup) = entry.backup.take()
            && let Err(error) = backup.close()
        {
            tracing::warn!(
                destination = ?entry.destination,
                "cannot remove old split output backup: {error}"
            );
        }
        // Keep the guard alive through the rename; dropping it now only tries to remove the old,
        // no-longer-existing temporary path.
        entry.prepared_temp.take();
    }
    Ok(())
}

/// Keep the long-standing readable file name when it is unique, but disambiguate two source
/// paths that sanitize to the same name (`a/b` and `a_b`, for example). The suffix is based on
/// the original source identity, so it is deterministic and does not depend on process state.
fn allocate_split_file_name(source: &str, owners: &mut HashMap<String, String>) -> Result<String> {
    let safe: String = source
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let plain = format!("{safe}.jsonl");
    let file_name = match owners.get(&plain) {
        None => plain,
        Some(owner) if owner == source => plain,
        Some(_) => {
            let digest = blake3::hash(source.as_bytes()).to_hex();
            format!("{safe}--{}.jsonl", &digest[..16])
        }
    };
    if let Some(owner) = owners.get(&file_name)
        && owner != source
    {
        anyhow::bail!(
            "source paths {owner:?} and {source:?} map to the same split output {file_name:?}"
        );
    }
    owners.insert(file_name.clone(), source.to_string());
    Ok(file_name)
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
    let manifest_path = out_dir.join("manifest.json");
    let staging = tempfile::Builder::new()
        .prefix(".shogiesa-split-")
        .tempdir_in(out_dir)
        .with_context(|| format!("cannot create split staging directory in {out_dir:?}"))?;
    let staging_dir = staging.path();

    // Group records by source path, writing into per-source output files via a bounded LRU pool
    let mut pool = WriterPool::new(args.max_open_writers.max(1));
    let mut file_names: HashMap<String, String> = HashMap::new();
    let mut file_name_owners: HashMap<String, String> = HashMap::new();
    let mut file_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    let mut broken = 0usize;

    for item in open_position_jsonl(&args.input)? {
        let Some(rec) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
        };

        let key = rec.source.path.clone();
        let file_name = match file_names.get(&key) {
            Some(file_name) => file_name.clone(),
            None => {
                let file_name = allocate_split_file_name(&key, &mut file_name_owners)?;
                file_names.insert(key.clone(), file_name.clone());
                file_name
            }
        };
        let out_path = out_dir.join(&file_name);
        validate_path_roles(
            &[("--input", &args.input)],
            &[
                ("split output", &out_path),
                ("split manifest", &manifest_path),
            ],
        )?;
        pool.write_line(&staging_dir.join(&file_name), &key, &rec)?;
        *file_counts.entry(file_name).or_default() += 1;
        total += 1;
    }
    pool.flush_all()?;
    drop(pool);

    let output_hashes: BTreeMap<String, String> = file_counts
        .keys()
        .map(|file_name| Ok((file_name.clone(), hash_file(&staging_dir.join(file_name))?)))
        .collect::<Result<_>>()?;

    let manifest = serde_json::json!({
        "shogiesa_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION,
        "fingerprint_algorithm": "blake3",
        "input": args.input.display().to_string(),
        "input_hash": hash_file(&args.input)?,
        "by_source": args.by_source,
        "total_positions": total,
        "skipped_records": broken,
        "files": file_counts,
        "output_hashes": output_hashes,
    });
    let staged_manifest = staging_dir.join("manifest.json");
    atomic_write_bytes(
        &staged_manifest,
        serde_json::to_string_pretty(&manifest)?.as_bytes(),
    )
    .with_context(|| format!("cannot write staged split manifest {staged_manifest:?}"))?;

    let mut staged_files: Vec<(PathBuf, PathBuf)> = file_counts
        .keys()
        .map(|file_name| (staging_dir.join(file_name), out_dir.join(file_name)))
        .collect();
    // The manifest is committed last, so readers never see it claim a new bundle before every
    // referenced file is in place. A failure still rolls the earlier file replacements back.
    staged_files.push((staged_manifest, manifest_path));
    commit_split_bundle(&staged_files)?;

    eprintln!(
        "done: {total} positions split, {broken} skipped, into {} files → {:?}",
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

    let mut train_writer = AtomicWriter::new(train_path)?;
    let mut valid_writer = AtomicWriter::new(valid_path)?;
    let mut test_writer = AtomicWriter::new(test_path)?;

    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut sources: HashMap<&'static str, HashSet<String>> = HashMap::new();
    let mut roots: HashMap<&'static str, HashSet<String>> = HashMap::new();
    let mut total = 0usize;
    let mut broken = 0usize;

    for item in open_position_jsonl(&args.input)? {
        let Some(rec) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
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
        let root_key = split_root_key(&rec.source).to_string();
        sources.entry(name).or_default().insert(rec.source.path);
        roots.entry(name).or_default().insert(root_key);
        total += 1;
    }

    train_writer.commit()?;
    valid_writer.commit()?;
    test_writer.commit()?;

    let split_manifest = |name: &str, path: &PathBuf, output_hash: &str| {
        let mut source_root_ids: Vec<String> = roots
            .get(name)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default();
        source_root_ids.sort();
        serde_json::json!({
            "path": path,
            "output_hash": output_hash,
            "positions": counts.get(name).copied().unwrap_or(0),
            "sources": sources.get(name).map(HashSet::len).unwrap_or(0),
            "source_roots": roots.get(name).map(HashSet::len).unwrap_or(0),
            "source_root_ids": source_root_ids,
        })
    };
    let train_hash = hash_file(train_path)?;
    let valid_hash = hash_file(valid_path)?;
    let test_hash = hash_file(test_path)?;
    let manifest = serde_json::json!({
        "shogiesa_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION,
        "fingerprint_algorithm": "blake3",
        "input": args.input.display().to_string(),
        "input_hash": hash_file(&args.input)?,
        "seed": args.seed,
        "requested": { "valid_frac": args.valid_frac, "test_frac": args.test_frac },
        "total_positions": total,
        "skipped_records": broken,
        "splits": {
            "train": split_manifest("train", train_path, &train_hash),
            "valid": split_manifest("valid", valid_path, &valid_hash),
            "test": split_manifest("test", test_path, &test_hash),
        },
    });
    let manifest_path = train_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("manifest.json");
    atomic_write_bytes(
        &manifest_path,
        serde_json::to_string_pretty(&manifest)?.as_bytes(),
    )
    .with_context(|| format!("cannot write {manifest_path:?}"))?;

    eprintln!(
        "done: {total} positions split, {broken} skipped (seed={}) → train={}, valid={}, test={}",
        args.seed,
        counts.get("train").copied().unwrap_or(0),
        counts.get("valid").copied().unwrap_or(0),
        counts.get("test").copied().unwrap_or(0),
    );
    Ok(())
}

/// Construct the public, versioned core manifest with this binary's build metadata.
fn new_run_manifest(command: &str, input_path: &Path) -> RunManifest {
    RunManifest::new(
        command,
        input_path,
        pack::FORMAT_VERSION,
        env!("CARGO_PKG_VERSION"),
        env!("SHOGIESA_GIT_SHA"),
        std::env::args().collect(),
    )
}

fn record_manifest_provenance(
    record: &PositionRecord,
    source_roots: &mut BTreeMap<String, usize>,
    engines: &mut BTreeMap<String, usize>,
    weights: &mut BTreeMap<String, usize>,
) {
    *source_roots
        .entry(split_root_key(&record.source).to_string())
        .or_default() += 1;
    for observation in &record.observations {
        *engines.entry(observation.engine.clone()).or_default() += 1;
        *weights
            .entry(
                observation
                    .weight_sha256
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            )
            .or_default() += 1;
    }
}

fn write_manifest(path: &Path, manifest: &RunManifest) -> Result<()> {
    atomic_write_bytes(path, serde_json::to_string_pretty(manifest)?.as_bytes())
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

/// Folds one file's content into a shared outer hasher, for `lineprior export --manifest`'s
/// whole-directory `input_hash`. Each fixed-width per-file digest is its own boundary marker, so
/// two different splits of the same total bytes across files never collide -- unlike feeding raw
/// file content into one hasher directly. Called only for files the export loop already read
/// successfully (not a strict pass over every discovered path), so a directory containing one
/// unreadable/non-UTF-8 file doesn't turn `--manifest` into a new failure mode for a run that
/// would otherwise succeed with a skip.
fn fold_file_hash(hasher: &mut blake3::Hasher, content: &str) {
    hasher.update(blake3::hash(content.as_bytes()).as_bytes());
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
        *manifest
            .score_bound_distribution
            .entry(key.to_string())
            .or_default() += count;
    }
    let (requested_total, underreach) = requested_depth_stats(records);
    manifest.requested_depth_total += requested_total;
    manifest.requested_depth_underreach += underreach;
}

// seeded_hash, HeapEntry, push_bounded now live in stratifykit-core (imported above) --
// this crate no longer hand-rolls bounded top-K sampling itself.

fn cmd_sample(args: SampleArgs) -> Result<()> {
    let seed = args.seed;
    let count = args.count;

    let reader = open_position_jsonl(&args.input)?;
    let mut broken = 0usize;
    let mut read_error: Option<anyhow::Error> = None;
    let records = reader.filter_map(|item| {
        match permissive_position_record(item, &args.input, &mut broken) {
            Ok(record) => record,
            Err(error) => {
                read_error.get_or_insert(error);
                None
            }
        }
    });
    // Reservoir sample restores original file order (unlike `select`, which outputs in ranked
    // order).
    let (kept, total) = reservoir_sample(records, count, seed, |r| r.sfen.as_str());
    if let Some(error) = read_error {
        return Err(error);
    }

    let mut writer = AtomicWriter::new(&args.out)?;
    for record in &kept {
        serde_json::to_writer(&mut writer, record)?;
        writer.write_all(b"\n")?;
    }
    writer.commit()?;
    eprintln!(
        "done: {}/{total} sampled (seed={seed}) → {:?}",
        kept.len(),
        args.out
    );
    if let Some(manifest_path) = &args.manifest {
        let mut manifest = new_run_manifest("sample", &args.input);
        manifest.input_hash = hash_file(&args.input)?;
        manifest.records_read = total;
        manifest.records_kept = kept.len();
        manifest.records_dropped = total - kept.len();
        accumulate_coverage(&mut manifest, &kept);
        write_manifest(manifest_path, &manifest)?;
    }
    Ok(())
}

/// One line of `shuffle --order-manifest`: `training_position` -> full provenance for that
/// output position, so "what were the 32 positions in block B5" is answerable later without
/// re-running the shuffle. Every key is always present (unlike `RunManifest`'s sparse optional
/// fields), since this file is meant to be loaded as a table, not skimmed as a summary.
#[derive(serde::Serialize)]
struct OrderManifestLine {
    training_position: u64,
    sample_id: String,
    game_id: String,
    game_position_index: u32,
    outcome: Option<GameOutcome>,
    /// Raw side-to-move-relative cp as stored on the selected teacher observation -- NOT
    /// black-perspective normalized (unlike `report`/`audit`'s analysis views). This is
    /// provenance passthrough, not an analysis field.
    teacher_cp: Option<i32>,
    teacher_depth: Option<u32>,
    source_file: String,
    split_id: Option<String>,
    shuffle_seed: u64,
    block_id: u64,
    /// Always `None` this round -- the gate-opening SFEN a game started from isn't retained
    /// anywhere on `PositionRecord` today (used transiently during extraction, then discarded).
    /// Populate once extraction captures it; not this round's scope.
    opening_id: Option<String>,
}

/// Deterministic per-record identity: a hash of the one existing "which real-world position is
/// this" key (`merge_alignment_key`'s `(sfen, source.path, source.ply)` triple, already used by
/// `merge-observations`'s collision detection for the same purpose).
///
/// ponytail: `source.path` isn't stable across independent re-extractions on a different
/// machine/cwd layout (no extractor canonicalizes it), so this is a within-run join key, not a
/// formula other tools should independently recompute -- the `--order-manifest` file itself is
/// the durable, portable artifact to keep alongside the shuffled data. Upgrade path if
/// independent recomputation by a tool that never sees the manifest becomes a real need: swap
/// `source.path` for a content-hash of the source game file.
fn sample_id(rec: &PositionRecord) -> String {
    hash_parts(&[
        rec.sfen.as_bytes(),
        rec.source.path.as_bytes(),
        &rec.source.ply.to_le_bytes(),
    ])
    .to_hex()
    .to_string()
}

/// Finds the observation (already filtered to one engine) matching `target_depth` -- primary
/// match on `requested_depth` (what `label` was actually asked to reach), falling back to the
/// achieved `depth` for legacy pre-schema-v6 data.
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

/// The observation matching `engine`+`depth`, for `shuffle --order-manifest`'s teacher_cp/
/// teacher_depth columns. Reuses `find_at_depth` (requested_depth-first, depth-fallback) after
/// narrowing to one engine -- the same selection logic `audit`/`tune` already use for their own
/// teacher-depth lookups.
fn select_teacher<'a>(
    rec: &'a PositionRecord,
    engine: &str,
    depth: u32,
) -> Option<&'a Observation> {
    let matching: Vec<&Observation> = rec
        .observations
        .iter()
        .filter(|o| o.engine == engine)
        .collect();
    find_at_depth(&matching, depth)
}

/// Deterministically permutes a dataset into training order -- the first command in this
/// codebase that actually reorders output (every other seeded command preserves input-line or
/// rank order). Sorts by `(seeded_hash(seed, sample_id), sample_id)` rather than a bare hash, so
/// the order is a genuine total order (no unspecified tie-break) and therefore byte-identical
/// across runs/toolchains for the same input+seed.
fn cmd_shuffle(args: ShuffleArgs) -> Result<()> {
    if args.block_size == 0 {
        anyhow::bail!("--block-size must be > 0");
    }
    let (mut ordered, _broken) = load_records(&args.input)?;
    let total = ordered.len();

    ordered.sort_by_cached_key(|r| {
        let id = sample_id(r);
        (seeded_hash(args.seed, &id), id)
    });
    let sample_ids: Vec<String> = ordered.iter().map(sample_id).collect();

    let mut writer = AtomicWriter::new(&args.out)?;
    for record in &ordered {
        serde_json::to_writer(&mut writer, record)?;
        writer.write_all(b"\n")?;
    }
    writer.commit()?;

    if let Some(order_manifest_path) = &args.order_manifest {
        let mut order_writer = AtomicWriter::new(order_manifest_path)?;
        for (i, rec) in ordered.iter().enumerate() {
            let teacher = match (&args.teacher_engine, args.teacher_depth) {
                (Some(engine), Some(depth)) => select_teacher(rec, engine, depth),
                _ => None,
            };
            let (teacher_cp, teacher_depth) = match teacher {
                Some(obs) => (
                    match obs.score {
                        Score::Cp { value } => Some(value),
                        Score::Mate { .. } => None,
                    },
                    Some(obs.depth),
                ),
                None => (None, None),
            };
            let line = OrderManifestLine {
                training_position: i as u64 + 1,
                sample_id: sample_ids[i].clone(),
                game_id: shogiesa_stratify::group_key(&rec.source),
                game_position_index: rec.source.ply,
                outcome: rec.game_result.as_ref().map(|g| g.outcome),
                teacher_cp,
                teacher_depth,
                source_file: rec.source.path.clone(),
                split_id: args.split_id.clone(),
                shuffle_seed: args.seed,
                block_id: i as u64 / args.block_size,
                opening_id: None,
            };
            serde_json::to_writer(&mut order_writer, &line)?;
            order_writer.write_all(b"\n")?;
        }
        order_writer.commit()?;
    }

    eprintln!(
        "done: {total} shuffled (seed={}) -> {:?}",
        args.seed, args.out
    );

    if let Some(manifest_path) = &args.manifest {
        let mut manifest = new_run_manifest("shuffle", &args.input);
        manifest.input_hash = hash_file(&args.input)?;
        manifest.records_read = total;
        manifest.records_kept = total;
        let id_refs: Vec<&[u8]> = sample_ids.iter().map(|s| s.as_bytes()).collect();
        manifest.order_hash = Some(hash_parts(&id_refs).to_hex().to_string());
        manifest.experiment_id.clone_from(&args.experiment_id);
        manifest.shuffle_seed = Some(args.seed);
        write_manifest(manifest_path, &manifest)?;
    }
    Ok(())
}

/// Black-perspective cp of a record's deepest observation. Thin wrapper over
/// `shogiesa_core::cp_from_black_perspective` -- kept here (rather than inlined at each call
/// site) since "pick the deepest observation, then convert" is a record-level operation the core
/// utility itself doesn't know about.
fn eval_black(rec: &PositionRecord) -> Option<i64> {
    rec.observations
        .iter()
        .max_by_key(|o| o.depth)
        .and_then(|o| match o.score {
            Score::Cp { value } => Some(cp_from_black_perspective_i64(
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
        let evals: Vec<Option<i64>> = indices.iter().map(|&i| eval_black(&records[i])).collect();
        for j in 1..indices.len() {
            if let (Some(e0), Some(e1)) = (evals[j - 1], evals[j])
                && (e1 - e0).abs() >= i64::from(blunder_threshold)
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
        let evals: Vec<Option<i64>> = indices.iter().map(|&i| eval_black(&records[i])).collect();

        // Losing positions: side to move's eval worse than -threshold
        if let Some(threshold) = args.losing_threshold {
            for (j, &idx) in indices.iter().enumerate() {
                if let Some(eval) = evals[j] {
                    let side_eval = match records[idx].tags.side_to_move {
                        SideToMove::Black => eval,
                        SideToMove::White => -eval,
                    };
                    if side_eval < -i64::from(threshold) && !records[idx].observations.is_empty() {
                        keep.insert(idx);
                    }
                }
            }
        }
    }

    let mut writer = AtomicWriter::new(&args.out)?;
    let mut mined = 0usize;
    for (i, rec) in records.iter().enumerate() {
        if keep.contains(&i) {
            serde_json::to_writer(&mut writer, rec)?;
            writer.write_all(b"\n")?;
            mined += 1;
        }
    }
    writer.commit()?;
    eprintln!(
        "done: {mined}/{total} hard positions mined → {:?}",
        args.out
    );
    Ok(())
}

// EvalBucket, eval_bucket_of, bucket_key now live in shogiesa-stratify (imported above) --
// shogi-specific bucketing logic, shared by every consumer that needs "which bucket is this
// record in".

fn cmd_balance(args: BalanceArgs) -> Result<()> {
    if args.by.is_empty() {
        anyhow::bail!("--by requires at least one of: phase, side, eval-bucket, wdl");
    }
    let by_phase = args.by.iter().any(|s| s == "phase");
    let by_side = args.by.iter().any(|s| s == "side");
    let by_eval = args.by.iter().any(|s| s == "eval-bucket");
    let by_wdl = args.by.iter().any(|s| s == "wdl");

    // Pass 1: tally each bucket's size -- needed before any record can be ranked, since `target`
    // defaults to the smallest bucket's size, which can't be known until every bucket has been
    // seen at least once.
    let mut bucket_sizes: HashMap<String, usize> = HashMap::new();
    let mut pass1_broken = 0usize;
    for item in open_position_jsonl(&args.input)? {
        // parse errors are warned once, in pass 2 below -- not duplicated here
        if let Some(record) = permissive_position_record_with_diagnostics(
            item,
            &args.input,
            &mut pass1_broken,
            false,
        )? {
            *bucket_sizes
                .entry(bucket_key(&record, by_phase, by_side, by_eval, by_wdl))
                .or_default() += 1;
        }
    }
    let min_size = bucket_sizes.values().copied().min().unwrap_or(0);
    let target = args.target.unwrap_or(min_size);

    // Pass 2: re-stream, keeping a bounded top-`target` heap per bucket, keyed by SFEN (matching
    // today's "sort by SFEN, take first N" exactly) -- memory O(bucket count x target) instead of
    // O(n).
    let mut total = 0usize;
    let mut broken = 0usize;
    let mut heaps: HashMap<String, BinaryHeap<HeapEntry<String, PositionRecord>>> = HashMap::new();
    for item in open_position_jsonl(&args.input)? {
        let Some(record) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
        };
        let bucket = bucket_key(&record, by_phase, by_side, by_eval, by_wdl);
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
    let mut kept: Vec<HeapEntry<String, PositionRecord>> =
        heaps.into_values().flat_map(|h| h.into_vec()).collect();
    kept.sort_by_key(|e| e.index);

    let mut writer = AtomicWriter::new(&args.out)?;
    for entry in &kept {
        serde_json::to_writer(&mut writer, &entry.record)?;
        writer.write_all(b"\n")?;
    }
    writer.commit()?;
    eprintln!(
        "done: {}/{total} selected (target {target}/bucket, {} buckets) → {:?}",
        kept.len(),
        bucket_sizes.len(),
        args.out
    );
    if let Some(manifest_path) = &args.manifest {
        let mut manifest = new_run_manifest("balance", &args.input);
        manifest.input_hash = hash_file(&args.input)?;
        manifest.records_read = total;
        manifest.records_kept = kept.len();
        manifest.records_dropped = total - kept.len();
        let kept_records: Vec<PositionRecord> = kept.into_iter().map(|e| e.record).collect();
        accumulate_coverage(&mut manifest, &kept_records);
        write_manifest(manifest_path, &manifest)?;
    }
    Ok(())
}

const STRATIFY_FORMAT_VERSION: u32 = 1;

/// Loads a `--write-template`-produced (and hand-edited) quota file. Unlike `filter --preset`'s
/// `load_quality_config_preset` (which selects one of several named candidates via a trailing
/// `:label`), a quota file holds exactly one quota set -- no selector suffix, so the error surface
/// is simpler: unreadable file, malformed JSON, or an empty `quotas` map (nothing to keep).
fn load_stratify_quota_file(path: &Path) -> Result<QuotaSpec> {
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read quota file {path:?}"))?;
    let file: QuotaSpec =
        serde_json::from_str(&text).with_context(|| format!("cannot parse quota file {path:?}"))?;
    if file.quotas.is_empty() {
        anyhow::bail!("quota file {path:?} has an empty \"quotas\" map -- nothing to keep");
    }
    Ok(file)
}

fn cmd_stratify(args: StratifyArgs) -> Result<()> {
    if let Some(template_path) = args.write_template.clone() {
        return cmd_stratify_write_template(&args, &template_path);
    }
    let Some(quota_path) = args.quota.clone() else {
        anyhow::bail!("stratify requires exactly one of --write-template or --quota");
    };
    let out = args
        .out
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--quota requires --out"))?;
    cmd_stratify_apply(&args, &quota_path, &out)
}

fn cmd_stratify_write_template(args: &StratifyArgs, template_path: &Path) -> Result<()> {
    if args.by.is_empty() {
        anyhow::bail!(
            "--write-template requires --by (at least one of: phase, side, eval-bucket, wdl)"
        );
    }
    let by_phase = args.by.iter().any(|s| s == "phase");
    let by_side = args.by.iter().any(|s| s == "side");
    let by_eval = args.by.iter().any(|s| s == "eval-bucket");
    let by_wdl = args.by.iter().any(|s| s == "wdl");

    // Same tally `balance`'s pass 1 already does -- observed counts become the template's starting
    // point, which the user then hand-edits down to their desired per-bucket targets.
    let mut bucket_sizes: HashMap<String, usize> = HashMap::new();
    let mut broken = 0usize;
    for item in open_position_jsonl(&args.input)? {
        let Some(record) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
        };
        *bucket_sizes
            .entry(bucket_key(&record, by_phase, by_side, by_eval, by_wdl))
            .or_default() += 1;
    }

    let file = QuotaSpec {
        stratify_format_version: STRATIFY_FORMAT_VERSION,
        input: args.input.display().to_string(),
        by: args.by.clone(),
        quotas: bucket_sizes.into_iter().collect(),
    };
    atomic_write_bytes(
        template_path,
        serde_json::to_string_pretty(&file)?.as_bytes(),
    )
    .with_context(|| format!("cannot write {template_path:?}"))?;
    eprintln!(
        "done: {} bucket(s) observed, {broken} record(s) skipped → {:?} (edit \"quotas\" counts down, then run `stratify --quota`)",
        file.quotas.len(),
        template_path
    );
    Ok(())
}

fn cmd_stratify_apply(args: &StratifyArgs, quota_path: &Path, out: &Path) -> Result<()> {
    let quota_file = load_stratify_quota_file(quota_path)?;
    let by_phase = quota_file.by.iter().any(|s| s == "phase");
    let by_side = quota_file.by.iter().any(|s| s == "side");
    let by_eval = quota_file.by.iter().any(|s| s == "eval-bucket");
    let by_wdl = quota_file.by.iter().any(|s| s == "wdl");
    let seed = args.seed;

    let mut broken = 0usize;
    let mut read_error: Option<anyhow::Error> = None;
    let records = open_position_jsonl(&args.input)?.filter_map(|item| {
        match permissive_position_record(item, &args.input, &mut broken) {
            Ok(record) => record,
            Err(error) => {
                read_error.get_or_insert(error);
                None
            }
        }
    });
    let result = group_aware_fill(
        records,
        &quota_file.quotas,
        seed,
        |r| bucket_key(r, by_phase, by_side, by_eval, by_wdl),
        |r| shogiesa_stratify::group_key(&r.source),
    );
    if let Some(error) = read_error {
        return Err(error);
    }

    let mut writer = AtomicWriter::new(out)?;
    for record in &result.kept {
        serde_json::to_writer(&mut writer, record)?;
        writer.write_all(b"\n")?;
    }
    writer.commit()?;

    let over_quota = result.quota_candidates - result.kept.len();
    eprintln!(
        "done: {}/{} valid record(s) kept, {broken} malformed/unsupported record(s) skipped ({} bucket(s) in quota file) → {out:?}",
        result.kept.len(),
        result.total,
        quota_file.quotas.len()
    );
    eprintln!("drop reasons:");
    eprintln!("  bucket_not_in_quota      {}", result.bucket_not_in_quota);
    eprintln!("  over_quota               {over_quota}");

    if let Some(manifest_path) = &args.manifest {
        let mut manifest = new_run_manifest("stratify", &args.input);
        manifest.input_hash = hash_file(&args.input)?;
        manifest.records_read = result.total + broken;
        manifest.records_kept = result.kept.len();
        manifest.records_dropped = result.total - result.kept.len() + broken;
        manifest.drop_reasons.insert(
            "bucket_not_in_quota".to_string(),
            result.bucket_not_in_quota,
        );
        manifest
            .drop_reasons
            .insert("over_quota".to_string(), over_quota);
        if broken > 0 {
            manifest
                .drop_reasons
                .insert("parse_error".to_string(), broken);
        }
        accumulate_coverage(&mut manifest, &result.kept);
        manifest.max_root_share_in_any_bucket = result.max_group_share_in_any_bucket;
        manifest.distinct_roots_kept = Some(result.distinct_groups_kept);

        write_manifest(manifest_path, &manifest)?;
    }
    Ok(())
}

/// Bump only when `make-gate-openings`' own selection logic changes (the tie-break/ranking/
/// dedup-key scheme `group_aware_fill` implements) -- not for unrelated edits to this command.
const GATE_OPENING_SELECTION_ALGORITHM_VERSION: &str = "gate-openings-group-aware-fill-v1";

/// Standard "simple" shogi piece-value table (integer points, not centipawns or an engine's own
/// eval scale). No existing material/point-value concept exists anywhere else in this codebase
/// (confirmed via grep) -- this is a fresh judgment call, not a verified external standard, with
/// no in-repo precedent to match against. Promoted minor pieces (tokin/narikyo/narikei/narigin)
/// take Gold's value, since they move exactly like Gold; promoted major pieces (horse/dragon)
/// get their own higher values, since they keep their original long-range move on top of a
/// king-step. `total_material`'s golden-value test is this table's own regression guard.
fn piece_point_value(pt: PieceType) -> u32 {
    match pt {
        PieceType::Pawn => 1,
        PieceType::Lance => 3,
        PieceType::Knight => 3,
        PieceType::Silver => 5,
        PieceType::Gold => 6,
        PieceType::Bishop => 8,
        PieceType::Rook => 10,
        PieceType::King => 0,
        PieceType::ProPawn | PieceType::ProLance | PieceType::ProKnight | PieceType::ProSilver => 6,
        PieceType::Horse => 10,
        PieceType::Dragon => 12,
    }
}

/// Sums `piece_point_value` over every piece on the board and in every hand, both sides combined
/// -- a single scalar describing how much material is still on the board for this position,
/// independent of whose side it favors. `None` (not `Some(0)`) when `sfen` doesn't parse, same
/// convention as `has_legal_move_lite`: a shogiesa-accepted-but-otherwise-unparseable SFEN isn't
/// conflated with a genuinely bare board.
fn total_material(sfen: &str) -> Option<u32> {
    // hand[color_idx][piece_idx]: R=0,B=1,G=2,S=3,N=4,L=5,P=6 (Board::hand's own documented
    // convention, mirrored here since PieceType::hand_idx is private to shogiesa-core).
    const HAND_ORDER: [PieceType; 7] = [
        PieceType::Rook,
        PieceType::Bishop,
        PieceType::Gold,
        PieceType::Silver,
        PieceType::Knight,
        PieceType::Lance,
        PieceType::Pawn,
    ];
    let board = Board::from_sfen(sfen).ok()?;
    let mut total = 0u32;
    for row in &board.grid {
        for (_, pt) in row.iter().flatten() {
            total += piece_point_value(*pt);
        }
    }
    for hand in &board.hand {
        for (pt, &count) in HAND_ORDER.iter().zip(hand.iter()) {
            total += piece_point_value(*pt) * u32::from(count);
        }
    }
    Some(total)
}

/// The first 3 whitespace-separated SFEN fields (board, side, hand) -- deliberately drops the
/// trailing move-count field, since a match-runner starts a *game* from this position and what
/// happens next depends only on board/side/hand, not on how many moves it took to reach it. Two
/// records with identical board+side+hand from different games/plies are the same starting
/// position for gating purposes and must collapse to one, or a duplicate wastes a quota slot that
/// could have gone to a genuinely different opening.
///
/// ponytail: NOT `zobrist_from_sfen` -- that hash covers board+side only (built for
/// `extract --dedup-zobrist`'s transposition-detection use case, where hand is naturally
/// consistent within one game). Reusing it here would silently conflate two unrelated positions
/// that share a board+side but differ in hand, which are NOT the same starting position.
fn gate_opening_dedup_key(sfen: &str) -> String {
    sfen.split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Why: gate opening suites feed external match runners as playable starting positions; a
/// syntactically valid SFEN can still be useless for gating if the side to move has zero legal
/// moves (checkmate/stalemate), and filtering here avoids every downstream engine/gate script
/// repeating the same defensive check.
///
/// Reuses the `shogi_core`/`shogi_usi_parser`/`shogi_legality_lite` crate family instead of
/// hand-rolling shogi legal-move generation (drops, forced promotion, nifu, uchifuzume, pins) --
/// that's a correctness project of its own shogiesa doesn't otherwise need. `Err` means the
/// external parser rejected a SFEN shogiesa's own `Sfen::parse` already accepted as syntactically
/// valid (a format-compatibility gap, not a bad record) -- callers must not conflate this with a
/// confirmed zero-legal-move position.
fn has_legal_move_lite(sfen: &str) -> Result<bool, shogi_usi_parser::Error> {
    use shogi_usi_parser::FromUsi;
    // shogi_usi_parser::PartialPosition::from_usi requires a leading "sfen " token (or
    // "startpos"); shogiesa's own SFEN strings are the bare 4-field body.
    let pos = shogi_core::PartialPosition::from_usi(&format!("sfen {sfen}"))?;
    Ok(!shogi_legality_lite::all_legal_moves_partial(&pos).is_empty())
}

/// Whether `mv` (a USI move string, e.g. "7g7f" or "P*5e") is one of the legal moves in `sfen`'s
/// position -- used by --usi-strict to catch an engine reporting a bestmove that isn't legal in
/// the position it was actually given (a symptom of protocol desync: the engine answering for a
/// stale/different position). `Err` means the external parser rejected a SFEN shogiesa's own
/// `Sfen::parse` already accepted -- callers must not flag on that, same convention as
/// `has_legal_move_lite`.
fn is_legal_bestmove_lite(sfen: &str, mv: &str) -> Result<bool, shogi_usi_parser::Error> {
    use shogi_core::ToUsi;
    use shogi_usi_parser::FromUsi;
    let pos = shogi_core::PartialPosition::from_usi(&format!("sfen {sfen}"))?;
    Ok(shogi_legality_lite::all_legal_moves_partial(&pos)
        .iter()
        .any(|m| m.to_usi_owned() == mv))
}

#[cfg(test)]
mod has_legal_move_lite_tests {
    use super::{has_legal_move_lite, is_legal_bestmove_lite};

    #[test]
    fn startpos_has_legal_moves() {
        let startpos = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
        assert!(has_legal_move_lite(startpos).unwrap());
    }

    #[test]
    fn checkmate_has_no_legal_moves() {
        // One of the shortest mate sequences from startpos (verified against
        // shogi_legality_lite's own `status_partial_works` test, which asserts the side to
        // move here -- White -- is checkmated).
        let mate = "lnsg1gsnl/5rkb1/ppppppp+Pp/9/9/9/PPPPPPP1P/1B5R1/LNSGKGSNL w P 8";
        assert!(!has_legal_move_lite(mate).unwrap());
    }

    // Blocking pre-step for --usi-strict's illegal-bestmove detection: proves
    // `shogi_core::ToUsi::to_usi_owned()`'s rendering of a drop and of a promotion actually
    // matches the USI move syntax real engines emit (and that shogiesa's own SFEN strings feed
    // `shogi_usi_parser` correctly), before is_legal_bestmove_lite is wired into analyze_record.
    // If either mismatched, every drop/promotion bestmove would false-positive as illegal.
    #[test]
    fn is_legal_bestmove_lite_accepts_drop_and_promotion() {
        let startpos = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
        assert!(is_legal_bestmove_lite(startpos, "7g7f").unwrap());
        // King can only step one square; 5i to 5g is a two-rank jump, illegal for the piece type
        // regardless of board state.
        assert!(!is_legal_bestmove_lite(startpos, "5i5g").unwrap());

        // Black pawn on 8c (one step from promotion zone), a captured pawn in hand, otherwise
        // empty board -- legal moves include the drop on file5 (nifu only blocks file8, which
        // already has a pawn) and the promoting/non-promoting advance of the 8c pawn to 8b.
        let drop_and_promo = "8k/9/1P7/9/9/9/9/9/8K b P 1";
        assert!(is_legal_bestmove_lite(drop_and_promo, "P*5e").unwrap());
        assert!(is_legal_bestmove_lite(drop_and_promo, "8c8b+").unwrap());
        assert!(is_legal_bestmove_lite(drop_and_promo, "8c8b").unwrap());
        assert!(!is_legal_bestmove_lite(drop_and_promo, "5e5d").unwrap());
    }
}

#[cfg(test)]
mod total_material_tests {
    use super::total_material;

    // Regression guard for `piece_point_value`'s table: standard starting material, both sides,
    // king excluded -- 2*(9*1 + 2*3 + 2*3 + 2*5 + 2*6 + 8 + 10) = 2*61 = 122.
    #[test]
    fn total_material_of_startpos_is_122() {
        let startpos = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1";
        assert_eq!(total_material(startpos), Some(122));
    }

    #[test]
    fn total_material_counts_hand_pieces_too() {
        // Bare kings plus one black pawn in hand -- material is exactly one pawn's worth.
        let sfen = "8k/9/9/9/9/9/9/9/8K b P 1";
        assert_eq!(total_material(sfen), Some(1));
    }

    #[test]
    fn total_material_of_bare_kings_is_zero() {
        let sfen = "8k/9/9/9/9/9/9/9/8K b - 1";
        assert_eq!(total_material(sfen), Some(0));
    }

    #[test]
    fn total_material_rejects_unparseable_sfen() {
        assert_eq!(total_material("not a real sfen"), None);
    }
}

fn cmd_make_gate_openings(args: MakeGateOpeningsArgs) -> Result<()> {
    let seed = args.seed;
    let reader = open_position_jsonl(&args.input)?;

    let mut total = 0usize;
    let mut invalid_sfen = 0usize;
    let mut below_min_ply = 0usize;
    let mut above_max_ply = 0usize;
    let mut unplayable = 0usize;
    let mut duplicate_sfen = 0usize;
    let mut seen: HashSet<String> = HashSet::new();
    let mut broken = 0usize;
    let mut read_error: Option<anyhow::Error> = None;

    // Domain-specific pre-filtering (sfen validity, ply range, dedup) happens here, before the
    // record ever reaches the generic group-aware fill below -- `make-gate-openings` has no
    // bucketing dimension, so every survivor goes into one universal `""` bucket.
    let filtered: Vec<PositionRecord> = reader
        .filter_map(|item| {
            let record = match permissive_position_record(item, &args.input, &mut broken) {
                Ok(Some(record)) => record,
                Ok(None) => return None,
                Err(error) => {
                    read_error.get_or_insert(error);
                    return None;
                }
            };
            total += 1;

            if Sfen::parse(&record.sfen).is_err() {
                invalid_sfen += 1;
                return None;
            }
            if record.source.ply < args.min_ply {
                below_min_ply += 1;
                return None;
            }
            if args.max_ply.is_some_and(|max| record.source.ply > max) {
                above_max_ply += 1;
                return None;
            }
            // Why: gate opening suites feed external match runners as playable starting
            // positions; a syntactically valid SFEN can still be useless for gating if the side
            // to move has zero legal moves (checkmate/stalemate), and filtering here avoids every
            // downstream engine/gate script repeating the same defensive check.
            if !args.allow_unplayable {
                match has_legal_move_lite(&record.sfen) {
                    Ok(true) => {}
                    Ok(false) => {
                        unplayable += 1;
                        return None;
                    }
                    Err(e) => {
                        tracing::warn!(sfen = %record.sfen, "cannot evaluate legality, skipping: {e}");
                        unplayable += 1;
                        return None;
                    }
                }
            }
            // Dedup before the fill's rank assignment -- a root full of duplicate SFENs must not
            // inflate its own rank counter and lose priority to a genuinely diverse root.
            if !seen.insert(gate_opening_dedup_key(&record.sfen)) {
                duplicate_sfen += 1;
                return None;
            }
            Some(record)
        })
        .collect();
    if let Some(error) = read_error {
        return Err(error);
    }

    let quotas = BTreeMap::from([(String::new(), args.count)]);
    let result = group_aware_fill(
        filtered.into_iter(),
        &quotas,
        seed,
        |_| String::new(),
        |r| shogiesa_stratify::group_key(&r.source),
    );
    let kept = result.kept;

    let mut writer = AtomicWriter::new(&args.out)?;
    for record in &kept {
        writer.write_all(record.sfen.as_bytes())?;
        writer.write_all(b"\n")?;
    }
    writer.commit()?;

    let over_count = result.quota_candidates - kept.len();
    eprintln!("done: {}/{total} kept → {:?}", kept.len(), args.out);
    eprintln!("drop reasons:");
    eprintln!("  invalid_sfen             {invalid_sfen}");
    eprintln!("  below_min_ply            {below_min_ply}");
    eprintln!("  above_max_ply            {above_max_ply}");
    eprintln!("  unplayable               {unplayable}");
    eprintln!("  duplicate_sfen           {duplicate_sfen}");
    eprintln!("  over_count               {over_count}");

    if let Some(manifest_path) = &args.manifest {
        let mut manifest = new_run_manifest("make-gate-openings", &args.input);
        manifest.input_hash = hash_file(&args.input)?;
        manifest.records_read = total;
        manifest.records_kept = kept.len();
        manifest.records_dropped = total - kept.len();
        manifest
            .drop_reasons
            .insert("invalid_sfen".to_string(), invalid_sfen);
        manifest
            .drop_reasons
            .insert("below_min_ply".to_string(), below_min_ply);
        manifest
            .drop_reasons
            .insert("above_max_ply".to_string(), above_max_ply);
        manifest
            .drop_reasons
            .insert("unplayable".to_string(), unplayable);
        manifest
            .drop_reasons
            .insert("duplicate_sfen".to_string(), duplicate_sfen);
        manifest
            .drop_reasons
            .insert("over_count".to_string(), over_count);
        accumulate_coverage(&mut manifest, &kept);

        // Reinterpretation of `stratify`'s per-bucket root-share stat for a single universal
        // "bucket" (the whole suite): `group_aware_fill` already scopes this to buckets with >=2
        // kept records, same reasoning as stratify (a singleton suite is trivially "100% one
        // root").
        manifest.max_root_share_in_any_bucket = result.max_group_share_in_any_bucket;
        manifest.distinct_roots_kept = Some(result.distinct_groups_kept);

        manifest.selection_seed = Some(seed);
        // `quota_candidates`, not `filtered.len()` -- `filtered` was already moved into
        // `group_aware_fill` above.
        manifest.canonical_valid_count = Some(result.quota_candidates);
        manifest.selection_algorithm_version =
            Some(GATE_OPENING_SELECTION_ALGORITHM_VERSION.to_string());
        manifest.output_sha256 = Some(hash_file_sha256(&args.out)?);

        let mut source_distribution: BTreeMap<String, usize> = BTreeMap::new();
        let mut phase_distribution: BTreeMap<GamePhase, usize> = BTreeMap::new();
        let mut material_distribution: BTreeMap<u32, usize> = BTreeMap::new();
        for record in &kept {
            *source_distribution
                .entry(shogiesa_stratify::group_key(&record.source))
                .or_default() += 1;
            *phase_distribution.entry(record.tags.phase).or_default() += 1;
            if let Some(material) = total_material(&record.sfen) {
                *material_distribution.entry(material).or_default() += 1;
            }
        }
        manifest.source_distribution = Some(source_distribution);
        manifest.phase_distribution = Some(phase_distribution);
        manifest.material_distribution = Some(material_distribution);

        write_manifest(manifest_path, &manifest)?;
    }
    Ok(())
}

// Why `select` exists at all: re-labeling an entire dataset at higher depth to chase accuracy
// costs the same whether 1% or 100% of it is actually wrong or uninformative. Each strategy
// below ranks positions by a signal that predicts "worth a second look" and reuses the exact
// judgment logic `filter`/`mine`/`balance` already have (evaluate_quality, blunder-window
// detection, bucket keys) rather than re-deriving what "uncertain"/"hard"/"thin" means.
// TotalF32 now lives in stratifykit-core (imported above).

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
    let mut total = 0usize;
    let mut broken = 0usize;
    let mut heap: BinaryHeap<HeapEntry<(TotalF32, u64), PositionRecord>> =
        BinaryHeap::with_capacity(count.saturating_add(1));
    for item in open_position_jsonl(&args.input)? {
        let Some(record) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
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
    let mut pass1_broken = 0usize;
    for item in open_position_jsonl(&args.input)? {
        // parse errors are warned once, in pass 2 below -- not duplicated here
        if let Some(record) = permissive_position_record_with_diagnostics(
            item,
            &args.input,
            &mut pass1_broken,
            false,
        )? {
            *bucket_counts
                .entry(bucket_key(&record, true, true, true, false))
                .or_default() += 1;
        }
    }

    let mut total = 0usize;
    let mut broken = 0usize;
    let mut heap: BinaryHeap<HeapEntry<(usize, u64), PositionRecord>> =
        BinaryHeap::with_capacity(count.saturating_add(1));
    for item in open_position_jsonl(&args.input)? {
        let Some(record) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
        };
        let bucket_count = bucket_counts[&bucket_key(&record, true, true, true, false)];
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

/// `select --strategy hard`: unlike `uncertain`/`coverage` above, fully materialized by default.
/// `blunder_adjacent_indices` fundamentally needs a whole game's positions grouped together, which
/// isn't safe to stream without assuming the input is contiguously grouped by source (an
/// assumption this codebase doesn't guarantee elsewhere) -- so this stays O(n) memory and is the
/// correctness-preserving fallback. `select_hard_grouped_streaming` below is the O(one game +
/// --count) alternative for callers who can guarantee that assumption
/// (`--assume-grouped-by-source`).
fn select_hard_materialized(args: &SelectArgs) -> Result<(usize, Vec<PositionRecord>)> {
    let (records, _) = load_records(&args.input)?;
    let total = records.len();
    let count = args.count.min(total);
    let seed = args.seed;

    let blunder_set =
        blunder_adjacent_indices(&records, args.blunder_threshold, args.blunder_window);
    let hardness = |i: usize| -> (bool, bool, i64) {
        let rec = &records[i];
        let cp_scores: Vec<i32> = rec
            .observations
            .iter()
            .filter_map(|o| match o.score {
                Score::Cp { value } => Some(value),
                Score::Mate { .. } => None,
            })
            .collect();
        let swing = score_swing_i64(&cp_scores).unwrap_or(0);
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

/// Per-game hardness for one already-ply-sorted group of positions from the same game -- the
/// blunder-adjacency + disagreement + swing math `select_hard_materialized` computes globally via
/// `blunder_adjacent_indices`, scoped to a single game so `select_hard_grouped_streaming` only
/// ever needs one game's positions in memory at a time. New code, not a refactor of
/// `blunder_adjacent_indices`/`select_hard_materialized` -- those stay untouched as the tested,
/// correctness-preserving fallback.
fn hardness_for_group(
    group: &[PositionRecord],
    blunder_threshold: i32,
    blunder_window: usize,
) -> Vec<(bool, bool, i64)> {
    let evals: Vec<Option<i64>> = group.iter().map(eval_black).collect();
    let mut is_blunder_adjacent = vec![false; group.len()];
    for j in 1..group.len() {
        if let (Some(e0), Some(e1)) = (evals[j - 1], evals[j])
            && (e1 - e0).abs() >= i64::from(blunder_threshold)
        {
            let lo = j.saturating_sub(blunder_window);
            let hi = (j + blunder_window + 1).min(group.len());
            for k in lo..hi {
                if !group[k].observations.is_empty() {
                    is_blunder_adjacent[k] = true;
                }
            }
        }
    }
    group
        .iter()
        .enumerate()
        .map(|(i, rec)| {
            let cp_scores: Vec<i32> = rec
                .observations
                .iter()
                .filter_map(|o| match o.score {
                    Score::Cp { value } => Some(value),
                    Score::Mate { .. } => None,
                })
                .collect();
            (
                is_blunder_adjacent[i],
                !bestmove_agreement(&rec.observations),
                score_swing_i64(&cp_scores).unwrap_or(0),
            )
        })
        .collect()
}

/// Heap key for `select_hard_grouped_streaming`: `push_bounded` keeps the smallest keys, and
/// `into_sorted_vec()` returns ascending order, but `select_hard_materialized` wants descending
/// hardness (hardest first) then ascending seeded-hash tie-break. `Reverse`-wrapping only the
/// hardness tuple gets that: a bigger original hardness becomes a smaller `Reverse` value (sorts
/// first), while the plain `u64` hash is left as-is since "smaller hash first" is already the
/// wanted tie-break direction.
type HardKey = (std::cmp::Reverse<(bool, bool, i64)>, u64);

/// Scores one buffered game's positions and pushes them into the shared global top-`count` heap,
/// then clears the buffer. Re-sorts by ply first (same as `blunder_adjacent_indices` does
/// internally) since "grouped by source" only guarantees contiguity by game, not ply order within
/// a game.
fn flush_hard_group(
    group: &mut Vec<PositionRecord>,
    heap: &mut BinaryHeap<HeapEntry<HardKey, PositionRecord>>,
    total: &mut usize,
    count: usize,
    seed: u64,
    blunder_threshold: i32,
    blunder_window: usize,
) {
    if group.is_empty() {
        return;
    }
    group.sort_by_key(|r| r.source.ply);
    let hardness = hardness_for_group(group, blunder_threshold, blunder_window);
    for (rec, h) in group.drain(..).zip(hardness) {
        let key = (std::cmp::Reverse(h), seeded_hash(seed, &rec.sfen));
        let index = *total;
        *total += 1;
        push_bounded(
            heap,
            count,
            HeapEntry {
                key,
                index,
                record: rec,
            },
        );
    }
}

/// Streaming `select --strategy hard --assume-grouped-by-source`: bounded by one game's positions
/// plus the top-`count` heap, instead of `select_hard_materialized`'s full dataset, PROVIDED the
/// input is already contiguously grouped by `source.path` (e.g. `extract`'s own output, or
/// anything already run through `split`). Group boundary is a `source.path` change -- matching
/// `blunder_adjacent_indices`'s exact existing grouping key, deliberately not `split_root_key`, so
/// a KIF variation's `#varN@ply`-suffixed path keeps counting as its own separate "game" for
/// blunder-adjacency purposes, same as `select_hard_materialized` does today. Output matches
/// `select_hard_materialized` exactly for grouped, ply-sorted-within-group input; it may only
/// differ in tie-break order among duplicate-SFEN positions right at the `--count` truncation
/// boundary, since the two paths track a different (but equally valid) last-resort tie-break
/// there -- materialized keeps file order, this keeps group-flush order.
fn select_hard_grouped_streaming(args: &SelectArgs) -> Result<(usize, Vec<PositionRecord>)> {
    let count = args.count;
    let seed = args.seed;
    let mut heap: BinaryHeap<HeapEntry<HardKey, PositionRecord>> =
        BinaryHeap::with_capacity(count.saturating_add(1));
    let mut total = 0usize;
    let mut broken = 0usize;
    let mut current_path: Option<String> = None;
    let mut group: Vec<PositionRecord> = Vec::new();

    for item in open_position_jsonl(&args.input)? {
        let Some(record) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
        };
        if current_path.as_deref() != Some(record.source.path.as_str()) {
            flush_hard_group(
                &mut group,
                &mut heap,
                &mut total,
                count,
                seed,
                args.blunder_threshold,
                args.blunder_window,
            );
            current_path = Some(record.source.path.clone());
        }
        group.push(record);
    }
    flush_hard_group(
        &mut group,
        &mut heap,
        &mut total,
        count,
        seed,
        args.blunder_threshold,
        args.blunder_window,
    );

    Ok((
        total,
        heap.into_sorted_vec()
            .into_iter()
            .map(|e| e.record)
            .collect(),
    ))
}

/// One lineprior-style observation, one JSONL line per move actually played. Distinct from
/// `PositionRecord`/`Observation` (a different schema for a different, external consumer, with
/// exactly one producer) so it lives here rather than in shogiesa-core.
#[derive(Debug, Serialize)]
struct LinePriorObservation {
    sequence_id: String,
    step: u32,
    state: String,
    action: String,
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    score: Option<f64>,
    weight: f64,
    source: String,
    tags: Vec<String>,
}

/// Manifest for `lineprior export` -- deliberately not `RunManifest`: none of that struct's
/// fields (labeled/engine/score-bound/depth stats) apply to this command's move-level JSONL, and
/// bolting sequence/outcome/tag distributions onto a struct already shared by unrelated commands
/// (label/filter/sample/...) would force those commands to carry meaningless zero-defaults.
#[derive(Debug, Serialize)]
struct LinePriorExportManifest {
    shogiesa_version: &'static str,
    git_sha: &'static str,
    input_path: String,
    input_hash: String,
    out_path: String,
    source: String,
    max_ply: Option<u32>,
    state_format: String,
    action_format: String,
    outcome_mode: String,
    score_mode: String,
    games_read: usize,
    games_skipped: usize,
    records_exported: usize,
    sequence_count: usize,
    outcome_distribution: BTreeMap<&'static str, usize>,
    /// Keyed on phase only (`phase_from_ply`'s `"opening"`/`"middlegame"`/`"endgame"`), not the
    /// full `tags` vec -- every record already carries `"shogi"`, so distributing over the whole
    /// vec would just add a useless `"shogi": records_exported` bucket next to the real counts.
    tag_distribution: BTreeMap<String, usize>,
    /// Same number as `outcome_distribution["unknown"]`, pulled out as its own field rather than
    /// tracked separately -- a one-glance check for "did KIF `変化` branches dilute this corpus"
    /// (they always resolve to `unknown`, see README's outcome-resolution caveat) without a
    /// second source of truth that could drift from the map.
    unknown_outcome_count: usize,
}

fn cmd_lineprior_export(args: LinePriorExportArgs) -> Result<()> {
    let paths = collect_game_paths(&args.input)?;
    if paths.is_empty() {
        anyhow::bail!("no .csa or .kif files found in {:?}", args.input);
    }

    let mut writer = AtomicWriter::new(&args.out)?;

    let mut total_games = 0usize;
    let mut total_moves = 0usize;
    let mut skipped_games = 0usize;
    let mut sequence_ids: HashSet<String> = HashSet::new();
    let mut outcome_distribution: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut tag_distribution: BTreeMap<String, usize> = BTreeMap::new();
    let mut input_hasher = blake3::Hasher::new();

    for path in &paths {
        total_games += 1;
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(path = %path.display(), "skipped: {e}");
                skipped_games += 1;
                continue;
            }
        };
        if args.manifest.is_some() {
            fold_file_hash(&mut input_hasher, &content);
        }
        let source_path = path.to_string_lossy().into_owned();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let result = match ext {
            "kif" | "ki2" => shogiesa_kif::extract_moves_from_str(&content, &source_path)
                .map_err(|e| e.to_string()),
            _ => shogiesa_csa::extract_moves_from_str(&content, &source_path)
                .map_err(|e| e.to_string()),
        };
        let (raw_moves, _outcome, _reason) = match result {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(path = %path.display(), "skipped: {e}");
                skipped_games += 1;
                continue;
            }
        };

        for raw in &raw_moves {
            // `continue`, not `break`: a KIF game's `raw_moves` is mainline-first then every
            // variation branch, and a variation restarts at a low ply -- breaking on the
            // mainline's first over-limit move would also skip every later variation row.
            if args.max_ply.is_some_and(|max| raw.ply > max) {
                continue;
            }
            let sequence_id = split_root_key(&raw.source).to_string();
            let outcome = raw.outcome.for_mover(raw.mover);
            let phase = phase_from_ply(raw.ply).to_string();

            sequence_ids.insert(sequence_id.clone());
            *outcome_distribution.entry(outcome).or_insert(0) += 1;
            *tag_distribution.entry(phase.clone()).or_insert(0) += 1;

            let obs = LinePriorObservation {
                sequence_id,
                step: raw.ply,
                state: raw.sfen_before.clone(),
                action: raw.usi_move.clone(),
                outcome,
                score: None,
                weight: 1.0,
                source: args.source.clone(),
                tags: vec!["shogi".to_string(), phase],
            };
            serde_json::to_writer(&mut writer, &obs)?;
            writer.write_all(b"\n")?;
            total_moves += 1;
        }
    }

    writer.commit()?;

    if let Some(manifest_path) = &args.manifest {
        let unknown_outcome_count = outcome_distribution.get("unknown").copied().unwrap_or(0);
        let manifest = LinePriorExportManifest {
            shogiesa_version: env!("CARGO_PKG_VERSION"),
            git_sha: env!("SHOGIESA_GIT_SHA"),
            input_path: args.input.display().to_string(),
            input_hash: input_hasher.finalize().to_hex().to_string(),
            out_path: args.out.display().to_string(),
            source: args.source.clone(),
            max_ply: args.max_ply,
            state_format: args.state_format.clone(),
            action_format: args.action_format.clone(),
            outcome_mode: args.outcome_mode.clone(),
            score_mode: args.score_mode.clone(),
            games_read: total_games,
            games_skipped: skipped_games,
            records_exported: total_moves,
            sequence_count: sequence_ids.len(),
            outcome_distribution,
            tag_distribution,
            unknown_outcome_count,
        };
        atomic_write_bytes(
            manifest_path,
            serde_json::to_string_pretty(&manifest)?.as_bytes(),
        )
        .with_context(|| format!("cannot write {manifest_path:?}"))?;
    }

    eprintln!(
        "done: {total_games} games, {total_moves} moves exported, {skipped_games} games skipped → {:?}",
        args.out
    );
    Ok(())
}

fn cmd_select(args: SelectArgs) -> Result<()> {
    let (total, ranked_records) = match args.strategy.as_str() {
        "uncertain" => select_uncertain_streaming(&args)?,
        "coverage" => select_coverage_streaming(&args)?,
        "hard" => {
            if args.assume_grouped_by_source {
                select_hard_grouped_streaming(&args)?
            } else {
                select_hard_materialized(&args)?
            }
        }
        other => anyhow::bail!("unknown --strategy {other:?} (expected uncertain/hard/coverage)"),
    };

    // Output in ranked order (most-worth-a-look first), unlike `sample`/`balance` which restore
    // input order -- a re-labeling queue is more useful read top-to-bottom by priority than by
    // original file position.
    let mut writer = AtomicWriter::new(&args.out)?;
    for rec in &ranked_records {
        serde_json::to_writer(&mut writer, rec)?;
        writer.write_all(b"\n")?;
    }
    writer.commit()?;
    eprintln!(
        "done: {}/{total} selected (strategy={}, seed={}) → {:?}",
        ranked_records.len(),
        args.strategy,
        args.seed,
        args.out
    );
    Ok(())
}

/// Every `.json` file under a `label --cache-dir`'s two-level shard layout
/// (`cache_dir/xx/<64-hex-hash>.json`, see `label_cache_path`). Filtering strictly on the `.json`
/// extension skips `write_cache_entry_atomically`'s in-flight temp files
/// (`<hash>.json.tmp.<pid>.<tid>`) by construction, so a walk never races a concurrent `label`
/// process sharing the same cache dir.
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

#[derive(Debug)]
struct GameInput {
    path: PathBuf,
    /// Portable provenance used only for recursively discovered directory inputs.
    source_path: Option<String>,
}

#[derive(Debug)]
struct RecursiveGamePath {
    path: PathBuf,
    source_path: String,
}

#[derive(Debug, Default)]
struct RecursiveDiscovery {
    paths: Vec<RecursiveGamePath>,
    skipped_files: usize,
    skipped_symlinks: usize,
    unreadable_directories: usize,
    unreadable_entries: usize,
}

fn normalized_relative_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("cannot make {:?} relative to {:?}", path, root))?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn collect_game_paths_recursive(root: &Path) -> Result<RecursiveDiscovery> {
    let mut discovery = RecursiveDiscovery::default();
    collect_game_paths_recursive_from(root, root, true, &mut discovery)?;
    discovery
        .paths
        .sort_by(|left, right| left.source_path.cmp(&right.source_path));
    Ok(discovery)
}

fn collect_game_paths_recursive_from(
    root: &Path,
    directory: &Path,
    is_root: bool,
    discovery: &mut RecursiveDiscovery,
) -> Result<()> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if !is_root => {
            tracing::warn!(path = %directory.display(), "unreadable directory skipped: {error}");
            discovery.unreadable_directories += 1;
            return Ok(());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read directory {:?}", directory));
        }
    };

    let mut readable_entries = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => readable_entries.push(entry),
            Err(error) => {
                tracing::warn!(path = %directory.display(), "unreadable directory entry skipped: {error}");
                discovery.unreadable_entries += 1;
            }
        }
    }
    readable_entries.sort_by_key(|entry| entry.file_name());

    for entry in readable_entries {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                tracing::warn!(path = %path.display(), "unreadable directory entry skipped: {error}");
                discovery.unreadable_entries += 1;
                continue;
            }
        };

        if file_type.is_symlink() {
            tracing::warn!(path = %path.display(), "symlink skipped during recursive discovery");
            discovery.skipped_symlinks += 1;
        } else if file_type.is_dir() {
            collect_game_paths_recursive_from(root, &path, false, discovery)?;
        } else if file_type.is_file()
            && matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("csa" | "kif" | "ki2")
            )
        {
            discovery.paths.push(RecursiveGamePath {
                source_path: normalized_relative_path(root, &path)?,
                path,
            });
        } else if file_type.is_file() {
            tracing::warn!(path = %path.display(), "non-game file skipped during recursive discovery");
            discovery.skipped_files += 1;
        } else {
            tracing::warn!(path = %path.display(), "non-regular entry skipped during recursive discovery");
            discovery.unreadable_entries += 1;
        }
    }

    Ok(())
}

type PositionJsonlReader = PositionRecordJsonlReader<BufReader<File>>;

fn open_position_jsonl(path: &Path) -> Result<PositionJsonlReader> {
    let file = File::open(path).with_context(|| format!("cannot open {path:?}"))?;
    Ok(PositionRecordJsonlReader::new(BufReader::new(file)))
}

/// Apply the CLI's permissive JSONL policy: malformed or unsupported records are counted,
/// diagnosed, and skipped, while stream I/O errors (including future non-parse reader errors)
/// remain fatal. Commands that need to distinguish malformed JSON from typed-shape errors, such
/// as `validate`, intentionally inspect the raw line instead of using this helper.
fn permissive_position_record(
    item: std::result::Result<PositionRecordLine, PositionRecordLineError>,
    path: &Path,
    broken: &mut usize,
) -> Result<Option<PositionRecord>> {
    permissive_position_record_with_diagnostics(item, path, broken, true)
}

/// The same permissive policy with optional parse diagnostics. Two-pass commands suppress the
/// first pass so one malformed physical line produces one warning, while still applying the
/// schema gate identically in both passes.
fn permissive_position_record_with_diagnostics(
    item: std::result::Result<PositionRecordLine, PositionRecordLineError>,
    path: &Path,
    broken: &mut usize,
    diagnose: bool,
) -> Result<Option<PositionRecord>> {
    match item {
        Ok(line) => Ok(Some(line.record)),
        Err(PositionRecordLineError::Parse {
            line_number,
            source,
        }) => {
            if diagnose {
                tracing::warn!(line = line_number, "parse error: {source}");
            }
            *broken += 1;
            Ok(None)
        }
        Err(error) => Err(error).with_context(|| format!("cannot read {path:?}")),
    }
}

fn load_records(path: &Path) -> Result<(Vec<PositionRecord>, usize)> {
    let mut broken = 0usize;
    let mut records = Vec::new();
    for item in open_position_jsonl(path)? {
        if let Some(record) = permissive_position_record(item, path, &mut broken)? {
            records.push(record);
        }
    }
    Ok((records, broken))
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

    // seeded_hash_is_a_stable_golden_value moved to stratifykit-core (seeded_hash now lives
    // there).

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
        let path = label_cache_path(
            &cache,
            "startpos",
            "engine",
            Some("1.0"),
            SearchLimit::Depth(8),
        );
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
            SearchLimit::Depth(8),
        );
        let path_b = label_cache_path(
            &cache_with_fingerprint(Some(222)),
            "startpos",
            "engine",
            Some("1.0"),
            SearchLimit::Depth(8),
        );
        assert_ne!(
            path_a, path_b,
            "a rebuilt engine binary must not collide with a stale cache entry"
        );
    }
}

#[cfg(test)]
mod atomic_writer_tests {
    use super::*;

    #[test]
    fn drop_without_commit_preserves_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.bin");
        fs::write(&path, b"previous").unwrap();

        {
            let mut writer = AtomicWriter::new(&path).unwrap();
            writer.write_all(b"partial replacement").unwrap();
        }

        assert_eq!(fs::read(&path).unwrap(), b"previous");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn commit_replaces_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.bin");
        fs::write(&path, b"previous").unwrap();

        let mut writer = AtomicWriter::new(&path).unwrap();
        writer.write_all(b"complete replacement").unwrap();
        writer.commit().unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"complete replacement");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn commit_follows_existing_output_symlink() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.bin");
        let link = dir.path().join("output.bin");
        fs::write(&target, b"previous").unwrap();
        symlink(&target, &link).unwrap();

        let mut writer = AtomicWriter::new(&link).unwrap();
        writer.write_all(b"replacement").unwrap();
        writer.commit().unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"replacement");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn split_bundle_rolls_back_earlier_replacements() {
        let dir = tempfile::tempdir().unwrap();
        let staged_a = dir.path().join("staged-a");
        let staged_b = dir.path().join("staged-b");
        let destination_a = dir.path().join("destination-a");
        let destination_b = dir.path().join("destination-b");
        fs::write(&staged_a, b"new-a").unwrap();
        fs::write(&staged_b, b"new-b").unwrap();
        fs::write(&destination_a, b"old-a").unwrap();
        fs::write(&destination_b, b"old-b").unwrap();

        let error = commit_split_bundle_with(
            &[
                (staged_a, destination_a.clone()),
                (staged_b, destination_b.clone()),
            ],
            |index| {
                if index == 1 {
                    anyhow::bail!("injected failure");
                }
                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("injected failure"));
        assert_eq!(fs::read(destination_a).unwrap(), b"old-a");
        assert_eq!(fs::read(destination_b).unwrap(), b"old-b");
    }

    #[test]
    fn failed_split_bundle_rollback_retains_previous_output_backup() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("destination");
        fs::create_dir(&destination).unwrap();
        let mut backup = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        backup.write_all(b"previous output").unwrap();
        backup.as_file().sync_all().unwrap();
        let backup_path = backup.path().to_path_buf();

        let mut entries = [SplitBundleEntry {
            staged: dir.path().join("staged"),
            destination,
            prepared_temp: None,
            backup: Some(backup.into_temp_path()),
            committed: true,
        }];
        let errors = rollback_split_bundle(&mut entries);

        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("previous output retained"));
        assert_eq!(fs::read(backup_path).unwrap(), b"previous output");
    }
}
