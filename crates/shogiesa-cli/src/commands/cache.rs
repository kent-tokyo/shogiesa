use super::super::*;
use super::label::{CacheRead, parse_cache_entry};

/// Every `.json` file under a `label --cache-dir`'s two-level shard layout
/// (`cache_dir/xx/<64-hex-hash>.json`, see `label_cache_path`). Filtering strictly on the `.json`
/// extension skips `write_cache_entry_atomically`'s in-flight temp files
/// (`<hash>.json.tmp.<pid>.<tid>`) by construction, so a walk never races a concurrent `label`
/// process sharing the same cache dir.
fn walk_cache_entries(cache_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = Vec::new();
    for shard in fs::read_dir(cache_dir).with_context(|| format!("cannot read {cache_dir:?}"))? {
        let shard_path = shard?.path();
        if !shard_path.is_dir() {
            continue;
        }
        for entry in
            fs::read_dir(&shard_path).with_context(|| format!("cannot read {shard_path:?}"))?
        {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                entries.push(path);
            }
        }
    }
    Ok(entries)
}

pub(crate) fn cmd_cache_stats(args: CacheStatsArgs) -> Result<()> {
    let entries = walk_cache_entries(&args.cache_dir)?;
    let now = std::time::SystemTime::now();
    let mut total_bytes = 0u64;
    let mut oldest: Option<std::time::SystemTime> = None;
    let mut newest: Option<std::time::SystemTime> = None;
    // Parsed straight from each entry's `Observation.engine` field -- present in both v1 and v2
    // payloads, not inferred from the (otherwise opaque, one-way-hashed) cache key/path.
    let mut engine_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut legacy_v1_count = 0usize;
    // These four distributions only exist for v2 entries -- a v1 payload is a bare `Observation`
    // with no schema_version/engine_fingerprint/requested_depth/multipv of its own to report.
    let mut schema_version_counts: BTreeMap<u32, usize> = BTreeMap::new();
    let mut fingerprint_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut requested_depth_counts: BTreeMap<u32, usize> = BTreeMap::new();
    let mut multipv_counts: BTreeMap<u32, usize> = BTreeMap::new();
    let mut search_limit_kind_counts: BTreeMap<&'static str, usize> = BTreeMap::new();

    for path in &entries {
        if let Ok(meta) = fs::metadata(path) {
            total_bytes += meta.len();
            if let Ok(modified) = meta.modified() {
                oldest = Some(oldest.map_or(modified, |o| o.min(modified)));
                newest = Some(newest.map_or(modified, |n| n.max(modified)));
            }
        }
        if let Some(parsed) = fs::read_to_string(path)
            .ok()
            .and_then(|s| parse_cache_entry(&s))
        {
            *engine_counts
                .entry(parsed.observation().engine.clone())
                .or_default() += 1;
            match &parsed {
                CacheRead::V1(_) => legacy_v1_count += 1,
                CacheRead::V2(entry) => {
                    *schema_version_counts
                        .entry(entry.schema_version)
                        .or_default() += 1;
                    let fingerprint_key = entry
                        .engine_fingerprint
                        .map(|fp| format!("{fp:016x}"))
                        .unwrap_or_else(|| "none".to_string());
                    *fingerprint_counts.entry(fingerprint_key).or_default() += 1;
                    *requested_depth_counts
                        .entry(entry.requested_depth)
                        .or_default() += 1;
                    *multipv_counts.entry(entry.multipv).or_default() += 1;
                    let kind_key = match entry.search_limit_kind {
                        SearchLimitKind::Depth => "depth",
                        SearchLimitKind::Nodes => "nodes",
                    };
                    *search_limit_kind_counts.entry(kind_key).or_default() += 1;
                }
            }
        }
    }

    println!("cache entries : {}", entries.len());
    println!("total size    : {total_bytes} bytes");
    if let Some(oldest) = oldest {
        let days = now.duration_since(oldest).unwrap_or_default().as_secs() / 86400;
        println!("oldest entry  : {days} days old");
    }
    if let Some(newest) = newest {
        let days = now.duration_since(newest).unwrap_or_default().as_secs() / 86400;
        println!("newest entry  : {days} days old");
    }
    if !engine_counts.is_empty() {
        println!("engine distribution:");
        for (engine, count) in &engine_counts {
            println!("  {engine:<20} {count:>6}");
        }
    }
    if legacy_v1_count > 0 {
        println!("legacy (v1, no metadata): {legacy_v1_count} entries");
    }
    if !schema_version_counts.is_empty() {
        println!("schema_version distribution (v2 entries only):");
        for (v, count) in &schema_version_counts {
            println!("  {v:<20} {count:>6}");
        }
    }
    if !fingerprint_counts.is_empty() {
        println!("engine_fingerprint distribution (v2 entries only):");
        for (fp, count) in &fingerprint_counts {
            println!("  {fp:<20} {count:>6}");
        }
    }
    if !requested_depth_counts.is_empty() {
        println!("requested_depth distribution (v2 entries only):");
        for (d, count) in &requested_depth_counts {
            println!("  {d:<20} {count:>6}");
        }
    }
    if !multipv_counts.is_empty() {
        println!("multipv distribution (v2 entries only):");
        for (m, count) in &multipv_counts {
            println!("  {m:<20} {count:>6}");
        }
    }
    if !search_limit_kind_counts.is_empty() {
        println!("search_limit_kind distribution (v2 entries only):");
        for (k, count) in &search_limit_kind_counts {
            println!("  {k:<20} {count:>6}");
        }
    }
    Ok(())
}

