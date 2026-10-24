use super::super::*;
use super::label::parse_positive_csv;

/// Parses `--phase`'s comma-separated `opening,middlegame,endgame` list into `QualityConfig`'s
/// `allowed_phases` -- shared by `filter`, `calibrate`, and `tune`. Unknown or duplicate entries
/// fail closed: silently ignoring a typo can produce a different training dataset than requested.
fn parse_allowed_phases(phase: Option<&str>) -> Result<Option<Vec<GamePhase>>> {
    let Some(value) = phase else {
        return Ok(None);
    };
    let mut phases = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in value.split(',') {
        let name = raw.trim();
        let parsed = match name {
            "opening" => GamePhase::Opening,
            "middlegame" => GamePhase::Middlegame,
            "endgame" => GamePhase::Endgame,
            "" => anyhow::bail!("--phase contains an empty value: {value:?}"),
            other => anyhow::bail!(
                "unknown --phase value {other:?}; expected opening, middlegame, or endgame"
            ),
        };
        if !seen.insert(name) {
            anyhow::bail!("--phase values must be distinct, duplicate {name:?}");
        }
        phases.push(parsed);
    }
    Ok(Some(phases))
}

fn validate_quality_config(config: &QualityConfig) -> Result<()> {
    if config.max_score_swing_cp.is_some_and(|value| value < 0) {
        anyhow::bail!("--max-score-swing-cp must be non-negative");
    }
    if config
        .max_engine_score_swing_cp
        .is_some_and(|value| value < 0)
    {
        anyhow::bail!("--max-engine-score-swing-cp must be non-negative");
    }
    if config.min_depth_reached == Some(0) {
        anyhow::bail!("--min-depth-reached must be greater than zero");
    }
    if let (Some(minimum), Some(maximum)) = (config.eval_min, config.eval_max)
        && minimum > maximum
    {
        anyhow::bail!("--eval-min must be less than or equal to --eval-max");
    }
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
        exclude_timeout_salvaged: args.exclude_timeout_salvaged,
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
        allow_timeout_salvaged_mate: args.allow_timeout_salvaged_mate,
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

pub(crate) fn cmd_filter(args: FilterArgs) -> Result<()> {
    let config = match &args.preset {
        Some(spec) => load_quality_config_preset(spec)?,
        None => {
            let allowed_phases = parse_allowed_phases(args.phase.as_deref())?;
            build_quality_config(&args, allowed_phases)
        }
    };
    validate_quality_config(&config)?;

    let mut reader = open_hashed_position_jsonl(&args.input)?;
    let mut writer = args.out.as_deref().map(AtomicWriter::new).transpose()?;
    let mut explain_writer = args
        .explain_out
        .as_deref()
        .map(AtomicWriter::new)
        .transpose()?;

    let mut total = 0usize;
    let mut passed = 0usize;
    let mut skipped = 0usize;
    let mut drop_reasons: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut manifest = args
        .manifest
        .is_some()
        .then(|| new_run_manifest("filter", &args.input));

    for item in &mut reader {
        total += 1;
        let Some(rec) = permissive_position_record(item, &args.input, &mut skipped)? else {
            *drop_reasons.entry("parse_error").or_default() += 1;
            continue;
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
    let (input_hash, _exact_sha256) = finish_hashed_position_jsonl(reader);

    if let Some(w) = writer {
        w.commit()?;
    }
    if let Some(w) = explain_writer {
        w.commit()?;
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
        m.input_hash = input_hash;
        m.records_read = total;
        m.records_kept = passed;
        m.records_dropped = skipped;
        m.drop_reasons = drop_reasons
            .into_iter()
            .map(|(reason, count)| (reason.to_string(), count))
            .collect();
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

fn parse_distinct_int_list(value: &str, flag: &str) -> Result<Vec<i32>> {
    let mut parsed = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in value.split(',') {
        let item = raw.trim();
        if item.is_empty() {
            anyhow::bail!("{flag} contains an empty value: {value:?}");
        }
        let number = item
            .parse::<i32>()
            .with_context(|| format!("invalid integer {item:?} in {flag}"))?;
        if !seen.insert(number) {
            anyhow::bail!("{flag} values must be distinct, duplicate {number}");
        }
        parsed.push(number);
    }
    Ok(parsed)
}

fn parse_score_swing_list(value: &str, flag: &str) -> Result<Vec<i32>> {
    let parsed = parse_distinct_int_list(value, flag)?;
    if let Some(negative) = parsed.iter().find(|&&number| number < 0) {
        anyhow::bail!("{flag} values must be non-negative, got {negative}");
    }
    Ok(parsed)
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
    margin_buckets: BTreeMap<i64, usize>,
    swing_buckets: BTreeMap<i64, usize>,
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
                    .entry((i64::from(margin).div_euclid(50)) * 50)
                    .or_default() += 1;
            }
            if let Score::Cp { value } = obs.score {
                cp_scores.push(value);
            }
            let bound_key = score_bound_str(obs.score_bound);
            *self.obs_score_bound_counts.entry(bound_key).or_default() += 1;
        }
        if let Some(swing) = score_swing_i64(&cp_scores) {
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

pub(crate) fn cmd_calibrate(args: CalibrateArgs) -> Result<()> {
    if args.sweep_policy_margin.is_none() && args.sweep_score_swing.is_none() {
        anyhow::bail!(
            "calibrate requires at least one of --sweep-policy-margin/--sweep-score-swing"
        );
    }

    let allowed_phases = parse_allowed_phases(args.phase.as_deref())?;
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
        exclude_timeout_salvaged: args.exclude_timeout_salvaged,
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
        allow_timeout_salvaged_mate: args.allow_timeout_salvaged_mate,
    };
    validate_quality_config(&base_config)?;

    let mut policy_margin_rows: Vec<SweepRow> = match &args.sweep_policy_margin {
        Some(s) => parse_distinct_int_list(s, "--sweep-policy-margin")?
            .into_iter()
            .map(|v| SweepRow::new("policy_margin", v))
            .collect(),
        None => Vec::new(),
    };
    let mut score_swing_rows: Vec<SweepRow> = match &args.sweep_score_swing {
        Some(s) => parse_score_swing_list(s, "--sweep-score-swing")?
            .into_iter()
            .map(|v| SweepRow::new("score_swing", v))
            .collect(),
        None => Vec::new(),
    };
    // These configs are invariant across records. Building them once avoids cloning the base
    // phase vector and every other fixed field for each (record, threshold) pair.
    let policy_margin_configs: Vec<QualityConfig> = policy_margin_rows
        .iter()
        .map(|row| QualityConfig {
            min_policy_margin_cp: Some(row.value),
            ..base_config.clone()
        })
        .collect();
    let score_swing_configs: Vec<QualityConfig> = score_swing_rows
        .iter()
        .map(|row| QualityConfig {
            max_score_swing_cp: Some(row.value),
            ..base_config.clone()
        })
        .collect();

    let mut total = 0usize;
    let mut broken = 0usize;
    let mut diagnostics = DatasetDiagnostics::default();
    let mut source_roots = BTreeMap::new();
    let mut engines = BTreeMap::new();
    let mut weights = BTreeMap::new();

    for item in open_position_jsonl(&args.input)? {
        let Some(rec) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
        };
        total += 1;
        record_manifest_provenance(&rec, &mut source_roots, &mut engines, &mut weights);

        // Each swept value gets its own evaluate_quality call, overriding only the one field this
        // sweep varies -- everything else (including the OTHER dimension, if held via --min-x/
        // --max-x) stays at the base config.
        for (row, config) in policy_margin_rows.iter_mut().zip(&policy_margin_configs) {
            row.coverage.record(&evaluate_quality(&rec, config));
        }
        for (row, config) in score_swing_rows.iter_mut().zip(&score_swing_configs) {
            row.coverage.record(&evaluate_quality(&rec, config));
        }

        // Sweep-independent, so accumulated once per record regardless of threshold, not
        // duplicated per sweep row (which would wrongly imply it varies by threshold).
        diagnostics.record(&rec);
    }

    let mut writer = AtomicWriter::new(&args.out)?;
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
    writer.commit()?;

    if let Some(manifest_path) = &args.manifest {
        let mut manifest = new_run_manifest("calibrate", &args.input);
        manifest.input_hash = hash_file(&args.input)?;
        manifest.records_read = total;
        manifest.records_kept = total;
        manifest.labeled_records = diagnostics.labeled;
        manifest.output_sha256 = Some(hash_file(&args.out)?);
        manifest.source_root_distribution = Some(source_roots);
        manifest.engine_distribution = Some(engines);
        manifest.weight_distribution = Some(weights);
        write_manifest(manifest_path, &manifest)?;
    }

    eprintln!(
        "done: {total} read, {} labeled → {:?}",
        diagnostics.labeled, args.out
    );
    diagnostics.print();

    Ok(())
}

fn parse_student_depths(value: &str, teacher_depth: u32) -> Result<Vec<u32>> {
    if teacher_depth == 0 {
        anyhow::bail!("--teacher-depth must be greater than zero");
    }
    let depths: Vec<u32> = parse_positive_csv(value, "--student-depths")?;
    if let Some(invalid) = depths.iter().find(|&&depth| depth >= teacher_depth) {
        anyhow::bail!(
            "--student-depths must all be shallower than --teacher-depth {teacher_depth}, got {invalid}"
        );
    }
    Ok(depths)
}

/// Aggregate stats for one student depth (or overall, across every student depth) -- shared
/// accumulation so `cmd_audit`'s per-depth and overall summaries can't diverge.
#[derive(Default)]
struct AuditStats {
    pairs: usize,
    bestmove_mismatches: usize,
    abs_score_error_sum: f64,
    abs_score_error_count: usize,
    abs_score_error_max: i64,
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
        score_error_cp: Option<i64>,
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
) -> Vec<(u32, bool, Option<i64>, &'a Observation, &'a Observation)> {
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
                    let t_black = cp_from_black_perspective_i64(
                        *t,
                        teacher.score_perspective,
                        rec.tags.side_to_move,
                    );
                    let s_black = cp_from_black_perspective_i64(
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

pub(crate) fn cmd_audit(args: AuditArgs) -> Result<()> {
    let student_depths = parse_student_depths(&args.student_depths, args.teacher_depth)?;

    let mut writer = AtomicWriter::new(&args.out)?;

    let mut total_records = 0usize;
    let mut broken = 0usize;
    let mut overall = AuditStats::default();
    let mut per_depth: BTreeMap<u32, AuditStats> = BTreeMap::new();
    let mut source_roots = BTreeMap::new();
    let mut engines = BTreeMap::new();
    let mut weights = BTreeMap::new();

    for item in open_position_jsonl(&args.input)? {
        let Some(rec) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
        };
        total_records += 1;
        record_manifest_provenance(&rec, &mut source_roots, &mut engines, &mut weights);

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

    writer.commit()?;
    if let Some(manifest_path) = &args.manifest {
        let mut manifest = new_run_manifest("audit", &args.input);
        manifest.input_hash = hash_file(&args.input)?;
        manifest.records_read = total_records;
        manifest.records_kept = overall.pairs;
        manifest.labeled_records = total_records;
        manifest.output_sha256 = Some(hash_file(&args.out)?);
        manifest.source_root_distribution = Some(source_roots);
        manifest.engine_distribution = Some(engines);
        manifest.weight_distribution = Some(weights);
        write_manifest(manifest_path, &manifest)?;
    }
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
/// Why: coverage and mismatch_rate are both mathematically in \[0,1\], but their *dynamic range
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
        atomic_write_bytes(path, report.as_bytes())?;
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

    atomic_write_bytes(path, report.as_bytes())?;
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
    atomic_write_bytes(path, serde_json::to_string_pretty(&file)?.as_bytes())?;
    Ok(())
}

pub(crate) fn cmd_tune(args: TuneArgs) -> Result<()> {
    if args.sweep_policy_margin.is_none() && args.sweep_score_swing.is_none() {
        anyhow::bail!("tune requires at least one of --sweep-policy-margin/--sweep-score-swing");
    }
    let student_depths = parse_student_depths(&args.student_depths, args.teacher_depth)?;

    let allowed_phases = parse_allowed_phases(args.phase.as_deref())?;
    // Same convention as calibrate: a field currently being swept is always None here (overridden
    // per grid cell below); Some only when that field is instead held fixed while the OTHER
    // dimension sweeps (enforced by clap's conflicts_with on the sweep/hold pairs).
    let base_config = QualityConfig {
        min_observations: args.min_observations,
        allowed_phases,
        exclude_mate: args.exclude_mate,
        exclude_in_check: args.exclude_in_check,
        exclude_capture: args.exclude_capture,
        exclude_timeout_salvaged: args.exclude_timeout_salvaged,
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
        allow_timeout_salvaged_mate: args.allow_timeout_salvaged_mate,
    };
    validate_quality_config(&base_config)?;

    // A held-but-not-swept axis becomes a single-value list (possibly `None`, meaning "gate
    // inactive") -- this makes `tune` a strict superset of `calibrate`'s shape: a 1xN or Nx1 grid
    // degenerates to exactly calibrate's independent-sweep behavior, not a divergent second mode.
    let policy_values: Vec<Option<i32>> = match &args.sweep_policy_margin {
        Some(s) => parse_distinct_int_list(s, "--sweep-policy-margin")?
            .into_iter()
            .map(Some)
            .collect(),
        None => vec![args.min_policy_margin_cp],
    };
    let swing_values: Vec<Option<i32>> = match &args.sweep_score_swing {
        Some(s) => parse_score_swing_list(s, "--sweep-score-swing")?
            .into_iter()
            .map(Some)
            .collect(),
        None => vec![args.max_score_swing_cp],
    };

    let mut grid: Vec<TuneCell> = Vec::with_capacity(policy_values.len() * swing_values.len());
    for &policy_margin in &policy_values {
        for &score_swing_value in &swing_values {
            grid.push(TuneCell::new(policy_margin, score_swing_value));
        }
    }
    // The grid is fixed before the stream starts. Resolve each cell's full config once rather
    // than cloning the base config (including allowed phases) for every input record.
    let grid_configs: Vec<QualityConfig> = grid
        .iter()
        .map(|cell| QualityConfig {
            min_policy_margin_cp: cell.policy_margin,
            max_score_swing_cp: cell.score_swing,
            ..base_config.clone()
        })
        .collect();
    let mut total = 0usize;
    let mut broken = 0usize;
    let mut diagnostics = DatasetDiagnostics::default();
    let mut source_roots = BTreeMap::new();
    let mut engines = BTreeMap::new();
    let mut weights = BTreeMap::new();

    for item in open_position_jsonl(&args.input)? {
        let Some(rec) = permissive_position_record(item, &args.input, &mut broken)? else {
            continue;
        };
        total += 1;
        record_manifest_provenance(&rec, &mut source_roots, &mut engines, &mut weights);
        diagnostics.record(&rec);

        // Computed ONCE per record, independent of any grid cell's thresholds -- the comparison
        // itself doesn't depend on a quality gate; only whether a given cell counts it toward
        // "kept" does. Folding these into every cell below, instead of recomputing per cell,
        // is what keeps this a single streaming pass despite an M*N grid.
        let audit_pairs = compute_audit_pairs(&rec, args.teacher_depth, &student_depths);

        // evaluate_quality remains the single source of gate semantics; only its immutable
        // per-cell configs are cached. This avoids parallel judgment logic while removing
        // M*N config allocations from the streaming hot path.
        for (cell, config) in grid.iter_mut().zip(&grid_configs) {
            let decision = evaluate_quality(&rec, config);
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

    let mut writer = AtomicWriter::new(&args.out)?;
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
    writer.commit()?;

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

    if let Some(manifest_path) = &args.manifest {
        let mut manifest = new_run_manifest("tune", &args.input);
        manifest.input_hash = hash_file(&args.input)?;
        manifest.records_read = total;
        manifest.records_kept = total;
        manifest.labeled_records = diagnostics.labeled;
        manifest.output_sha256 = Some(hash_file(&args.out)?);
        manifest.source_root_distribution = Some(source_roots);
        manifest.engine_distribution = Some(engines);
        manifest.weight_distribution = Some(weights);
        write_manifest(manifest_path, &manifest)?;
    }

    Ok(())
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
