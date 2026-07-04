use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use std::hash::{Hash, Hasher};

use shogiesa_core::{
    GamePhase, Observation, PositionRecord, QualityConfig, SCHEMA_VERSION, Score, SideToMove,
    engine_bestmove_agreement, evaluate_quality, score_swing, sfen::Sfen, zobrist_from_sfen,
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
    /// Require all observations to agree on bestmove
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
    /// Minimum cp score — positions with lower eval are excluded (e.g. --eval-min=-1200)
    #[arg(long, allow_hyphen_values = true)]
    eval_min: Option<i32>,
    /// Maximum cp score — positions with higher eval are excluded (e.g. --eval-max=1200)
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
        Commands::Pack(args) => cmd_pack(args),
        Commands::Unpack(args) => cmd_unpack(args),
        Commands::Filter(args) => cmd_filter(args),
        Commands::Report(args) => cmd_report(args),
        Commands::Validate(args) => cmd_validate(args),
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
                    if use_zobrist {
                        let hash = zobrist_from_sfen(&rec.sfen).unwrap_or(0);
                        if !seen_hashes.insert(hash) {
                            continue;
                        }
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

fn analyze_record(
    rec: &mut PositionRecord,
    engine: &mut UsiEngine,
    depths: &[u32],
    timeout_ms: u64,
    existing_policy: ExistingPolicy,
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
        match engine.analyse(&rec.sfen, depth, timeout_ms) {
            Ok(result) => {
                // Dedupe on the achieved depth, not the requested one, for the same reason —
                // if Skip couldn't skip (under-reach) and re-achieves the same depth, this
                // replaces the stale entry instead of duplicating it.
                if !matches!(existing_policy, ExistingPolicy::Append) {
                    rec.observations
                        .retain(|o| !(o.engine == engine.engine_name && o.depth == result.depth));
                }
                rec.observations.push(Observation {
                    engine: engine.engine_name.clone(),
                    engine_version: engine.engine_version.clone(),
                    depth: result.depth,
                    score: result.score,
                    score_bound: result.score_bound,
                    bestmove: result.bestmove,
                    nodes: result.nodes,
                    time_ms: result.time_ms,
                    pv: result.pv,
                    policy_margin_cp: result.policy_margin_cp,
                    candidates: result.candidates,
                });
            }
            Err(e) => tracing::warn!(depth, "analysis error: {e}"),
        }
    }
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

    let engine_path = args.engine;
    let engine_name = args.engine_name.unwrap_or_default();
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

    // Parse and validate all input records (streaming for large files)
    let content =
        fs::read_to_string(&args.input).with_context(|| format!("cannot open {:?}", args.input))?;
    let mut records: Vec<PositionRecord> = Vec::new();
    let mut skipped = 0usize;
    for (i, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<PositionRecord>(line) {
            Ok(rec) if Sfen::parse(&rec.sfen).is_ok() => records.push(rec),
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

    let total = records.len();
    info!(total, jobs, "labeling started");

    // Verify the engine launches before committing to parallel work
    let probe = UsiEngine::launch(
        &engine_path,
        engine_name.clone(),
        timeout_ms,
        &engine_options,
    )
    .with_context(|| format!("failed to launch engine {engine_path:?}"))?;
    let engine_display_name = probe.engine_name.clone();
    drop(probe); // cleanly quits via Drop

    // Parallel analysis: each rayon thread owns one UsiEngine via thread_local
    std::thread_local! {
        static ENGINE: std::cell::RefCell<Option<UsiEngine>> = const { std::cell::RefCell::new(None) };
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .context("failed to build thread pool")?;

    let done = AtomicUsize::new(0);
    let engine_launch_failures = AtomicUsize::new(0);
    let print_every = (total / 100).max(1);
    let labeled_records: Vec<PositionRecord> = pool.install(|| {
        records
            .into_par_iter()
            .map(|mut rec| {
                ENGINE.with(|cell| {
                    let mut opt = cell.borrow_mut();
                    if opt.is_none()
                        && let Ok(e) = UsiEngine::launch(
                            &engine_path,
                            engine_name.clone(),
                            timeout_ms,
                            &engine_options,
                        )
                    {
                        *opt = Some(e);
                    }
                    if let Some(engine) = opt.as_mut() {
                        analyze_record(&mut rec, engine, &depths, timeout_ms, existing_policy);
                    } else {
                        engine_launch_failures.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(sfen = %rec.sfen, "engine unavailable, position left unlabeled");
                    }
                });
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n.is_multiple_of(print_every) || n == total {
                    eprint!("\r  {n}/{total}");
                }
                rec
            })
            .collect()
    });
    eprintln!();

    let labeled = labeled_records.len();
    let engine_launch_failures = engine_launch_failures.load(Ordering::Relaxed);
    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);
    for rec in &labeled_records {
        serde_json::to_writer(&mut writer, rec)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()?;

    eprintln!(
        "done [{engine_display_name}, jobs={jobs}]: {total} in, {labeled} labeled, {skipped} skipped, {engine_launch_failures} engine launch failures → {:?}",
        args.out
    );
    if let Some(manifest_path) = &args.manifest {
        let mut manifest = RunManifest::new("label", &args.input);
        manifest.input_hash = hash_file(&args.input)?;
        manifest.records_read = total;
        manifest.records_kept = labeled;
        manifest.records_dropped = skipped;
        accumulate_coverage(&mut manifest, &labeled_records);
        manifest.engine_name = Some(engine_display_name);
        manifest.depths = Some(depths);
        manifest.multipv = (args.multipv > 1).then_some(args.multipv);
        manifest.engine_options = Some(args.engine_options.clone());
        manifest.jobs = Some(jobs);
        manifest.engine_launch_failures = Some(engine_launch_failures);
        write_manifest(manifest_path, &manifest)?;
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

    // Group records by source path, writing into per-source output files
    let mut writers: HashMap<String, BufWriter<File>> = HashMap::new();
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
        if !writers.contains_key(&key) {
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
            let file_name = format!("{safe}.jsonl");
            let out_path = out_dir.join(&file_name);
            let f =
                File::create(&out_path).with_context(|| format!("cannot create {out_path:?}"))?;
            writers.insert(key.clone(), BufWriter::new(f));
            file_names.insert(key.clone(), file_name);
        }
        let w = writers.get_mut(&key).unwrap();
        serde_json::to_writer(&mut *w, &rec)?;
        w.write_all(b"\n")?;
        *file_counts.entry(file_names[&key].clone()).or_default() += 1;
        total += 1;
    }

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
        writers.len(),
        out_dir
    );
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

fn assign_split_bucket(
    seed: u64,
    source_path: &str,
    valid_frac: f64,
    test_frac: f64,
) -> SplitBucket {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut h);
    split_root_path(source_path).hash(&mut h);
    let unit = h.finish() as f64 / u64::MAX as f64;
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

        let bucket =
            assign_split_bucket(args.seed, &rec.source.path, args.valid_frac, args.test_frac);
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

/// Opt-in run provenance, written when `--manifest PATH` is given. `input_hash` is a plain
/// `DefaultHasher` digest (same mechanism already used by `assign_split_bucket`/`cmd_sample`),
/// not a cryptographic SHA-256 — this is a "did the input change between runs" marker, not an
/// integrity check against untrusted input, so it isn't worth a new dependency.
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
            records_read: 0,
            records_kept: 0,
            records_dropped: 0,
            drop_reasons: BTreeMap::new(),
            labeled_records: 0,
            unlabeled_records: 0,
            observations_with_candidates: 0,
            observations_total: 0,
            score_bound_distribution: BTreeMap::new(),
            filter_config: None,
            engine_name: None,
            depths: None,
            multipv: None,
            engine_options: None,
            jobs: None,
            engine_launch_failures: None,
        }
    }
}

fn write_manifest(path: &Path, manifest: &RunManifest) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(manifest)?)
        .with_context(|| format!("cannot write {path:?}"))
}

