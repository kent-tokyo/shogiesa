use std::collections::{BTreeMap, HashMap, HashSet};
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
    /// Validate data integrity of a positions dataset
    Validate(ReportArgs),
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
        Commands::Validate(args) => cmd_validate(args),
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

/// Extract the side-to-move character ('b' or 'w') from a SFEN string.
fn sfen_side(sfen: &str) -> Option<&str> {
    sfen.split_ascii_whitespace().nth(1)
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
    let mut phases = BTreeMap::<&str, usize>::new();
    let mut sides = BTreeMap::<&str, usize>::new();
    let mut schema_versions = BTreeMap::<u32, usize>::new();
    let mut ply_sum = 0u64;
    let mut ply_min = u32::MAX;
    let mut ply_max = 0u32;
    let mut sfen_counts: HashMap<&str, usize> = HashMap::new();
    let mut tag_mismatches = 0usize;

    for rec in &records {
        *phases.entry(rec.tags.phase.as_str()).or_default() += 1;
        *sides.entry(rec.tags.side_to_move.as_str()).or_default() += 1;
        *schema_versions.entry(rec.schema_version).or_default() += 1;
        let ply = rec.source.ply;
        ply_sum += ply as u64;
        ply_min = ply_min.min(ply);
        ply_max = ply_max.max(ply);
        *sfen_counts.entry(rec.sfen.as_str()).or_default() += 1;
        // Check side_to_move tag vs SFEN
        if let Some(sf_side) = sfen_side(&rec.sfen) {
            let expected = match sf_side {
                "b" => "black",
                "w" => "white",
                _ => "",
            };
            if !expected.is_empty() && rec.tags.side_to_move != expected {
                tag_mismatches += 1;
            }
        }
    }

    let duplicate_sfens: usize = sfen_counts
        .values()
        .filter(|&&c| c > 1)
        .map(|&c| c - 1)
        .sum();
    let mut sources = BTreeMap::<&str, usize>::new();
    for rec in &records {
        *sources.entry(rec.source.path.as_str()).or_default() += 1;
    }

    println!("=== shogiesa report ===");
    println!("positions      : {n}");
    println!("broken lines   : {broken}");
    println!(
        "ply range      : {ply_min}–{ply_max} (avg {:.1})",
        ply_sum as f64 / n as f64
    );
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
    println!("source files: {}", sources.len());
    for (path, count) in sources.iter().take(10) {
        println!("  {path}: {count}");
    }
    if sources.len() > 10 {
        println!("  … and {} more", sources.len() - 10);
    }

    Ok(())
}

fn cmd_validate(args: ReportArgs) -> Result<()> {
    let content =
        fs::read_to_string(&args.input).with_context(|| format!("cannot read {:?}", args.input))?;

    let total_lines = content.lines().filter(|l| !l.trim().is_empty()).count();
    let mut valid_json = 0usize;
    let mut valid_records = 0usize;
    let mut tag_mismatches = 0usize;
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

        if let Some(sf_side) = sfen_side(&rec.sfen) {
            let expected = match sf_side {
                "b" => "black",
                "w" => "white",
                _ => "",
            };
            if !expected.is_empty() && rec.tags.side_to_move != expected {
                tag_mismatches += 1;
            }
        }
    }

    let broken = total_lines - valid_json;

    println!("=== shogiesa validate ===");
    println!("total lines    : {total_lines}");
    println!("valid JSON     : {valid_json}");
    println!("valid records  : {valid_records}");
    println!("broken lines   : {broken}");
    println!("duplicate SFENs: {duplicate_sfens}");
    println!("tag mismatches : {tag_mismatches}  (side_to_move vs SFEN)");
    println!("schema versions: {schema_versions:?}");

    if tag_mismatches > 0 || broken > 0 {
        println!();
        if tag_mismatches > 0 {
            println!("WARN: {tag_mismatches} side_to_move tag mismatches found");
        }
        if broken > 0 {
            println!("WARN: {broken} broken lines found");
        }
        std::process::exit(1);
    } else {
        println!();
        println!("OK");
    }
    Ok(())
}
