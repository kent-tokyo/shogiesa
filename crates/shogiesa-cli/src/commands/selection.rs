use super::super::*;
use super::label::hash_parts;
use super::transforms::split_root_key;

pub(crate) fn cmd_sample(args: SampleArgs) -> Result<()> {
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
pub(crate) fn cmd_shuffle(args: ShuffleArgs) -> Result<()> {
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
pub(super) fn eval_black(rec: &PositionRecord) -> Option<i64> {
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

pub(crate) fn cmd_mine(args: MineArgs) -> Result<()> {
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

pub(crate) fn cmd_balance(args: BalanceArgs) -> Result<()> {
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

pub(crate) fn cmd_stratify(args: StratifyArgs) -> Result<()> {
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
pub(crate) fn is_legal_bestmove_lite(
    sfen: &str,
    mv: &str,
) -> Result<bool, shogi_usi_parser::Error> {
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

pub(crate) fn cmd_make_gate_openings(args: MakeGateOpeningsArgs) -> Result<()> {
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

pub(crate) fn cmd_lineprior_export(args: LinePriorExportArgs) -> Result<()> {
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

pub(crate) fn cmd_select(args: SelectArgs) -> Result<()> {
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
