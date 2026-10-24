use super::super::*;

#[derive(Debug, Serialize)]
struct DatasetDiffFileSummary {
    path: String,
    input_hash: String,
    records: usize,
    broken_lines: usize,
    observations: usize,
}

#[derive(Debug, Default, Serialize)]
struct DatasetDiffSummary {
    matched_records: usize,
    unchanged_records: usize,
    changed_records: usize,
    added_records: usize,
    removed_records: usize,
}

#[derive(Debug, Default, Serialize)]
struct DatasetDiffDeltas {
    observations: i64,
    source_roots: BTreeMap<String, i64>,
    phases: BTreeMap<String, i64>,
    eval_buckets: BTreeMap<String, i64>,
}

#[derive(Debug, Serialize)]
struct DatasetDiffReport {
    report_version: u32,
    match_by: String,
    baseline: DatasetDiffFileSummary,
    candidate: DatasetDiffFileSummary,
    summary: DatasetDiffSummary,
    field_changes: BTreeMap<String, usize>,
    deltas: DatasetDiffDeltas,
}

fn dataset_diff_key(record: &PositionRecord, match_by: &str) -> String {
    match match_by {
        "position" => record.sfen.clone(),
        "occurrence" => format!(
            "{}\0{}\0{}",
            record.sfen, record.source.path, record.source.ply
        ),
        _ => unreachable!("clap restricts --match-by"),
    }
}

fn add_dataset_delta(map: &mut BTreeMap<String, i64>, key: String, amount: i64) {
    let remove = {
        let entry = map.entry(key.clone()).or_default();
        *entry += amount;
        *entry == 0
    };
    if remove {
        map.remove(&key);
    }
}

fn accumulate_dataset_deltas(
    records: &[PositionRecord],
    direction: i64,
    deltas: &mut DatasetDiffDeltas,
) {
    for record in records {
        deltas.observations += direction * record.observations.len() as i64;
        add_dataset_delta(
            &mut deltas.source_roots,
            split_root_key(&record.source).to_string(),
            direction,
        );
        add_dataset_delta(&mut deltas.phases, record.tags.phase.to_string(), direction);
        add_dataset_delta(
            &mut deltas.eval_buckets,
            eval_bucket_of(record).to_string(),
            direction,
        );
    }
}

fn count_changed_field<T: Serialize>(
    name: &str,
    baseline: &T,
    candidate: &T,
    changes: &mut BTreeMap<String, usize>,
) -> Result<()> {
    if serde_json::to_value(baseline)? != serde_json::to_value(candidate)? {
        *changes.entry(name.to_string()).or_default() += 1;
    }
    Ok(())
}

fn record_dataset_field_changes(
    baseline: &PositionRecord,
    candidate: &PositionRecord,
    changes: &mut BTreeMap<String, usize>,
) -> Result<()> {
    count_changed_field(
        "schema_version",
        &baseline.schema_version,
        &candidate.schema_version,
        changes,
    )?;
    count_changed_field("sfen", &baseline.sfen, &candidate.sfen, changes)?;
    count_changed_field("source", &baseline.source, &candidate.source, changes)?;
    count_changed_field("tags", &baseline.tags, &candidate.tags, changes)?;
    count_changed_field(
        "observations",
        &baseline.observations,
        &candidate.observations,
        changes,
    )?;
    count_changed_field(
        "stability",
        &baseline.stability,
        &candidate.stability,
        changes,
    )?;
    count_changed_field(
        "game_result",
        &baseline.game_result,
        &candidate.game_result,
        changes,
    )?;
    Ok(())
}

fn dataset_file_summary(
    path: &Path,
    records: &[PositionRecord],
    broken_lines: usize,
) -> Result<DatasetDiffFileSummary> {
    Ok(DatasetDiffFileSummary {
        path: path.display().to_string(),
        input_hash: hash_file(path)?,
        records: records.len(),
        broken_lines,
        observations: records.iter().map(|record| record.observations.len()).sum(),
    })
}

fn write_delta_section(out: &mut String, title: &str, values: &BTreeMap<String, i64>) {
    writeln!(out, "{title}:").unwrap();
    if values.is_empty() {
        writeln!(out, "  (none)").unwrap();
    } else {
        for (key, delta) in values {
            writeln!(out, "  {key}: {delta:+}").unwrap();
        }
    }
}

fn records_by_serialized_form(
    records: Vec<PositionRecord>,
) -> Result<BTreeMap<String, Vec<PositionRecord>>> {
    let mut grouped = BTreeMap::<String, Vec<PositionRecord>>::new();
    for record in records {
        grouped
            .entry(serde_json::to_string(&record)?)
            .or_default()
            .push(record);
    }
    Ok(grouped)
}

