use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use shogiesa_core::PositionRecord;
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
    /// Report statistics about a positions dataset
    Report(ReportArgs),
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
    /// Deduplicate positions by SFEN
    #[arg(long)]
    dedup: bool,
}

#[derive(clap::Args)]
struct ReportArgs {
    /// Input JSONL file
    #[arg(short, long)]
    input: PathBuf,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Extract(args) => cmd_extract(args),
        Commands::Report(args) => cmd_report(args),
    }
}

fn cmd_extract(args: ExtractArgs) -> Result<()> {
    let config = shogiesa_csa::ExtractConfig {
        min_ply: args.min_ply,
        max_ply: args.max_ply,
        every_n: args.every_n_plies,
        dedup: args.dedup,
    };

    let paths = collect_csa_paths(&args.input)?;
    if paths.is_empty() {
        anyhow::bail!("no .csa files found in {:?}", args.input);
    }

    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut writer = BufWriter::new(out_file);

    let mut seen = HashSet::new();
    let mut total_games = 0usize;
    let mut total_positions = 0usize;
    let mut skipped = 0usize;

    for path in &paths {
        total_games += 1;
        match shogiesa_csa::extract_from_path(path, &config, &mut seen) {
            Ok(records) => {
                for rec in &records {
                    serde_json::to_writer(&mut writer, rec)?;
                    writer.write_all(b"\n")?;
                }
                total_positions += records.len();
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

fn collect_csa_paths(input: &PathBuf) -> Result<Vec<PathBuf>> {
    if input.is_file() {
        return Ok(vec![input.clone()]);
    }
    let mut paths = Vec::new();
    for entry in
        fs::read_dir(input).with_context(|| format!("cannot read directory {:?}", input))?
    {
        let entry = entry?;
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("csa") {
            paths.push(p);
        }
    }
    paths.sort();
    Ok(paths)
}

fn cmd_report(args: ReportArgs) -> Result<()> {
    let content =
        fs::read_to_string(&args.input).with_context(|| format!("cannot read {:?}", args.input))?;

    let records: Vec<PositionRecord> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .filter_map(|(i, line)| {
            serde_json::from_str::<PositionRecord>(line)
                .map_err(|e| tracing::warn!(line = i + 1, "parse error: {e}"))
                .ok()
        })
        .collect();

    if records.is_empty() {
        println!("no valid records in {:?}", args.input);
        return Ok(());
    }

    let n = records.len();

    // Phase distribution
    let mut phases = std::collections::BTreeMap::<&str, usize>::new();
    let mut sides = std::collections::BTreeMap::<&str, usize>::new();
    let mut ply_sum = 0u64;
    let mut ply_min = u32::MAX;
    let mut ply_max = 0u32;

    for rec in &records {
        *phases.entry(rec.tags.phase.as_str()).or_default() += 1;
        *sides.entry(rec.tags.side_to_move.as_str()).or_default() += 1;
        let ply = rec.source.ply;
        ply_sum += ply as u64;
        ply_min = ply_min.min(ply);
        ply_max = ply_max.max(ply);
    }

    // Source file counts
    let mut sources = std::collections::BTreeMap::<&str, usize>::new();
    for rec in &records {
        *sources.entry(rec.source.path.as_str()).or_default() += 1;
    }

    println!("=== shogiesa report ===");
    println!("positions : {n}");
    println!(
        "ply range : {ply_min}–{ply_max} (avg {:.1})",
        ply_sum as f64 / n as f64
    );
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
    println!("source files: {}", sources.len());
    for (path, count) in sources.iter().take(10) {
        println!("  {path}: {count}");
    }
    if sources.len() > 10 {
        println!("  … and {} more", sources.len() - 10);
    }

    Ok(())
}
