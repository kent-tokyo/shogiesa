use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use std::hash::{Hash, Hasher};

use shogiesa_core::{
    GamePhase, Observation, PositionRecord, SCHEMA_VERSION, Score, SideToMove, score_swing,
    sfen::Sfen, zobrist_from_sfen,
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
    /// Output JSONL file
    #[arg(short, long)]
    out: PathBuf,
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
    out_dir: PathBuf,
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
}

#[derive(clap::Args)]
struct PackArgs {
    /// Input positions JSONL
    #[arg(short, long)]
    input: PathBuf,
    /// Output binary pack file (.shgpk)
    #[arg(short, long)]
    out: PathBuf,
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
    /// Output JSONL file
    #[arg(short, long)]
    out: PathBuf,
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
) {
    for &depth in depths {
        match engine.analyse(&rec.sfen, depth, timeout_ms) {
            Ok(result) => {
                rec.observations.push(Observation {
                    engine: engine.engine_name.clone(),
                    engine_version: engine.engine_version.clone(),
                    depth: result.depth,
                    score: result.score,
                    bestmove: result.bestmove,
                    nodes: result.nodes,
                    time_ms: result.time_ms,
                    pv: result.pv,
                    policy_margin_cp: result.policy_margin_cp,
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

    rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build_global()
        .ok(); // ignore error if global pool already set

    let done = AtomicUsize::new(0);
    let engine_launch_failures = AtomicUsize::new(0);
    let print_every = (total / 100).max(1);
    let labeled_records: Vec<PositionRecord> = records
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
                    analyze_record(&mut rec, engine, &depths, timeout_ms);
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
        .collect();
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
    if !args.by_source {
        anyhow::bail!("--by-source is required (it's currently the only split mode)");
    }
    fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("cannot create {:?}", args.out_dir))?;

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
            let out_path = args.out_dir.join(&file_name);
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
    let manifest_path = args.out_dir.join("manifest.json");
    fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)
        .with_context(|| format!("cannot write {manifest_path:?}"))?;

    eprintln!(
        "done: {total} positions split into {} files → {:?}",
        writers.len(),
        args.out_dir
    );
    Ok(())
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
    for (i, rec) in records.iter().enumerate() {
        if selected.contains(&i) {
            serde_json::to_writer(&mut writer, rec)?;
            writer.write_all(b"\n")?;
            kept += 1;
        }
    }
    writer.flush()?;
    eprintln!(
        "done: {kept}/{total} sampled (seed={seed}) → {:?}",
        args.out
    );
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
    for (i, rec) in records.iter().enumerate() {
        if keep.contains(&i) {
            serde_json::to_writer(&mut writer, rec)?;
            writer.write_all(b"\n")?;
            kept += 1;
        }
    }
    writer.flush()?;
    eprintln!(
        "done: {kept}/{total} selected (target {target}/bucket, {} buckets) → {:?}",
        buckets.len(),
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
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<PositionRecord>(&line) {
            Ok(rec) => {
                pack::encode_record(&rec, &mut writer)?;
                total += 1;
            }
            Err(e) => {
                tracing::warn!(line = i + 1, "JSON parse error: {e}");
                skipped += 1;
            }
        }
    }
    writer.flush()?;
    eprintln!("done: {total} packed, {skipped} skipped → {:?}", args.out);
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

fn cp_value(s: &Score) -> Option<i32> {
    match s {
        Score::Cp { value } => Some(*value),
        Score::Mate { .. } => None,
    }
}

/// Returns the reason the record was dropped, or `None` if it passes every gate.
fn filter_reason(
    rec: &PositionRecord,
    args: &FilterArgs,
    allowed_phases: &Option<Vec<GamePhase>>,
) -> Option<&'static str> {
    let obs = &rec.observations;

    if (obs.len() as u32) < args.min_observations {
        return Some("min_observations");
    }

    if allowed_phases
        .as_ref()
        .is_some_and(|p| !p.contains(&rec.tags.phase))
    {
        return Some("phase");
    }

    if args.exclude_mate && obs.iter().any(|o| matches!(o.score, Score::Mate { .. })) {
        return Some("mate");
    }

    if args.exclude_in_check && rec.tags.in_check {
        return Some("in_check");
    }

    if args.exclude_capture && rec.tags.has_capture {
        return Some("capture");
    }

    let cp_scores: Vec<i32> = obs.iter().filter_map(|o| cp_value(&o.score)).collect();

    if args
        .eval_min
        .is_some_and(|min| cp_scores.iter().any(|&v| v < min))
    {
        return Some("eval_min");
    }
    if args
        .eval_max
        .is_some_and(|max| cp_scores.iter().any(|&v| v > max))
    {
        return Some("eval_max");
    }

    if let Some(max_swing) = args.max_score_swing_cp
        && score_swing(&cp_scores).is_some_and(|swing| swing > max_swing)
    {
        return Some("score_swing");
    }

    if args.min_policy_margin_cp.is_some_and(|min| {
        obs.iter()
            .any(|o| o.policy_margin_cp.is_some_and(|m| m < min))
    }) {
        return Some("policy_margin");
    }

    if args.require_bestmove_agreement && obs.len() >= 2 {
        let first = &obs[0].bestmove;
        if obs.iter().any(|o| &o.bestmove != first) {
            return Some("bestmove_disagreement");
        }
    }

    None
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

    let reader = BufReader::new(
        File::open(&args.input).with_context(|| format!("cannot open {:?}", args.input))?,
    );
    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);

    let mut total = 0usize;
    let mut passed = 0usize;
    let mut skipped = 0usize;
    let mut drop_reasons: BTreeMap<&'static str, usize> = BTreeMap::new();

    for (i, line) in reader.lines().enumerate() {
        let line = line?;
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

        match filter_reason(&rec, &args, &allowed_phases) {
            None => {
                serde_json::to_writer(&mut writer, &rec)?;
                writer.write_all(b"\n")?;
                passed += 1;
            }
            Some(reason) => {
                skipped += 1;
                *drop_reasons.entry(reason).or_default() += 1;
            }
        }
    }

    writer.flush()?;
    eprintln!(
        "done: {total} read, {passed} passed, {skipped} filtered → {:?}",
        args.out
    );
    if !drop_reasons.is_empty() {
        eprintln!("drop reasons:");
        for (reason, count) in &drop_reasons {
            eprintln!("  {reason:<24} {count}");
        }
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
    let mut depth_counts: BTreeMap<u32, usize> = BTreeMap::new();
    // eval buckets: key = floor(cp / 200) * 200; special keys: i32::MIN = unlabeled, i32::MAX = mate
    let mut eval_buckets: BTreeMap<i32, usize> = BTreeMap::new();

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
        } else {
            labeled += 1;
            // depth disagreement
            let first = &rec.observations[0].bestmove;
            if rec.observations.iter().any(|o| &o.bestmove != first) {
                depth_disagree += 1;
            }
            for obs in &rec.observations {
                *depth_counts.entry(obs.depth).or_default() += 1;
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
        println!("  depth counts:");
        for (&depth, &count) in &depth_counts {
            println!("    depth {depth:>2}     : {count:>6}");
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

    Ok(())
}

fn cmd_validate(args: ValidateArgs) -> Result<()> {
    let content =
        fs::read_to_string(&args.input).with_context(|| format!("cannot read {:?}", args.input))?;

    let total_lines = content.lines().filter(|l| !l.trim().is_empty()).count();
    let mut valid_json = 0usize;
    let mut valid_records = 0usize;
    let mut tag_mismatches = 0usize;
    let mut invalid_sfens = 0usize;
    let mut schema_versions = BTreeMap::<u32, usize>::new();
    let mut seen_sfens: HashSet<String> = HashSet::new();
    let mut duplicate_sfens = 0usize;

    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
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