pub(crate) fn cmd_dataset_diff(args: DatasetDiffArgs) -> Result<()> {
    let (baseline_records, baseline_broken) = load_records(&args.baseline)?;
    let (candidate_records, candidate_broken) = load_records(&args.candidate)?;

    let baseline = dataset_file_summary(&args.baseline, &baseline_records, baseline_broken)?;
    let candidate = dataset_file_summary(&args.candidate, &candidate_records, candidate_broken)?;

    let mut deltas = DatasetDiffDeltas::default();
    accumulate_dataset_deltas(&baseline_records, -1, &mut deltas);
    accumulate_dataset_deltas(&candidate_records, 1, &mut deltas);

    let mut baseline_by_key = BTreeMap::<String, Vec<PositionRecord>>::new();
    for record in baseline_records {
        baseline_by_key
            .entry(dataset_diff_key(&record, &args.match_by))
            .or_default()
            .push(record);
    }
    let mut candidate_by_key = BTreeMap::<String, Vec<PositionRecord>>::new();
    for record in candidate_records {
        candidate_by_key
            .entry(dataset_diff_key(&record, &args.match_by))
            .or_default()
            .push(record);
    }

    let keys: BTreeSet<String> = baseline_by_key
        .keys()
        .chain(candidate_by_key.keys())
        .cloned()
        .collect();
    let mut summary = DatasetDiffSummary::default();
    let mut field_changes = BTreeMap::<String, usize>::new();

    for key in keys {
        let baseline_group = baseline_by_key.remove(&key).unwrap_or_default();
        let candidate_group = candidate_by_key.remove(&key).unwrap_or_default();
        let mut baseline_group = records_by_serialized_form(baseline_group)?;
        let mut candidate_group = records_by_serialized_form(candidate_group)?;
        let serialized_forms: BTreeSet<String> = baseline_group
            .keys()
            .chain(candidate_group.keys())
            .cloned()
            .collect();
        let mut unmatched_baseline = Vec::<(String, PositionRecord)>::new();
        let mut unmatched_candidate = Vec::<(String, PositionRecord)>::new();
        for serialized in serialized_forms {
            let mut baseline_records = baseline_group.remove(&serialized).unwrap_or_default();
            let mut candidate_records = candidate_group.remove(&serialized).unwrap_or_default();
            let unchanged = baseline_records.len().min(candidate_records.len());
            summary.unchanged_records += unchanged;
            baseline_records.drain(..unchanged);
            candidate_records.drain(..unchanged);
            unmatched_baseline.extend(
                baseline_records
                    .into_iter()
                    .map(|record| (serialized.clone(), record)),
            );
            unmatched_candidate.extend(
                candidate_records
                    .into_iter()
                    .map(|record| (serialized.clone(), record)),
            );
        }

        let changed = unmatched_baseline.len().min(unmatched_candidate.len());
        for ((_, baseline), (_, candidate)) in unmatched_baseline
            .iter()
            .take(changed)
            .zip(unmatched_candidate.iter().take(changed))
        {
            record_dataset_field_changes(baseline, candidate, &mut field_changes)?;
        }
        summary.changed_records += changed;
        summary.removed_records += unmatched_baseline.len() - changed;
        summary.added_records += unmatched_candidate.len() - changed;
    }
    summary.matched_records = summary.unchanged_records + summary.changed_records;

    let report = DatasetDiffReport {
        report_version: 1,
        match_by: args.match_by,
        baseline,
        candidate,
        summary,
        field_changes,
        deltas,
    };

    if let Some(path) = &args.json_out {
        atomic_write_bytes(
            path,
            (serde_json::to_string_pretty(&report)? + "\n").as_bytes(),
        )
        .with_context(|| format!("cannot write {path:?}"))?;
    }

    let mut out = String::new();
    writeln!(out, "dataset diff").unwrap();
    writeln!(out, "match by           : {}", report.match_by).unwrap();
    writeln!(out, "baseline           : {}", report.baseline.path).unwrap();
    writeln!(out, "candidate          : {}", report.candidate.path).unwrap();
    writeln!(out, "baseline records   : {}", report.baseline.records).unwrap();
    writeln!(out, "candidate records  : {}", report.candidate.records).unwrap();
    writeln!(out, "baseline broken    : {}", report.baseline.broken_lines).unwrap();
    writeln!(
        out,
        "candidate broken   : {}",
        report.candidate.broken_lines
    )
    .unwrap();
    writeln!(
        out,
        "matched records    : {}",
        report.summary.matched_records
    )
    .unwrap();
    writeln!(
        out,
        "unchanged records  : {}",
        report.summary.unchanged_records
    )
    .unwrap();
    writeln!(
        out,
        "changed records    : {}",
        report.summary.changed_records
    )
    .unwrap();
    writeln!(out, "added records      : {}", report.summary.added_records).unwrap();
    writeln!(
        out,
        "removed records    : {}",
        report.summary.removed_records
    )
    .unwrap();
    writeln!(out, "observation delta  : {:+}", report.deltas.observations).unwrap();
    writeln!(out, "field changes:").unwrap();
    if report.field_changes.is_empty() {
        writeln!(out, "  (none)").unwrap();
    } else {
        for (field, count) in &report.field_changes {
            writeln!(out, "  {field}: {count}").unwrap();
        }
    }
    write_delta_section(&mut out, "source-root delta", &report.deltas.source_roots);
    write_delta_section(&mut out, "phase delta", &report.deltas.phases);
    write_delta_section(&mut out, "eval-bucket delta", &report.deltas.eval_buckets);
    print!("{out}");
    Ok(())
}