pub(crate) fn cmd_cache_verify(args: CacheVerifyArgs) -> Result<()> {
    let entries = walk_cache_entries(&args.cache_dir)?;
    let mut corrupted = 0usize;
    let mut legacy_v1_count = 0usize;
    let mut schema_version_counts: BTreeMap<u32, usize> = BTreeMap::new();

    for path in &entries {
        match fs::read_to_string(path)
            .ok()
            .and_then(|s| parse_cache_entry(&s))
        {
            Some(CacheRead::V1(_)) => legacy_v1_count += 1,
            Some(CacheRead::V2(entry)) => {
                *schema_version_counts
                    .entry(entry.schema_version)
                    .or_default() += 1;
            }
            None => {
                corrupted += 1;
                tracing::warn!(path = %path.display(), "corrupted cache entry");
            }
        }
    }

    println!("cache entries : {}", entries.len());
    println!("corrupted     : {corrupted}");
    if legacy_v1_count > 0 {
        println!("legacy (v1, no metadata): {legacy_v1_count} entries");
    }
    if !schema_version_counts.is_empty() {
        println!("schema_version distribution (v2 entries only):");
        for (v, count) in &schema_version_counts {
            println!("  {v:<20} {count:>6}");
        }
    }
    // Deliberately not claiming a LIVE staleness check ("does this entry match today's engine/
    // schema"): label_cache_path already folds SCHEMA_VERSION and the engine fingerprint into the
    // (one-way) cache key hash, so a schema bump or a different engine binary simply produces a
    // different key -- a stale entry is never wrongly returned as a hit, it's just orphaned dead
    // weight. A true live check would need this command to also take --engine/
    // --engine-fingerprint-mode to recompute today's fingerprint and compare -- a real but
    // separate feature, not built here; v1 legacy entries have no metadata to check against in
    // the first place.
    println!(
        "note: v1 (legacy) entries store no schema_version/engine_fingerprint metadata; v2 \
         entries do (see the distribution above). Neither format supports a live \"does this \
         match today's engine/schema\" check here -- a schema bump or engine change already \
         changes future cache keys by construction (see label_cache_path), so a stale entry is \
         never wrongly reused, just orphaned."
    );
    Ok(())
}

/// The file's age (now minus mtime), or `None` if either the metadata/mtime read fails or the
/// clock has gone backwards -- either way, "unknown age" should never count as "old enough to
/// prune."
fn file_age(path: &Path, now: std::time::SystemTime) -> Option<std::time::Duration> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    now.duration_since(modified).ok()
}

pub(crate) fn cmd_cache_prune(args: CachePruneArgs) -> Result<()> {
    if !args.corrupted_only && !args.legacy_only && args.older_than_days.is_none() {
        anyhow::bail!(
            "cache prune requires --corrupted-only, --legacy-only, and/or --older-than-days"
        );
    }
    let entries = walk_cache_entries(&args.cache_dir)?;
    let now = std::time::SystemTime::now();
    let max_age = args
        .older_than_days
        .map(|days| std::time::Duration::from_secs(days * 86400));

    let mut to_delete = Vec::new();
    for path in &entries {
        let mut matched = false;
        if args.corrupted_only || args.legacy_only {
            let parsed = fs::read_to_string(path)
                .ok()
                .and_then(|s| parse_cache_entry(&s));
            match &parsed {
                None if args.corrupted_only => matched = true,
                Some(CacheRead::V1(_)) if args.legacy_only => matched = true,
                _ => {}
            }
        }
        if let Some(max_age) = max_age
            && file_age(path, now).is_some_and(|age| age >= max_age)
        {
            matched = true;
        }
        if matched {
            to_delete.push(path.clone());
        }
    }

    if args.yes {
        let mut deleted = 0usize;
        for path in &to_delete {
            match fs::remove_file(path) {
                Ok(()) => deleted += 1,
                Err(e) => tracing::warn!(path = %path.display(), "failed to delete: {e}"),
            }
        }
        eprintln!(
            "deleted {deleted}/{} matched entries ({} total)",
            to_delete.len(),
            entries.len()
        );
    } else {
        eprintln!(
            "dry run: {}/{} entries matched and would be deleted (pass --yes to actually delete)",
            to_delete.len(),
            entries.len()
        );
    }
    Ok(())
}