/// Hash a whole file's bytes with `DefaultHasher`. Only called when `--manifest` is given, on
/// commands that already fully materialize their input (`sample`/`balance`) — an extra read is
/// acceptable there since they aren't streaming to begin with.
/// Hashes line-by-line (content + `\n` per line) to match `cmd_pack`/`cmd_filter`'s inline
/// streaming hash exactly — so the same input file gets the same `input_hash` in a manifest
/// regardless of which command produced it.
fn hash_file(path: &Path) -> Result<String> {
    let reader = BufReader::new(File::open(path).with_context(|| format!("cannot open {path:?}"))?);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for line in reader.lines() {
        h.write(line?.as_bytes());
        h.write(b"\n");
    }
    Ok(format!("{:016x}", h.finish()))
}

/// Tally labeled/unlabeled records, MultiPV candidate coverage, and score-bound distribution
/// across a batch of records — the same descriptive stats `report` computes ad hoc, shared here
/// for `RunManifest`. Not a quality *decision* (no pass/fail judgment), so it lives in the CLI
/// rather than `shogiesa_core::evaluate_quality`.
/// Tally MultiPV-candidate coverage and score-bound distribution across a batch of
/// observations. Shared by `accumulate_coverage` (manifests) and `cmd_report` (stdout) so the
/// `match c.score_bound { ... }` logic isn't duplicated.
fn candidate_coverage_stats(
    records: &[PositionRecord],
) -> (usize, usize, BTreeMap<&'static str, usize>) {
    let mut with_candidates = 0;
    let mut total = 0;
    let mut score_bound_distribution: BTreeMap<&'static str, usize> = BTreeMap::new();
    for rec in records {
        for obs in &rec.observations {
            total += 1;
            if !obs.candidates.is_empty() {
                with_candidates += 1;
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
    (with_candidates, total, score_bound_distribution)
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
}

fn cmd_sample(args: SampleArgs) -> Result<()> {
    let (records, _) = load_records(&args.input)?;
    let total = records.len();
    let count = args.count.min(total);

    // Sort indices by hash(seed, sfen) — deterministic, spread across the dataset
    let seed = args.seed;
    let mut indices: Vec<usize> = (0..total).collect();
    indices.sort_by_key(|&i| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        seed.hash(&mut h);
        records[i].sfen.hash(&mut h);
        h.finish()
    });
    indices.truncate(count);

    // Output in original order
    let selected: HashSet<usize> = indices.into_iter().collect();
    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);
    let mut kept = 0usize;
    let mut kept_records = Vec::new();
    for (i, rec) in records.iter().enumerate() {
        if selected.contains(&i) {
            serde_json::to_writer(&mut writer, rec)?;
            writer.write_all(b"\n")?;
            kept += 1;
            if args.manifest.is_some() {
                kept_records.push(rec.clone());
            }
        }
    }
    writer.flush()?;
    eprintln!(
        "done: {kept}/{total} sampled (seed={seed}) → {:?}",
        args.out
    );
    if let Some(manifest_path) = &args.manifest {
        let mut manifest = RunManifest::new("sample", &args.input);
        manifest.input_hash = hash_file(&args.input)?;
        manifest.records_read = total;
        manifest.records_kept = kept;
        manifest.records_dropped = total - kept;
        accumulate_coverage(&mut manifest, &kept_records);
        write_manifest(manifest_path, &manifest)?;
    }
    Ok(())
}

fn eval_black(rec: &PositionRecord) -> Option<i32> {
    rec.observations
        .iter()
        .max_by_key(|o| o.depth)
        .and_then(|o| match o.score {
            Score::Cp { value } => Some(match rec.tags.side_to_move {
                SideToMove::Black => value,
                SideToMove::White => -value,
            }),
            Score::Mate { .. } => None,
        })
}

fn cmd_mine(args: MineArgs) -> Result<()> {
    let (records, _) = load_records(&args.input)?;
    let total = records.len();

    // Group indices by source game path, then sort each group by ply
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

        // Blunder detection: large eval swing between consecutive labeled positions
        for j in 1..indices.len() {
            if let (Some(e0), Some(e1)) = (evals[j - 1], evals[j])
                && (e1 - e0).abs() >= args.blunder_threshold
            {
                let lo = j.saturating_sub(args.blunder_window);
                let hi = (j + args.blunder_window + 1).min(indices.len());
                for k in lo..hi {
                    if !records[indices[k]].observations.is_empty() {
                        keep.insert(indices[k]);
                    }
                }
            }
        }

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

fn cmd_balance(args: BalanceArgs) -> Result<()> {
    if args.by.is_empty() {
        anyhow::bail!("--by requires at least one of: phase, side, eval-bucket");
    }
    let by_phase = args.by.iter().any(|s| s == "phase");
    let by_side = args.by.iter().any(|s| s == "side");
    let by_eval = args.by.iter().any(|s| s == "eval-bucket");

    let (records, _) = load_records(&args.input)?;
    let total = records.len();

    // Build composite bucket key for each record
    let mut buckets: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, rec) in records.iter().enumerate() {
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
                    Score::Cp { value } => format!("{}:", (value.div_euclid(200)) * 200),
                    Score::Mate { .. } => "mate:".to_string(),
                })
                .unwrap_or_else(|| "_none_:".to_string());
            key.push_str(&bucket_str);
        }
        buckets.entry(key).or_default().push(i);
    }

    let min_size = buckets.values().map(|v| v.len()).min().unwrap_or(0);
    let target = args.target.unwrap_or(min_size);

    // Select `target` entries from each bucket sorted by SFEN (deterministic)
    let mut keep = HashSet::<usize>::new();
    for indices in buckets.values() {
        let mut sorted = indices.clone();
        sorted.sort_by(|&a, &b| records[a].sfen.cmp(&records[b].sfen));
        for &idx in sorted.iter().take(target) {
            keep.insert(idx);
        }
    }

    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);
    let mut kept = 0usize;
    let mut kept_records = Vec::new();
    for (i, rec) in records.iter().enumerate() {
        if keep.contains(&i) {
            serde_json::to_writer(&mut writer, rec)?;
            writer.write_all(b"\n")?;
            kept += 1;
            if args.manifest.is_some() {
                kept_records.push(rec.clone());
            }
        }
    }
    writer.flush()?;
    eprintln!(
        "done: {kept}/{total} selected (target {target}/bucket, {} buckets) → {:?}",
        buckets.len(),
        args.out
    );
    if let Some(manifest_path) = &args.manifest {
        let mut manifest = RunManifest::new("balance", &args.input);
        manifest.input_hash = hash_file(&args.input)?;
        manifest.records_read = total;
        manifest.records_kept = kept;
        manifest.records_dropped = total - kept;
        accumulate_coverage(&mut manifest, &kept_records);
        write_manifest(manifest_path, &manifest)?;
    }
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
    let mut input_hasher = std::collections::hash_map::DefaultHasher::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if manifest.is_some() {
            input_hasher.write(line.as_bytes());
            input_hasher.write(b"\n");
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
        m.input_hash = format!("{:016x}", input_hasher.finish());
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
    let mut input_hasher = std::collections::hash_map::DefaultHasher::new();

    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if manifest.is_some() {
            input_hasher.write(line.as_bytes());
            input_hasher.write(b"\n");
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
        m.input_hash = format!("{:016x}", input_hasher.finish());
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
    let non_empty: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    let broken = non_empty
        .iter()
        .filter(|l| serde_json::from_str::<PositionRecord>(l).is_err())
        .count();
    let records: Vec<PositionRecord> = non_empty
        .iter()
        .enumerate()
        .filter_map(|(i, line)| {
            serde_json::from_str::<PositionRecord>(line)
                .map_err(|e| tracing::warn!(line = i + 1, "parse error: {e}"))
                .ok()
        })
        .collect();
    Ok((records, broken))
}

fn cmd_report(args: ReportArgs) -> Result<()> {
    let (records, broken) = load_records(&args.input)?;

    if records.is_empty() {
        println!("no valid records in {:?}", args.input);
        return Ok(());
    }

    let n = records.len();
    let mut phases = BTreeMap::<String, usize>::new();
    let mut sides = BTreeMap::<String, usize>::new();
    let mut schema_versions = BTreeMap::<u32, usize>::new();
    let mut ply_sum = 0u64;
    let mut ply_min = u32::MAX;
    let mut ply_max = 0u32;
    let mut sfen_counts: HashMap<&str, usize> = HashMap::new();
    let mut tag_mismatches = 0usize;
    let mut invalid_sfens = 0usize;
    let mut labeled = 0usize;
    let mut in_check = 0usize;
    let mut has_capture = 0usize;
    let mut depth_disagree = 0usize;
    let mut multi_engine = 0usize;
    let mut engine_disagree = 0usize;
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

    for rec in &records {
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
        *sfen_counts.entry(rec.sfen.as_str()).or_default() += 1;

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
            // depth disagreement
            let first = &rec.observations[0].bestmove;
            if rec.observations.iter().any(|o| &o.bestmove != first) {
                depth_disagree += 1;
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
            // eval bucket from deepest observation
            if let Some(deepest) = rec.observations.iter().max_by_key(|o| o.depth) {
                let key = match deepest.score {
                    Score::Cp { value } => {
                        // bucket width 200cp; clamp display at ±1400
                        (value.div_euclid(200)) * 200
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
        }
    }

    let duplicate_sfens: usize = sfen_counts
        .values()
        .filter(|&&c| c > 1)
        .map(|&c| c - 1)
        .sum();
    let duplicate_rate = duplicate_sfens as f64 / n as f64 * 100.0;

    let mut sources = BTreeMap::<&str, usize>::new();
    for rec in &records {
        *sources.entry(rec.source.path.as_str()).or_default() += 1;
    }
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
        let (obs_with_candidates, obs_total, score_bound_distribution) =
            candidate_coverage_stats(&records);
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
