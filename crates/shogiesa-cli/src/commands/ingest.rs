use super::super::*;

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

pub(crate) fn cmd_extract(args: ExtractArgs) -> Result<()> {
    let config = shogiesa_core::ExtractConfig {
        min_ply: args.min_ply,
        max_ply: args.max_ply,
        every_n: args.every_n_plies,
        dedup: args.dedup,
    };

    let recursive_discovery = if args.recursive {
        let metadata = fs::symlink_metadata(&args.input)
            .with_context(|| format!("cannot inspect input {:?}", args.input))?;
        if metadata.file_type().is_symlink() {
            anyhow::bail!(
                "recursive input root must not be a symlink: {:?}",
                args.input
            );
        }
        metadata.is_dir()
    } else {
        false
    };
    let (paths, discovery): (Vec<GameInput>, Option<RecursiveDiscovery>) = if recursive_discovery {
        let discovery = collect_game_paths_recursive(&args.input)?;
        let paths = discovery
            .paths
            .iter()
            .map(|entry| GameInput {
                path: entry.path.clone(),
                source_path: Some(entry.source_path.clone()),
            })
            .collect();
        (paths, Some(discovery))
    } else {
        (
            collect_game_paths(&args.input)?
                .into_iter()
                .map(|path| GameInput {
                    path,
                    source_path: None,
                })
                .collect(),
            None,
        )
    };
    if paths.is_empty() {
        anyhow::bail!("no .csa, .kif, or .ki2 files found in {:?}", args.input);
    }
    if let Some(discovery) = &discovery {
        eprintln!(
            "discovery: {} game files, {} non-game files skipped, {} symlinks skipped, {} unreadable directories, {} unreadable entries",
            discovery.paths.len(),
            discovery.skipped_files,
            discovery.skipped_symlinks,
            discovery.unreadable_directories,
            discovery.unreadable_entries,
        );
    }

    let mut writer = AtomicWriter::new(&args.out)?;

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

    for input in &paths {
        total_games += 1;
        let path = &input.path;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let result = match (ext, input.source_path.as_deref()) {
            ("kif" | "ki2", Some(source_path)) => shogiesa_kif::extract_from_path_with_source(
                path,
                source_path,
                &extract_config,
                &mut seen,
            )
            .map_err(|e| e.to_string()),
            ("kif" | "ki2", None) => {
                shogiesa_kif::extract_from_path(path, &extract_config, &mut seen)
                    .map_err(|e| e.to_string())
            }
            (_, Some(source_path)) => shogiesa_csa::extract_from_path_with_source(
                path,
                source_path,
                &extract_config,
                &mut seen,
            )
            .map_err(|e| e.to_string()),
            (_, None) => shogiesa_csa::extract_from_path(path, &extract_config, &mut seen)
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

    writer.commit()?;
    eprintln!(
        "done: {} games, {} positions extracted, {} skipped → {:?}",
        total_games, total_positions, skipped, args.out
    );
    Ok(())
}

/// Which literal kifu-file label ("Engine1"/"Engine2") won a match-runner game, per its own
/// "# Result: ..." header line. Kept distinct from any candidate/baseline naming convention --
/// match-runner's own source doesn't guarantee which physical engine slot is "the candidate"
/// under test, so `from-match` only ever reasons about the labels the kifu file itself states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchResult {
    Engine1Win,
    Engine2Win,
    Draw,
}

/// Every `.txt` file directly inside `input` if it's a directory, or `input` itself if it's a
/// single file -- mirrors `collect_game_paths`'s file-or-directory convention but filters on the
/// match-runner's own kifu extension instead of game-record formats.
fn collect_match_kifu_paths(input: &Path) -> Result<Vec<PathBuf>> {
    if input.is_file() {
        return Ok(vec![input.to_path_buf()]);
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(input).with_context(|| format!("cannot read directory {input:?}"))? {
        let p = entry?.path();
        if p.extension().and_then(|e| e.to_str()) == Some("txt") {
            paths.push(p);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Parses one Sekirei match-runner kifu `.txt` file's header lines (everything before the
/// `position ...` line). Unrecognized lines are ignored, not errors -- forward-compatible with
/// extra headers the match-runner might add later.
///
/// Also captures each engine slot's color from its own header line (e.g. `# Engine1: ... (Black)`
/// -- present in every match-runner kifu today, previously parsed for identity only). Needed to
/// translate `MatchResult` (engine-slot-relative) into `GameOutcome` (color-absolute) for
/// `game_result` provenance -- distinct from `match_qualifies`'s existing candidate/baseline
/// reasoning, which stays engine-slot-relative and is unaffected by this.
fn parse_match_kifu_header(
    line: &str,
    result: &mut Option<MatchResult>,
    engine1_color: &mut Option<SideToMove>,
    engine2_color: &mut Option<SideToMove>,
) {
    let line = line.trim();
    if let Some(rest) = line.strip_prefix("# Engine1:") {
        if rest.trim_end().ends_with("(Black)") {
            *engine1_color = Some(SideToMove::Black);
        } else if rest.trim_end().ends_with("(White)") {
            *engine1_color = Some(SideToMove::White);
        }
        return;
    }
    if let Some(rest) = line.strip_prefix("# Engine2:") {
        if rest.trim_end().ends_with("(Black)") {
            *engine2_color = Some(SideToMove::Black);
        } else if rest.trim_end().ends_with("(White)") {
            *engine2_color = Some(SideToMove::White);
        }
        return;
    }
    match line {
        "# Result: Engine1 Win" => *result = Some(MatchResult::Engine1Win),
        "# Result: Engine2 Win" => *result = Some(MatchResult::Engine2Win),
        "# Result: Draw" => *result = Some(MatchResult::Draw),
        _ => {}
    }
}

/// Translates a match-runner-relative `MatchResult` plus each engine's own color into a
/// color-absolute `GameOutcome`, for `game_result` provenance. `None` if either engine's color
/// wasn't parsed from the header -- defensive: the real match-runner format always has both, but
/// this must not panic on a malformed or foreign kifu file.
fn match_result_to_game_outcome(
    result: MatchResult,
    engine1_color: Option<SideToMove>,
    engine2_color: Option<SideToMove>,
) -> Option<GameOutcome> {
    let wins = |color: SideToMove| match color {
        SideToMove::Black => GameOutcome::BlackWins,
        SideToMove::White => GameOutcome::WhiteWins,
    };
    match result {
        MatchResult::Draw => Some(GameOutcome::Draw),
        MatchResult::Engine1Win => engine1_color.map(wins),
        MatchResult::Engine2Win => engine2_color.map(wins),
    }
}

#[cfg(test)]
mod match_result_to_game_outcome_tests {
    use super::{GameOutcome, MatchResult, SideToMove, match_result_to_game_outcome};

    #[test]
    fn engine1_win_black_is_black_wins() {
        assert_eq!(
            match_result_to_game_outcome(
                MatchResult::Engine1Win,
                Some(SideToMove::Black),
                Some(SideToMove::White)
            ),
            Some(GameOutcome::BlackWins)
        );
    }

    #[test]
    fn engine2_win_white_is_black_wins() {
        // Engine2 plays White here, so Engine2 winning means Black (Engine1) lost.
        assert_eq!(
            match_result_to_game_outcome(
                MatchResult::Engine2Win,
                Some(SideToMove::Black),
                Some(SideToMove::White)
            ),
            Some(GameOutcome::WhiteWins)
        );
    }

    #[test]
    fn draw_is_draw_regardless_of_colors() {
        assert_eq!(
            match_result_to_game_outcome(MatchResult::Draw, None, None),
            Some(GameOutcome::Draw)
        );
    }

    #[test]
    fn missing_color_is_none() {
        assert_eq!(
            match_result_to_game_outcome(MatchResult::Engine1Win, None, Some(SideToMove::White)),
            None
        );
        assert_eq!(
            match_result_to_game_outcome(MatchResult::Engine2Win, Some(SideToMove::Black), None),
            None
        );
    }
}

/// Whether `--losing-side` (if given) selects this game, per the kifu's own literal Result line.
/// A Draw or missing/unparseable Result is never "a loss for either side" -- skipped entirely
/// under an explicit `--losing-side`, matching the plain English reading of the flag.
fn match_qualifies(losing_side: Option<&str>, result: Option<MatchResult>) -> bool {
    match losing_side {
        None => true,
        Some("engine1") => result == Some(MatchResult::Engine2Win),
        Some("engine2") => result == Some(MatchResult::Engine1Win),
        Some(_) => false, // unreachable: clap's value_parser restricts this already
    }
}

/// Extracts positions from one match-runner kifu file's content. Mirrors
/// `shogiesa_csa::extract_from_str`'s structure (stop-this-game-on-error resilience, same
/// `ExtractConfig` gates, same dedup set) but replays a `position startpos moves ...` or
/// `position sfen ... moves ...` USI move list instead of a CSA/KIF game record.
fn extract_from_match_kifu(
    content: &str,
    source_path: &str,
    losing_side: Option<&str>,
    config: &shogiesa_core::ExtractConfig,
    seen: &mut HashSet<String>,
) -> Vec<PositionRecord> {
    let mut result = None;
    let mut engine1_color = None;
    let mut engine2_color = None;
    let mut position_line = None;
    for line in content.lines() {
        if line.starts_with("position ") {
            position_line = Some(line);
            break;
        }
        parse_match_kifu_header(line, &mut result, &mut engine1_color, &mut engine2_color);
    }
    if !match_qualifies(losing_side, result) {
        return Vec::new();
    }
    let game_result = result
        .and_then(|r| match_result_to_game_outcome(r, engine1_color, engine2_color))
        .map(|outcome| GameResultInfo {
            outcome,
            result_source: "match_header".to_string(),
        });
    let Some(position_line) = position_line else {
        tracing::warn!(
            path = source_path,
            "no `position ...` line found, skipping game"
        );
        return Vec::new();
    };

    let tokens: Vec<&str> = position_line.split_whitespace().collect();
    let (mut board, move_tokens): (Board, &[&str]) = match tokens.as_slice() {
        ["position", "startpos", "moves", rest @ ..] => (Board::initial(SideToMove::Black), rest),
        ["position", "startpos"] => (Board::initial(SideToMove::Black), &[]),
        [
            "position",
            "sfen",
            b,
            side,
            hand,
            move_count,
            "moves",
            rest @ ..,
        ] => {
            let sfen = format!("{b} {side} {hand} {move_count}");
            match Board::from_sfen(&sfen) {
                Ok(board) => (board, rest),
                Err(e) => {
                    tracing::warn!(path = source_path, "bad sfen {sfen:?}: {e}");
                    return Vec::new();
                }
            }
        }
        ["position", "sfen", b, side, hand, move_count] => {
            let sfen = format!("{b} {side} {hand} {move_count}");
            match Board::from_sfen(&sfen) {
                Ok(board) => (board, &[] as &[&str]),
                Err(e) => {
                    tracing::warn!(path = source_path, "bad sfen {sfen:?}: {e}");
                    return Vec::new();
                }
            }
        }
        _ => {
            tracing::warn!(
                path = source_path,
                "unrecognized `position` line, skipping game"
            );
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    // `board`'s own `move_count` already reflects the starting position (1 for `startpos`, or
    // whatever `position sfen ...`'s move-count field says for a custom gate-opening start) --
    // deriving `ply` from it, instead of hardcoding 0, is what makes a non-startpos game's
    // extracted positions carry their true overall ply rather than restarting from this replay's
    // own move-index. No-op for startpos (`Board::initial` always sets `move_count == 1`).
    let mut ply: u32 = board.move_count - 1;
    for token in move_tokens {
        let mv = match parse_usi_move(token) {
            Ok(mv) => mv,
            Err(e) => {
                tracing::warn!(path = source_path, ply, "malformed move {token:?}: {e}");
                break;
            }
        };
        let mover = board.side;
        let has_capture = match mv {
            UsiMove::Normal {
                to_file, to_rank, ..
            } => board.is_capture(to_file, to_rank, mover),
            UsiMove::Drop { .. } => false,
        };
        if let Err(e) = board.apply_usi_move(mover, &mv) {
            tracing::warn!(path = source_path, ply, "board error: {e}");
            break;
        }
        ply += 1;

        if config.max_ply.is_some_and(|max| ply > max) {
            break;
        }
        if ply < config.min_ply {
            continue;
        }
        if !(ply - config.min_ply).is_multiple_of(config.every_n) {
            continue;
        }

        let sfen = board.to_sfen();
        if config.dedup && !seen.insert(sfen.clone()) {
            continue;
        }

        let tags = PositionTags {
            phase: phase_from_ply(ply),
            side_to_move: board.side,
            in_check: board.is_in_check(),
            has_capture,
        };
        // A match-runner game is strictly linear (no variation/branch concept) -- the kifu
        // file's own path already carries unambiguous run+game identity, exactly how CSA
        // extraction already relies on bare `path` with `root_id: None` today.
        //
        // `game_result` IS stamped per-record (computed above) for Gate-B-style raw-vs-curated
        // WDL provenance -- additive and independent of `--losing-side`: that flag's
        // pre-filtering (which games get extracted at all) is unchanged, and remains the only
        // mechanism for selecting "the losing side's positions". `game_result` exists so a
        // consumer inspecting WDL distribution on a NOT-pre-filtered extraction (e.g.
        // `--losing-side` omitted, every game kept) has a signal to work with.
        let source = SourceInfo {
            kind: "from_match".to_string(),
            path: source_path.to_string(),
            ply,
            root_id: None,
            variation_id: None,
            branch_from_ply: None,
        };
        let mut rec = PositionRecord::new(sfen, source, tags);
        rec.game_result = game_result.clone();
        out.push(rec);
    }
    out
}

pub(crate) fn cmd_from_match(args: FromMatchArgs) -> Result<()> {
    let config = shogiesa_core::ExtractConfig {
        min_ply: args.min_ply,
        max_ply: args.max_ply,
        every_n: args.every_n_plies,
        dedup: args.dedup,
    };

    let paths = collect_match_kifu_paths(&args.input)?;
    if paths.is_empty() {
        anyhow::bail!("no .txt kifu files found in {:?}", args.input);
    }

    let mut writer = AtomicWriter::new(&args.out)?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut total_games = 0usize;
    let mut total_positions = 0usize;

    for path in &paths {
        total_games += 1;
        let content = fs::read_to_string(path).with_context(|| format!("cannot read {path:?}"))?;
        let source = path.to_string_lossy().into_owned();
        let records = extract_from_match_kifu(
            &content,
            &source,
            args.losing_side.as_deref(),
            &config,
            &mut seen,
        );
        for rec in &records {
            serde_json::to_writer(&mut writer, rec)?;
            writer.write_all(b"\n")?;
            total_positions += 1;
        }
    }

    writer.commit()?;
    eprintln!(
        "done: {total_games} games read, {total_positions} positions extracted → {:?}",
        args.out
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum MergeObservationPolicy {
    KeepBoth,
    PreferPrimary,
    PreferSecondary,
}

/// Merges `incoming` into `base` per `policy`, keyed on `(engine, engine_version, depth,
/// requested_depth)` -- deliberately broader than `label`'s own in-place dedup key (which omits
/// `engine_version`): that narrower key is safe there because `label` already knows it's
/// re-running the same engine binary, but `merge-observations` is explicitly merging data whose
/// provenance might differ, so conflating two different engine versions at the same nominal
/// depth would be a real bug this command must guard against that `label` doesn't need to.
/// Returns how many keys collided.
fn merge_observations_into(
    base: &mut Vec<Observation>,
    incoming: Vec<Observation>,
    policy: MergeObservationPolicy,
) -> usize {
    // search_limit_kind/requested_nodes are included so a node-limited observation is never
    // considered a collision with a differently-node-limited one just because both happen to
    // report requested_depth: None -- without this, every node-limited observation from one
    // engine would collapse onto the same key regardless of its actual requested_nodes.
    let key = |o: &Observation| {
        (
            o.engine.clone(),
            o.engine_version.clone(),
            o.depth,
            o.requested_depth,
            o.search_limit_kind,
            o.requested_nodes,
        )
    };
    let mut collisions = 0usize;
    match policy {
        MergeObservationPolicy::KeepBoth => base.extend(incoming),
        MergeObservationPolicy::PreferPrimary => {
            for obs in incoming {
                let k = key(&obs);
                if base.iter().any(|o| key(o) == k) {
                    collisions += 1;
                    continue;
                }
                base.push(obs);
            }
        }
        MergeObservationPolicy::PreferSecondary => {
            for obs in incoming {
                let k = key(&obs);
                if base.iter().any(|o| key(o) == k) {
                    collisions += 1;
                    base.retain(|o| key(o) != k);
                }
                base.push(obs);
            }
        }
    }
    collisions
}

pub(crate) fn merge_resume_observations(base: &mut Vec<Observation>, incoming: Vec<Observation>) {
    merge_observations_into(base, incoming, MergeObservationPolicy::KeepBoth);
}

/// Which record in `--primary` a record in `--secondary` corresponds to: `(sfen, source.path,
/// source.ply)`, not bare `sfen` alone -- bare `sfen` would wrongly conflate two different
/// games/plies that happen to reach an identical position (common in early openings). This
/// triple already uniquely identifies a specific extracted occurrence for every extractor in
/// this codebase and survives unchanged through `label`/`select`/`filter`.
pub(crate) fn merge_alignment_key(rec: &PositionRecord) -> (String, String, u32) {
    (rec.sfen.clone(), rec.source.path.clone(), rec.source.ply)
}

/// `label --resume-from`'s index: alignment key -> byte offset of that record's JSON line in the
/// resume file. Why: resume files can be multi-GB after an interrupted labeling run. Keeping
/// every resumed `PositionRecord` in memory (its full `Vec<Observation>`, PV lines, etc.) defeats
/// `label`'s bounded streaming pipeline, so the index stores only lookup keys and file offsets;
/// matching records are re-read from disk (one seek + one line) only when `--input` actually
/// produces that key, via `read_resume_observations_at`.
///
/// Only `sfen`/`source.path`/`source.ply` are parsed per line -- not the full record -- so
/// indexing never allocates a resumed record's observations at all. A later occurrence of the
/// same key overwrites an earlier one's offset, matching `HashMap::collect`'s prior "last wins"
/// behavior when multiple lines share a key.
///
/// ponytail: this still holds one key (two `String`s + a `u32`) per resumed record in memory, so
/// it's a large constant-factor win over the old full-record map, not O(1). A truly bounded
/// approach would need a sorted merge-join between `--input` and `--resume-from`, which isn't
/// available here -- neither file is guaranteed sorted by alignment key (a prior `label` run's
/// writer can emit records out of order). Sharded index files would only trade this map's memory
/// for disk I/O without addressing that same limit, so they're not worth the added complexity
/// here. Revisit only if the offset index itself becomes the memory ceiling in practice.
pub(crate) type ResumeIndex = HashMap<(String, String, u32), u64>;

pub(crate) fn build_resume_index(path: &Path) -> Result<ResumeIndex> {
    #[derive(Deserialize)]
    struct KeyOnly {
        sfen: String,
        source: KeyOnlySource,
    }
    #[derive(Deserialize)]
    struct KeyOnlySource {
        path: String,
        ply: u32,
    }

    let file = File::open(path).with_context(|| format!("cannot open {path:?}"))?;
    let mut reader = BufReader::new(file);
    let mut index = HashMap::new();
    let mut offset = 0u64;
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .with_context(|| format!("cannot read {path:?}"))?;
        if n == 0 {
            break;
        }
        let line_offset = offset;
        offset += n as u64;
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<KeyOnly>(line) {
            Ok(rec) => {
                index.insert((rec.sfen, rec.source.path, rec.source.ply), line_offset);
            }
            Err(e) => {
                tracing::warn!("resume-from parse error at offset {line_offset}: {e}");
            }
        }
    }
    Ok(index)
}

/// Re-reads one resumed record's observations from `--resume-from`, given the byte offset
/// `build_resume_index` recorded for its alignment key. A fresh `BufReader` per lookup (rather
/// than one kept across calls) is deliberate: every call first seeks the underlying file to an
/// absolute offset, so any of a previous lookup's read-ahead buffering is simply discarded, never
/// a correctness concern.
pub(crate) fn read_resume_observations_at(
    file: &mut File,
    offset: u64,
) -> Result<Vec<Observation>> {
    file.seek(SeekFrom::Start(offset))
        .context("cannot seek resume-from file")?;
    let mut line = String::new();
    BufReader::new(file)
        .read_line(&mut line)
        .context("cannot read resume-from file")?;
    let record = parse_json_line(line.trim()).context("cannot parse resume-from record")?;
    Ok(record.observations)
}

pub(crate) fn cmd_merge_observations(args: MergeObservationsArgs) -> Result<()> {
    let policy = match args.on_collision.as_str() {
        "keep-both" => MergeObservationPolicy::KeepBoth,
        "prefer-primary" => MergeObservationPolicy::PreferPrimary,
        "prefer-secondary" => MergeObservationPolicy::PreferSecondary,
        _ => unreachable!("clap's value_parser already restricts --on-collision"),
    };

    let (secondary_records, _) = load_records(&args.secondary)?;
    // Last-wins on an internally-duplicated secondary key -- a documented limitation, not worth
    // extra bookkeeping for an edge case the real "aligned relabel pass" use case won't hit.
    let secondary_by_key: HashMap<(String, String, u32), usize> = secondary_records
        .iter()
        .enumerate()
        .map(|(i, r)| (merge_alignment_key(r), i))
        .collect();
    let mut consumed = vec![false; secondary_records.len()];

    let (primary_records, _) = load_records(&args.primary)?;
    let mut writer = AtomicWriter::new(&args.out)?;
    let (mut both, mut primary_only, mut collisions) = (0usize, 0usize, 0usize);

    for mut rec in primary_records {
        if let Some(&idx) = secondary_by_key.get(&merge_alignment_key(&rec)) {
            consumed[idx] = true;
            both += 1;
            collisions += merge_observations_into(
                &mut rec.observations,
                secondary_records[idx].observations.clone(),
                policy,
            );
            // Stale after merging in observations the pre-merge stability wasn't computed from --
            // a stability score reflecting only one side's observations would silently
            // misrepresent the merged set. Re-run `stability` after merging if wanted.
            rec.stability = None;
        } else {
            primary_only += 1;
        }
        serde_json::to_writer(&mut writer, &rec)?;
        writer.write_all(b"\n")?;
    }

    let mut secondary_only = 0usize;
    for (idx, rec) in secondary_records.into_iter().enumerate() {
        if !consumed[idx] {
            serde_json::to_writer(&mut writer, &rec)?;
            writer.write_all(b"\n")?;
            secondary_only += 1;
        }
    }
    writer.commit()?;

    eprintln!(
        "done: {} written ({both} merged, {primary_only} primary-only, {secondary_only} \
         secondary-only, {collisions} colliding observation keys resolved by --on-collision {}) \
         → {:?}",
        both + primary_only + secondary_only,
        args.on_collision,
        args.out
    );
    Ok(())
}

#[cfg(test)]
mod resume_index_tests {
    use super::*;

    fn labeled_line(sfen: &str, path: &str, ply: u32, bestmove: &str) -> String {
        let rec = serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "sfen": sfen,
            "source": {"kind": "test", "path": path, "ply": ply},
            "tags": {"phase": "opening", "side_to_move": "black", "in_check": false, "has_capture": false},
            "observations": [{
                "engine": "test", "depth": 4, "score": {"kind": "cp", "value": 10},
                "bestmove": bestmove,
            }],
        });
        rec.to_string()
    }

    #[test]
    fn index_maps_keys_to_the_offset_of_their_line_and_a_later_duplicate_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("resume.jsonl");
        let lines = [
            labeled_line("sfen-a", "g.csa", 1, "7g7f"),
            labeled_line("sfen-b", "g.csa", 2, "3c3d"),
            // Same key as the first line, later in the file, with a different bestmove --
            // the index must resolve to *this* occurrence, not the first.
            labeled_line("sfen-a", "g.csa", 1, "2g2f"),
        ];
        fs::write(&path, lines.join("\n") + "\n").unwrap();

        let index = build_resume_index(&path).unwrap();
        assert_eq!(index.len(), 2, "two distinct keys, despite three lines");

        let mut file = File::open(&path).unwrap();
        let offset_a = index[&("sfen-a".to_string(), "g.csa".to_string(), 1)];
        let obs_a = read_resume_observations_at(&mut file, offset_a).unwrap();
        assert_eq!(
            obs_a[0].bestmove, "2g2f",
            "must resolve to the later (last-wins) occurrence of a duplicated key"
        );

        let offset_b = index[&("sfen-b".to_string(), "g.csa".to_string(), 2)];
        let obs_b = read_resume_observations_at(&mut file, offset_b).unwrap();
        assert_eq!(obs_b[0].bestmove, "3c3d");
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
mod merge_observations_tests {
    use super::*;

    fn obs(engine: &str, depth: u32, requested_depth: Option<u32>, bestmove: &str) -> Observation {
        Observation {
            engine: engine.to_string(),
            engine_version: None,
            depth,
            requested_depth,
            requested_nodes: None,
            search_limit_kind: SearchLimitKind::Depth,
            score: Score::Cp { value: 0 },
            score_perspective: ScorePerspective::default(),
            score_bound: shogiesa_core::ScoreBound::default(),
            bestmove: bestmove.to_string(),
            bestmove_kind: None,
            nodes: None,
            time_ms: None,
            seldepth: None,
            nps: None,
            hashfull: None,
            pv: None,
            policy_margin_cp: None,
            candidates: Vec::new(),
            engine_options_hash: None,
            weight_sha256: None,
            was_timeout_salvaged: false,
        }
    }

    fn nodes_obs(engine: &str, depth: u32, requested_nodes: u64, bestmove: &str) -> Observation {
        Observation {
            requested_depth: None,
            requested_nodes: Some(requested_nodes),
            search_limit_kind: SearchLimitKind::Nodes,
            ..obs(engine, depth, None, bestmove)
        }
    }

    #[test]
    fn no_collision_appends_both() {
        let mut base = vec![obs("e1", 4, Some(4), "7g7f")];
        let incoming = vec![obs("e2", 4, Some(4), "3c3d")];
        let collisions =
            merge_observations_into(&mut base, incoming, MergeObservationPolicy::KeepBoth);
        assert_eq!(collisions, 0);
        assert_eq!(base.len(), 2);
    }

    #[test]
    fn keep_both_on_collision_keeps_both() {
        let mut base = vec![obs("e1", 4, Some(4), "7g7f")];
        let incoming = vec![obs("e1", 4, Some(4), "3c3d")]; // same (engine, depth, requested_depth)
        let collisions =
            merge_observations_into(&mut base, incoming, MergeObservationPolicy::KeepBoth);
        assert_eq!(collisions, 0); // KeepBoth never even checks for a collision
        assert_eq!(base.len(), 2);
    }

    #[test]
    fn prefer_primary_on_collision_drops_secondary() {
        let mut base = vec![obs("e1", 4, Some(4), "7g7f")];
        let incoming = vec![obs("e1", 4, Some(4), "3c3d")];
        let collisions =
            merge_observations_into(&mut base, incoming, MergeObservationPolicy::PreferPrimary);
        assert_eq!(collisions, 1);
        assert_eq!(base.len(), 1);
        assert_eq!(base[0].bestmove, "7g7f"); // primary's survives
    }

    #[test]
    fn prefer_secondary_on_collision_replaces_primary() {
        let mut base = vec![obs("e1", 4, Some(4), "7g7f")];
        let incoming = vec![obs("e1", 4, Some(4), "3c3d")];
        let collisions =
            merge_observations_into(&mut base, incoming, MergeObservationPolicy::PreferSecondary);
        assert_eq!(collisions, 1);
        assert_eq!(base.len(), 1);
        assert_eq!(base[0].bestmove, "3c3d"); // secondary's replaces primary's
    }

    #[test]
    fn multiple_simultaneous_collisions_all_resolved() {
        let mut base = vec![obs("e1", 4, Some(4), "7g7f"), obs("e2", 6, Some(6), "3c3d")];
        let incoming = vec![
            obs("e1", 4, Some(4), "aaaa"), // collides with base[0]
            obs("e2", 6, Some(6), "bbbb"), // collides with base[1]
            obs("e3", 8, Some(8), "cccc"), // no collision
        ];
        let collisions =
            merge_observations_into(&mut base, incoming, MergeObservationPolicy::PreferSecondary);
        assert_eq!(collisions, 2);
        assert_eq!(base.len(), 3);
        assert!(base.iter().any(|o| o.bestmove == "aaaa"));
        assert!(base.iter().any(|o| o.bestmove == "bbbb"));
        assert!(base.iter().any(|o| o.bestmove == "cccc"));
    }

    #[test]
    fn engine_version_mismatch_is_not_a_collision() {
        // merge-observations' key includes engine_version (unlike label's own narrower in-place
        // dedup key) precisely so two different engine versions at the same nominal depth are
        // never silently conflated.
        let mut base = vec![obs("e1", 4, Some(4), "7g7f")];
        let mut versioned = obs("e1", 4, Some(4), "3c3d");
        versioned.engine_version = Some("2.0".to_string());
        let collisions = merge_observations_into(
            &mut base,
            vec![versioned],
            MergeObservationPolicy::PreferPrimary,
        );
        assert_eq!(collisions, 0);
        assert_eq!(base.len(), 2);
    }

    #[test]
    fn shallow_and_deep_same_engine_never_collide_under_any_policy() {
        // The flagship use case this command exists for -- a shallow pass (depth 4) plus a
        // deeper relabel (depth 12) of the same engine -- has DIFFERENT depths, so it never
        // collides on (engine, engine_version, depth, requested_depth) no matter which
        // --on-collision policy is chosen: both observations always survive. `--on-collision`
        // only ever matters when two passes produced the exact same (engine, engine_version,
        // depth, requested_depth) tuple (e.g. a flaky re-run at the identical depth) -- it is
        // NOT a "the deeper one wins" switch, since depth is part of the key that determines
        // whether there's a collision to resolve in the first place.
        for policy in [
            MergeObservationPolicy::KeepBoth,
            MergeObservationPolicy::PreferPrimary,
            MergeObservationPolicy::PreferSecondary,
        ] {
            let mut base = vec![obs("e1", 4, Some(4), "7g7f")];
            let deep = vec![obs("e1", 12, Some(12), "3c3d")];
            let collisions = merge_observations_into(&mut base, deep, policy);
            assert_eq!(collisions, 0);
            assert_eq!(base.len(), 2, "both depths must survive under every policy");
        }
    }

    #[test]
    fn nodes_mode_observations_with_different_requested_nodes_never_collide() {
        // Regression test: every node-limited observation has requested_depth: None, so the old
        // key (engine, engine_version, depth, requested_depth) collapsed any two node-limited
        // observations from the same engine that happened to reach the same achieved depth --
        // even with completely different requested_nodes -- silently dropping one under
        // PreferPrimary/PreferSecondary. search_limit_kind/requested_nodes in the key fixes this.
        let mut base = vec![nodes_obs("e1", 8, 50_000, "7g7f")];
        let incoming = vec![nodes_obs("e1", 8, 200_000, "3c3d")]; // same achieved depth, different budget
        let collisions =
            merge_observations_into(&mut base, incoming, MergeObservationPolicy::PreferPrimary);
        assert_eq!(collisions, 0, "different requested_nodes must not collide");
        assert_eq!(base.len(), 2, "both node-limited observations must survive");
    }
}
