use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use shogiesa_core::{
    GamePhase, Observation, PositionRecord, QualityConfig, SCHEMA_VERSION, Score, ScorePerspective,
    SideToMove, SourceInfo, bestmove_agreement, cp_from_black_perspective,
    engine_bestmove_agreement, evaluate_quality, has_special_bestmove,
    requested_depth_underreached, score_swing, sfen::Sfen, zobrist_from_sfen,
};
use shogiesa_pack as pack;
use shogiesa_usi::UsiEngine;
use tracing::info;

#[derive(Parser)]
#[command(
    name = "shogiesa",
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
    /// Report statistics about a positions dataset
    Report(ReportArgs),
    /// Validate data integrity of a positions dataset
    Validate(ValidateArgs),
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
    /// Write results as they arrive instead of preserving input order. Drops the reorder
    /// buffer entirely, so a slow position never delays already-finished ones behind it.
    #[arg(long)]
    unordered_output: bool,
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
        Commands::Report(args) => cmd_report(args),
        Commands::Validate(args) => cmd_validate(args),
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
    hits: Arc<AtomicUsize>,
    misses: Arc<AtomicUsize>,
}

/// How (if at all) the engine binary itself contributes to the label cache key, on top of its
/// USI-reported `id name`/`id version`. Those strings are controlled by the engine and aren't
/// guaranteed to change after a local rebuild, so relying on them alone risks a cache hit
/// silently reusing labels produced by a different executable.
#[derive(Clone, Copy, PartialEq, Eq)]
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
                .and_then(|s| serde_json::from_str(&s).ok());
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
                        if let Ok(json) = serde_json::to_string(&obs) {
                            let _ = write_cache_entry_atomically(path, &json);
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

    // Writer: runs on this thread. Default mode buffers out-of-order arrivals in `pending`
    // (bounded to `queue_depth` entries by the permit scheme above) and only flushes the next
    // contiguous job_id, so output order matches input order regardless of which worker finishes
    // first. `--unordered-output` skips this and writes on arrival, for throughput when input
    // order doesn't matter.
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

    let mut write_one =
        |record: &PositionRecord, manifest: &mut Option<RunManifest>| -> Result<()> {
            serde_json::to_writer(&mut out_writer, record)?;
            out_writer.write_all(b"\n")?;
            if let Some(m) = manifest.as_mut() {
                accumulate_coverage(m, std::slice::from_ref(record));
            }
            // Release the permit only now that the record is durably written -- this is the other
            // half of the dispatch window: it caps how far ahead of the writer the pipeline can get.
            let _ = writer_permit_tx.send(());
            Ok(())
        };

    for job in result_rx {
        if args.unordered_output {
            write_one(&job.record, &mut manifest)?;
            written += 1;
        } else {
            for record in reorder_push(&mut pending, &mut next_id, job.id, job.record) {
                write_one(&record, &mut manifest)?;
                written += 1;
            }
            // Guards the permit scheme's whole reason for existing: without it, `pending` could
            // in principle grow past `queue_depth` and this bounded pipeline would silently
            // revert to the unbounded memory use it replaced.
            debug_assert!(pending.len() <= queue_depth);
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

    let cache_suffix = cache_counts
        .map(|(hits, misses)| format!(", {hits} cache hits, {misses} cache misses"))
        .unwrap_or_default();
    eprintln!(
        "done [{engine_display_name}, jobs={jobs}]: {total} in, {written} labeled, {skipped} skipped, {engine_launch_failures} engine launch failures{cache_suffix} → {:?}",
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
        if let Some((hits, misses)) = cache_counts {
            manifest.cache_hits = Some(hits);
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
                let key = match c.score_bound {
                    shogiesa_core::ScoreBound::Exact => "exact",
                    shogiesa_core::ScoreBound::Lowerbound => "lowerbound",
                    shogiesa_core::ScoreBound::Upperbound => "upperbound",
                };
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

fn cmd_filter(args: FilterArgs) -> Result<()> {
    let allowed_phases: Option<Vec<GamePhase>> = args.phase.as_deref().map(|s| {
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
    });
    let config = build_quality_config(&args, allowed_phases);

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
                let bound_key = match obs.score_bound {
                    shogiesa_core::ScoreBound::Exact => "exact",
                    shogiesa_core::ScoreBound::Lowerbound => "lowerbound",
                    shogiesa_core::ScoreBound::Upperbound => "upperbound",
                };
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