pub(crate) fn cmd_report(args: ReportArgs) -> Result<()> {
    let reader = open_position_jsonl(&args.input)?;

    let mut n = 0usize;
    let mut broken = 0usize;
    let mut phases = BTreeMap::<String, usize>::new();
    let mut sides = BTreeMap::<String, usize>::new();
    let mut wdl_counts = BTreeMap::<&'static str, usize>::new();
    let mut result_source_counts = BTreeMap::<String, usize>::new();
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
    let mut swing_buckets: BTreeMap<i64, usize> = BTreeMap::new();
    let mut margin_sum = 0f64;
    let mut margin_count = 0usize;
    let mut obs_score_bound_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut sources = BTreeMap::<String, usize>::new();
    let mut obs_with_candidates = 0usize;
    let mut obs_total = 0usize;
    let mut score_bound_distribution: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut requested_depth_total = 0usize;
    let mut requested_depth_underreach = 0usize;

    for item in reader {
        let Some(rec) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
        };
        n += 1;

        *phases.entry(format!("{}", rec.tags.phase)).or_default() += 1;
        *sides
            .entry(format!("{}", rec.tags.side_to_move))
            .or_default() += 1;
        *schema_versions.entry(rec.schema_version).or_default() += 1;
        *wdl_counts
            .entry(shogiesa_stratify::feature_wdl(&rec))
            .or_default() += 1;
        *result_source_counts
            .entry(
                rec.game_result
                    .as_ref()
                    .map(|gr| gr.result_source.clone())
                    .unwrap_or_else(|| "no_game_result".to_string()),
            )
            .or_default() += 1;
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
            if let Some(swing) = score_swing_i64(&cp_scores) {
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
            if !rec.observations.is_empty() {
                let key = match eval_bucket_of(&rec) {
                    EvalBucket::Cp(value) => value,
                    EvalBucket::Mate => i32::MAX,
                    EvalBucket::Unlabeled => i32::MIN,
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
    println!("wdl distribution (mover-relative):");
    for (label, count) in &wdl_counts {
        println!(
            "  {label:<12} {count:>6}  ({:.1}%)",
            *count as f64 / n as f64 * 100.0
        );
    }
    println!();
    println!("game_result source distribution:");
    for (label, count) in &result_source_counts {
        println!(
            "  {label:<24} {count:>6}  ({:.1}%)",
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
                format!("  {:+5}..{:+5}", key, i64::from(key) + 199)
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

#[derive(Default)]
struct ConflictGroupStats {
    evaluated: usize,
    conflicts: usize,
}

/// Compare each record's decisive game result with the deepest CP observation from each
/// engine/weight pair. Draws and unknown results are deliberately excluded: neither supplies a
/// binary WDL target, and counting them as conflicts would turn missing evidence into a quality
/// failure. A zero/deadband CP is also excluded because it is not evidence of either sign.
pub(crate) fn cmd_conflict_report(args: ConflictReportArgs) -> Result<()> {
    if args.min_abs_cp < 0 {
        anyhow::bail!("--min-abs-cp must be non-negative");
    }

    let reader = open_position_jsonl(&args.input)?;
    let mut records_read = 0usize;
    let mut parse_errors = 0usize;
    let mut no_game_result = 0usize;
    let mut non_decisive_result = 0usize;
    let mut decisive_records = 0usize;
    let mut no_cp_or_mate = 0usize;
    let mut deadband = 0usize;
    let mut evaluated = 0usize;
    let mut conflicts = 0usize;
    let mut groups: BTreeMap<String, ConflictGroupStats> = BTreeMap::new();

    for item in reader {
        records_read += 1;
        let Some(rec) = permissive_position_record(item, &args.input, &mut parse_errors)? else {
            continue;
        };

        let Some(game_result) = rec.game_result.as_ref() else {
            no_game_result += 1;
            continue;
        };
        if !matches!(
            game_result.outcome,
            GameOutcome::BlackWins | GameOutcome::WhiteWins
        ) {
            non_decisive_result += 1;
            continue;
        }
        decisive_records += 1;

        let mut deepest_by_engine: BTreeMap<&str, &Observation> = BTreeMap::new();
        for observation in &rec.observations {
            deepest_by_engine
                .entry(observation.engine.as_str())
                .and_modify(|current| {
                    if observation.depth > current.depth {
                        *current = observation;
                    }
                })
                .or_insert(observation);
        }
        if deepest_by_engine.is_empty() {
            no_cp_or_mate += 1;
            continue;
        }

        let mut record_evaluated = false;
        for observation in deepest_by_engine.into_values() {
            let Some(cp) = (match observation.score {
                Score::Cp { value } => Some(cp_from_black_perspective_i64(
                    value,
                    observation.score_perspective,
                    rec.tags.side_to_move,
                )),
                Score::Mate { .. } => None,
            }) else {
                no_cp_or_mate += 1;
                continue;
            };
            if cp.abs() <= i64::from(args.min_abs_cp) {
                deadband += 1;
                continue;
            }

            let conflict = match game_result.outcome {
                GameOutcome::BlackWins => cp < 0,
                GameOutcome::WhiteWins => cp > 0,
                GameOutcome::Draw | GameOutcome::Unknown => false,
            };
            let weight = observation.weight_sha256.as_deref().unwrap_or("none");
            let group_key = format!("{} / weight={weight}", observation.engine);
            let stats = groups.entry(group_key).or_default();
            stats.evaluated += 1;
            stats.conflicts += conflict as usize;
            evaluated += 1;
            conflicts += conflict as usize;
            record_evaluated = true;
        }
        if !record_evaluated && rec.observations.is_empty() {
            no_cp_or_mate += 1;
        }
    }

    println!("=== shogiesa conflict-report ===");
    println!("input             : {:?}", args.input);
    println!("records read      : {records_read}");
    println!("parse errors      : {parse_errors}");
    println!("no game_result    : {no_game_result}");
    println!("non-decisive result: {non_decisive_result}");
    println!("decisive records  : {decisive_records}");
    println!("evaluated pairs   : {evaluated}");
    println!(
        "conflicts         : {conflicts}  ({:.1}%)",
        conflicts as f64 / evaluated.max(1) as f64 * 100.0
    );
    println!("excluded no CP/mate: {no_cp_or_mate}");
    println!(
        "excluded deadband : {deadband}  (|cp| <= {})",
        args.min_abs_cp
    );
    println!();
    println!("by engine / weight:");
    for (group, stats) in groups {
        println!(
            "  {group:<40} evaluated={:>6} conflicts={:>6} ({:.1}%)",
            stats.evaluated,
            stats.conflicts,
            stats.conflicts as f64 / stats.evaluated.max(1) as f64 * 100.0
        );
    }
    Ok(())
}

#[derive(Default)]
struct BlockReportStats {
    root: String,
    block_index: usize,
    records: usize,
    cp_count: usize,
    cp_sum: f64,
    cp_sum_sq: f64,
    outcomes: BTreeMap<&'static str, usize>,
    in_check: usize,
    material_count: usize,
    material_sum: f64,
    king_camp_count: usize,
    activity_count: usize,
    activity_sum: f64,
}

impl BlockReportStats {
    fn new(root: String, block_index: usize) -> Self {
        Self {
            root,
            block_index,
            ..Self::default()
        }
    }

    fn add(&mut self, record: &PositionRecord) {
        self.records += 1;
        if let Some(cp) = eval_black(record) {
            let cp = cp as f64;
            self.cp_count += 1;
            self.cp_sum += cp;
            self.cp_sum_sq += cp * cp;
        }
        let outcome = record
            .game_result
            .as_ref()
            .map(|result| match result.outcome {
                GameOutcome::BlackWins => "black_wins",
                GameOutcome::WhiteWins => "white_wins",
                GameOutcome::Draw => "draw",
                GameOutcome::Unknown => "unknown",
            })
            .unwrap_or("no_game_result");
        *self.outcomes.entry(outcome).or_default() += 1;
        self.in_check += usize::from(record.tags.in_check);

        if let Some((material, king_in_camp, active)) = board_block_features(&record.sfen) {
            self.material_count += 1;
            self.material_sum += f64::from(material);
            self.king_camp_count += usize::from(king_in_camp);
            self.activity_count += 1;
            self.activity_sum += f64::from(active);
        }
    }

    fn print(&self) {
        let cp_mean = self.cp_sum / self.cp_count.max(1) as f64;
        let cp_variance =
            (self.cp_sum_sq / self.cp_count.max(1) as f64 - cp_mean * cp_mean).max(0.0);
        let material_mean = self.material_sum / self.material_count.max(1) as f64;
        let camp_ratio = self.king_camp_count as f64 / self.material_count.max(1) as f64;
        let activity_mean = self.activity_sum / self.activity_count.max(1) as f64;
        println!(
            "  {}#block{} records={} cp_count={} cp_mean={:.1} cp_variance={:.1} \
             in_check={:.1}% material_balance_black={:.2} king_in_enemy_camp={:.1}% \
             pieces_in_enemy_camp={:.2} black_wins={} white_wins={} draw={} unknown={} no_game_result={}",
            self.root,
            self.block_index,
            self.records,
            self.cp_count,
            cp_mean,
            cp_variance,
            self.in_check as f64 / self.records.max(1) as f64 * 100.0,
            material_mean,
            camp_ratio * 100.0,
            activity_mean,
            self.outcomes.get("black_wins").copied().unwrap_or(0),
            self.outcomes.get("white_wins").copied().unwrap_or(0),
            self.outcomes.get("draw").copied().unwrap_or(0),
            self.outcomes.get("unknown").copied().unwrap_or(0),
            self.outcomes.get("no_game_result").copied().unwrap_or(0),
        );
    }
}

/// Returns black-minus-white material, whether either king is in the opponent's camp, and a
/// deliberately small activity proxy: non-king pieces currently in the opponent's camp. These
/// are diagnostics, not engine features or claims about legal mobility.
fn board_block_features(sfen: &str) -> Option<(i32, bool, u32)> {
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
    let mut material = 0i32;
    let mut king_in_enemy_camp = false;
    let mut active = 0u32;
    for (rank_index, row) in board.grid.iter().enumerate() {
        let rank = rank_index + 1;
        for (side, piece) in row.iter().flatten() {
            let value = piece_point_value(*piece) as i32;
            material += if *side == SideToMove::Black {
                value
            } else {
                -value
            };
            let enemy_camp = match side {
                SideToMove::Black => rank <= 3,
                SideToMove::White => rank >= 7,
            };
            if enemy_camp {
                if *piece == PieceType::King {
                    king_in_enemy_camp = true;
                } else {
                    active += 1;
                }
            }
        }
    }
    for (side_index, hand) in board.hand.iter().enumerate() {
        for (piece, &count) in HAND_ORDER.iter().zip(hand.iter()) {
            let value = piece_point_value(*piece) as i32 * i32::from(count);
            material += if side_index == 0 { value } else { -value };
        }
    }
    Some((material, king_in_enemy_camp, active))
}

pub(crate) fn cmd_block_report(args: BlockReportArgs) -> Result<()> {
    if args.block_size == 0 {
        anyhow::bail!("--block-size must be > 0");
    }
    let reader = open_position_jsonl(&args.input)?;
    let mut records_read = 0usize;
    let mut parse_errors = 0usize;
    let mut blocks = 0usize;
    let mut current: Option<BlockReportStats> = None;

    println!("=== shogiesa block-report ===");
    println!("input             : {:?}", args.input);
    println!("block size        : {}", args.block_size);
    println!("variance          : population variance over deepest black-perspective CP");
    println!("material balance  : integer points, black minus white; activity is enemy-camp proxy");
    println!("blocks:");

    for item in reader {
        records_read += 1;
        let Some(record) = permissive_position_record(item, &args.input, &mut parse_errors)? else {
            continue;
        };
        let root = split_root_key(&record.source).to_string();
        let root_changed = current.as_ref().is_some_and(|block| block.root != root);
        if (root_changed
            || current
                .as_ref()
                .is_some_and(|block| block.records >= args.block_size))
            && let Some(block) = current.take()
        {
            block.print();
            blocks += 1;
        }
        if current.is_none() {
            current = Some(BlockReportStats::new(root, blocks + 1));
        }
        current
            .as_mut()
            .expect("block initialized above")
            .add(&record);
    }
    if let Some(block) = current {
        block.print();
        blocks += 1;
    }

    println!("records read      : {records_read}");
    println!("parse errors      : {parse_errors}");
    println!("blocks            : {blocks}");
    Ok(())
}

// bucket_floor (formerly ply_bucket_floor), mean_of, classify_bucket now live in
// stratifykit-core (imported above); `bucket_size == 0` is still checked and rejected at
// `cmd_distribution`'s entry point, matching this codebase's convention of validating there, not
// deep inside a pure helper.

/// ±5000cp at 200cp/bucket -- generous headroom over any realistic shogi eval; caps the phase x
/// side x eval-bucket grid's enumerated cp-axis length so a wildly out-of-range engine score can't
/// blow up the printed table. `report`'s own eval histogram has no such cap (it only ever prints
/// buckets it actually saw), so there's no existing bound to inherit here.
const MAX_EVAL_CP_SPAN_BUCKETS: i32 = 50;

pub(crate) fn cmd_distribution(args: DistributionArgs) -> Result<()> {
    if args.ply_bucket_size == 0 {
        anyhow::bail!("--ply-bucket-size must be > 0");
    }

    let reader = open_position_jsonl(&args.input)?;

    let mut n = 0usize;
    let mut broken = 0usize;
    let mut cross_tally: BTreeMap<(GamePhase, SideToMove, EvalBucket), usize> = BTreeMap::new();
    let mut ply_tally: BTreeMap<u32, usize> = BTreeMap::new();
    let mut root_tally: HashMap<String, usize> = HashMap::new();
    let mut wdl_tally: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut result_source_tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut eval_cp_min: Option<i32> = None;
    let mut eval_cp_max: Option<i32> = None;
    let mut ply_min: Option<u32> = None;
    let mut ply_max: Option<u32> = None;

    for item in reader {
        let Some(rec) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
        };
        n += 1;

        let eb = eval_bucket_of(&rec);
        if let EvalBucket::Cp(v) = eb {
            eval_cp_min = Some(eval_cp_min.map_or(v, |m| m.min(v)));
            eval_cp_max = Some(eval_cp_max.map_or(v, |m| m.max(v)));
        }
        *cross_tally
            .entry((rec.tags.phase, rec.tags.side_to_move, eb))
            .or_default() += 1;

        let ply = rec.source.ply;
        *ply_tally
            .entry(bucket_floor(ply, args.ply_bucket_size))
            .or_default() += 1;
        ply_min = Some(ply_min.map_or(ply, |m| m.min(ply)));
        ply_max = Some(ply_max.map_or(ply, |m| m.max(ply)));

        *root_tally
            .entry(shogiesa_stratify::group_key(&rec.source))
            .or_default() += 1;

        *wdl_tally
            .entry(shogiesa_stratify::feature_wdl(&rec))
            .or_default() += 1;

        *result_source_tally
            .entry(
                rec.game_result
                    .as_ref()
                    .map(|gr| gr.result_source.clone())
                    .unwrap_or_else(|| "no_game_result".to_string()),
            )
            .or_default() += 1;
    }

    if n == 0 {
        println!("no valid records in {:?}", args.input);
        return Ok(());
    }

    println!("=== shogiesa distribution ===");
    println!("positions   : {n}");
    println!("broken lines: {broken}");

    print_eval_coverage(
        &cross_tally,
        eval_cp_min,
        eval_cp_max,
        args.under_ratio,
        args.over_ratio,
    );
    print_ply_histogram(
        &ply_tally,
        ply_min.unwrap(),
        ply_max.unwrap(),
        args.ply_bucket_size,
        args.under_ratio,
        args.over_ratio,
    );
    print_source_root_distribution(&root_tally, n);
    print_wdl_distribution(&wdl_tally, n);
    print_result_source_distribution(&result_source_tally, n);

    Ok(())
}

fn print_coverage_row(
    phase: GamePhase,
    side: SideToMove,
    eb: EvalBucket,
    count: usize,
    flag: impl std::fmt::Display,
) {
    let eval_label = match eb {
        EvalBucket::Cp(v) => format!("{v:+}..{:+}", i64::from(v) + 199),
        EvalBucket::Mate => "mate".to_string(),
        EvalBucket::Unlabeled => "unlabeled".to_string(),
    };
    let label = format!("{phase}:{side}:{eval_label}");
    println!("  {label:<32}{count:>6}  {flag}");
}

/// The flagship new capability: unlike `report`'s eval/cross-tab stats and `select --strategy
/// coverage`'s bucket ranking (both of which only ever populate a tally from records actually
/// seen, so a combination with zero records simply never appears), this enumerates the full
/// expected bucket space and prints every combination -- including empty ones -- so a gap is
/// visible instead of silently absent.
///
/// The 12 sentinel cells (mate/unlabeled x phase/side) are tracked and reported separately from
/// the numeric cp grid: a fully-labeled, no-mate dataset will *always* show all 12 sentinel cells
/// empty, which is expected, not a gap -- folding that count into one "N missing" headline would
/// make the number meaningless noise on the most common (fully-labeled) datasets.
fn print_eval_coverage(
    cross_tally: &BTreeMap<(GamePhase, SideToMove, EvalBucket), usize>,
    eval_cp_min: Option<i32>,
    eval_cp_max: Option<i32>,
    under_ratio: f64,
    over_ratio: f64,
) {
    println!();
    println!("phase x side x eval-bucket coverage:");

    let phases = [
        GamePhase::Opening,
        GamePhase::Middlegame,
        GamePhase::Endgame,
    ];
    let sides = [SideToMove::Black, SideToMove::White];

    let cp_mean = mean_of(
        &cross_tally
            .iter()
            .filter(|&(&(_, _, eb), _)| matches!(eb, EvalBucket::Cp(_)))
            .map(|(_, &c)| c)
            .collect::<Vec<_>>(),
    );

    let span_ok = match (eval_cp_min, eval_cp_max) {
        // Widen before subtraction: JSONL can carry any i32 score, including opposite extremes.
        (Some(lo), Some(hi)) => {
            (i64::from(hi) - i64::from(lo)) / 200 < i64::from(MAX_EVAL_CP_SPAN_BUCKETS)
        }
        _ => true, // no cp observations at all -- nothing to enumerate, trivially "ok"
    };

    let mut cp_cells_total = 0usize;
    let mut cp_cells_missing = 0usize;
    let mut sentinel_cells_missing = 0usize;

    for &phase in &phases {
        for &side in &sides {
            // Sentinels first (matches EvalBucket's declared Ord: Unlabeled < Cp < Mate). A
            // sentinel cell that isn't empty is never itself flagged UNDER/OVER -- "-" instead --
            // since there's no expectation for how many mate/unlabeled positions a dataset should
            // have.
            for eb in [EvalBucket::Unlabeled, EvalBucket::Mate] {
                let count = cross_tally.get(&(phase, side, eb)).copied().unwrap_or(0);
                if count == 0 {
                    sentinel_cells_missing += 1;
                    print_coverage_row(phase, side, eb, count, BucketStatus::Missing);
                } else {
                    print_coverage_row(phase, side, eb, count, "-");
                }
            }
            if let (Some(lo), Some(hi)) = (eval_cp_min, eval_cp_max)
                && span_ok
            {
                let mut v = lo;
                while v <= hi {
                    cp_cells_total += 1;
                    let eb = EvalBucket::Cp(v);
                    let count = cross_tally.get(&(phase, side, eb)).copied().unwrap_or(0);
                    let status = classify_bucket(count, cp_mean, under_ratio, over_ratio);
                    if status == BucketStatus::Missing {
                        cp_cells_missing += 1;
                    }
                    print_coverage_row(phase, side, eb, count, status);
                    let Some(next) = v.checked_add(200) else {
                        break;
                    };
                    v = next;
                }
            }
        }
    }

    if !span_ok {
        println!(
            "  cp-bucket span too wide ({}..{}) -- exceeds {MAX_EVAL_CP_SPAN_BUCKETS} enumerated \
             buckets; showing observed cp buckets only, missing-bucket detection skipped for this run",
            eval_cp_min.unwrap(),
            eval_cp_max.unwrap()
        );
        for (&(phase, side, eb), &count) in cross_tally
            .iter()
            .filter(|&(&(_, _, eb), _)| matches!(eb, EvalBucket::Cp(_)))
        {
            let flag = classify_bucket(count, cp_mean, under_ratio, over_ratio);
            print_coverage_row(phase, side, eb, count, flag);
        }
    } else {
        println!("  (cp-grid: {cp_cells_total} cells, {cp_cells_missing} missing)");
    }
    println!(
        "  (sentinel cells (mate/unlabeled x phase/side): 12 cells, {sentinel_cells_missing} empty)"
    );
}

fn print_ply_histogram(
    ply_tally: &BTreeMap<u32, usize>,
    ply_min: u32,
    ply_max: u32,
    bucket_size: u32,
    under_ratio: f64,
    over_ratio: f64,
) {
    println!();
    println!("ply distribution (bucket size {bucket_size}):");
    let mean = mean_of(&ply_tally.values().copied().collect::<Vec<_>>());
    let lo = bucket_floor(ply_min, bucket_size);
    let hi = bucket_floor(ply_max, bucket_size);
    let mut total = 0usize;
    let mut missing = 0usize;
    let mut b = lo;
    while b <= hi {
        total += 1;
        let count = ply_tally.get(&b).copied().unwrap_or(0);
        let flag = classify_bucket(count, mean, under_ratio, over_ratio);
        if flag == BucketStatus::Missing {
            missing += 1;
        }
        println!(
            "  {b:>4}..{:<4}{count:>6}  {flag}",
            b.saturating_add(bucket_size.saturating_sub(1))
        );
        let Some(next) = b.checked_add(bucket_size) else {
            break;
        };
        b = next;
    }
    println!("  ({total} buckets enumerated, {missing} missing)");
}

/// Root-id-aware source distribution -- unlike `report`'s `sources` stat (keyed by raw
/// `rec.source.path`, so a game's mainline and its variations count as separate "sources"), this
/// groups via `shogiesa_stratify::group_key` (root_id when present, falling back to stripping the
/// path's `#varN@ply` suffix) -- the same root-id-or-path grouping `split
/// --train/--valid/--test` uses (via its own copy, `split_root_key`) for leakage safety.
fn print_source_root_distribution(root_tally: &HashMap<String, usize>, n: usize) {
    println!();
    println!("source-root distribution:");
    let distinct_roots = root_tally.len();
    let top_root_count = root_tally.values().copied().max().unwrap_or(0);
    let top_root_pct = top_root_count as f64 / n as f64 * 100.0;
    let warn = if top_root_pct > 50.0 {
        "WARN: too concentrated"
    } else {
        "OK"
    };
    println!("  distinct roots : {distinct_roots}");
    println!("  top root       : {top_root_pct:.1}%  {warn}");
}

/// A simple 1-D count/percentage tally, not the eval-bucket grid's full zero-count enumeration --
/// WDL's `"success"`/`"failure"`/`"draw"`/`"unknown"` categories are inherently unbalanced (unlike
/// the numeric eval/ply axes' expected-uniform-fill assumption), so there's no "gap" to flag here.
fn print_wdl_distribution(wdl_tally: &BTreeMap<&'static str, usize>, n: usize) {
    println!();
    println!("wdl distribution (mover-relative):");
    for (label, count) in wdl_tally {
        let pct = *count as f64 / n as f64 * 100.0;
        println!("  {label:<10}{count:>8}  {pct:.1}%");
    }
}

fn print_result_source_distribution(tally: &BTreeMap<String, usize>, n: usize) {
    println!();
    println!("game_result source distribution:");
    for (label, count) in tally {
        let pct = *count as f64 / n as f64 * 100.0;
        println!("  {label:<24}{count:>8}  {pct:.1}%");
    }
}

pub(crate) fn cmd_validate(args: ValidateArgs) -> Result<()> {
    // Why: validate is meant to run against multi-GB JSONL exports, so the input is read one
    // line at a time instead of buffering the whole file into memory.
    let file = File::open(&args.input).with_context(|| format!("cannot read {:?}", args.input))?;
    let reader = BufReader::new(file);

    let mut total_lines = 0usize;
    let mut valid_json = 0usize;
    let mut valid_records = 0usize;
    let mut invalid_records = 0usize;
    let mut unsupported_schema_versions = BTreeMap::<u32, usize>::new();
    let mut tag_mismatches = 0usize;
    let mut invalid_sfens = 0usize;
    let mut schema_versions = BTreeMap::<u32, usize>::new();
    let mut seen_sfens: HashSet<String> = HashSet::new();
    let mut duplicate_sfens = 0usize;

    for (line_index, line) in reader.lines().enumerate() {
        let source_line = line_index + 1;
        let line = line.with_context(|| format!("cannot read {:?}", args.input))?;
        if line.trim().is_empty() {
            continue;
        }
        total_lines += 1;

        if serde_json::from_str::<serde_json::Value>(&line).is_err() {
            continue;
        }
        valid_json += 1;

        let rec = match parse_json_line(&line) {
            Ok(rec) => rec,
            Err(error) => {
                invalid_records += 1;
                if let PositionRecordParseError::UnsupportedSchema(version) = &error {
                    *unsupported_schema_versions
                        .entry(version.found)
                        .or_default() += 1;
                }
                tracing::warn!(line = source_line, "invalid position record: {error}");
                continue;
            }
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
    let has_problems = tag_mismatches > 0 || broken > 0 || invalid_records > 0 || invalid_sfens > 0;

    println!("=== shogiesa validate ===");
    println!("total lines    : {total_lines}");
    println!("valid JSON     : {valid_json}");
    println!("valid records  : {valid_records}");
    println!("broken lines   : {broken}");
    println!("invalid records: {invalid_records}");
    println!("invalid SFENs  : {invalid_sfens}");
    println!("duplicate SFENs: {duplicate_sfens}");
    println!("tag mismatches : {tag_mismatches}  (side_to_move vs SFEN)");
    println!("schema versions: {schema_versions:?}");
    println!("unsupported schema versions: {unsupported_schema_versions:?}");

    if has_problems {
        println!();
        if broken > 0 {
            println!("WARN: {broken} broken lines");
        }
        if invalid_records > 0 {
            println!("WARN: {invalid_records} invalid position records");
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
