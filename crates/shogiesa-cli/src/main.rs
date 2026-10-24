use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet};
use std::fmt::{Display, Write as _}; // writeln! into a String, for cmd_tune's Markdown report
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, IsTerminal, Read, Seek, SeekFrom, Write};
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
    let stderr_is_terminal = std::io::stderr().is_terminal();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .with_ansi(stderr_is_terminal)
        .init();

    let cli = Cli::parse();
    validate_command_paths(&cli.command)?;
    match cli.command {
        Commands::Extract(args) => commands::cmd_extract(args),
        Commands::Label(args) => commands::cmd_label(*args),
        Commands::Stability(args) => commands::cmd_stability(args),
        Commands::Split(args) => commands::cmd_split(args),
        Commands::Sample(args) => commands::cmd_sample(args),
        Commands::Mine(args) => commands::cmd_mine(args),
        Commands::Balance(args) => commands::cmd_balance(args),
        Commands::Stratify(args) => commands::cmd_stratify(args),
        Commands::Select(args) => commands::cmd_select(args),
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
        Commands::MakeGateOpenings(args) => commands::cmd_make_gate_openings(args),
        Commands::Shuffle(args) => commands::cmd_shuffle(args),
        Commands::LinePrior(args) => match args.action {
            LinePriorAction::Export(a) => commands::cmd_lineprior_export(a),
        },
    }
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
        .entry(commands::split_root_key(&record.source).to_string())
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
