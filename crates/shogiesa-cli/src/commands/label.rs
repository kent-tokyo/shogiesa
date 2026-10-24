use super::super::*;

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
    /// Which mode produced `engine_fingerprint` -- stored separately from the `Option<u64>` value
    /// itself so a v2 `CacheEntry` can record it (an `Option<u64>` alone can't distinguish "no
    /// fingerprint because mode is `none`" from "no fingerprint because computing it failed").
    engine_fingerprint_mode: EngineFingerprintMode,
    hits: Arc<AtomicUsize>,
    misses: Arc<AtomicUsize>,
}

/// How (if at all) the engine binary itself contributes to the label cache key, on top of its
/// USI-reported `id name`/`id version`. Those strings are controlled by the engine and aren't
/// guaranteed to change after a local rebuild, so relying on them alone risks a cache hit
/// silently reusing labels produced by a different executable.
///
/// Derives `Serialize`/`Deserialize` purely so a v2 `CacheEntry` (below) can store which mode
/// produced its `engine_fingerprint` -- the existing hand-rolled `as_str`/`parse` below (used for
/// CLI arg parsing and the `label --manifest` field) are untouched by this; `serde`'s
/// `rename_all = "snake_case"` happens to already match those same literal strings.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
pub(super) fn hash_parts(parts: &[&[u8]]) -> blake3::Hash {
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
pub(super) fn hash_parts_u64(parts: &[&[u8]]) -> u64 {
    u64::from_le_bytes(hash_parts(parts).as_bytes()[..8].try_into().unwrap())
}

/// Sorts (and borrows) engine option pairs -- shared by `engine_options_hash` (u64, label-cache
/// key) and `engine_options_hash_hex` (full hex digest, cross-repo Observation/RunManifest use)
/// so the two never drift on what counts as "the same options" (sort order, and whether the
/// CLI-synthesized MultiPV entry is included -- it is, since it's already in `engine_options` by
/// the time either function is called).
fn sorted_engine_option_bytes(options: &[(String, String)]) -> Vec<&[u8]> {
    let mut sorted: Vec<&(String, String)> = options.iter().collect();
    sorted.sort();
    let mut parts: Vec<&[u8]> = Vec::with_capacity(sorted.len() * 2);
    for (k, v) in &sorted {
        parts.push(k.as_bytes());
        parts.push(v.as_bytes());
    }
    parts
}

/// Hash of the resolved USI engine options, sorted so option order doesn't change the label
/// cache key below.
fn engine_options_hash(options: &[(String, String)]) -> u64 {
    hash_parts_u64(&sorted_engine_option_bytes(options))
}

/// Full (untruncated) blake3 hex digest of the same input `engine_options_hash` hashes.
/// `engine_options_hash` (u64) stays truncated for the on-disk cache key/`CacheEntry`, which
/// never leaves this process; this hex form is for `Observation.engine_options_hash`/
/// `RunManifest`, which do leave it (shogiesa -> quietset -> lineprior -> veridict) -- a
/// truncated u64 exceeds 2^53 almost always and a JS/TS consumer's `JSON.parse` would silently
/// mangle it as an IEEE-754 double.
fn engine_options_hash_hex(options: &[(String, String)]) -> String {
    hash_parts(&sorted_engine_option_bytes(options))
        .to_hex()
        .to_string()
}

/// SHA-256 of `--weight-file`'s bytes, for `weight_sha256` -- errors clearly if given a
/// directory, since the envelope field is defined over one file's bytes.
fn compute_weight_sha256(path: &Path) -> Result<String> {
    if path.is_dir() {
        anyhow::bail!("weight_sha256 requires a single file, got a directory: {path:?}");
    }
    hash_file_sha256(path)
}

/// SHA-256 of the engine binary's raw bytes, for `RunManifest.binary_sha256`. Deliberately a
/// separate `fs::read` from `compute_engine_fingerprint`'s own Content-mode read (a different,
/// blake3-truncated value for an unrelated purpose -- the label cache key) rather than threading
/// bytes out of that function: engine binaries are small and this runs once per `label`
/// invocation, not per position, so the extra read is negligible next to actual engine search
/// cost. Same graceful warn-and-None fallback as `compute_engine_fingerprint` for an unreadable
/// (e.g. bare PATH-resolved) path.
fn compute_binary_sha256(engine_path: &Path) -> Option<String> {
    match fs::read(engine_path) {
        Ok(bytes) => Some(sha256_hex(&bytes)),
        Err(e) => {
            tracing::warn!(
                engine = %engine_path.display(),
                "cannot read engine binary for binary_sha256 ({e})"
            );
            None
        }
    }
}

/// Case-insensitive lookup of one USI engine option's value among `--engine-option` pairs,
/// parsed as `u32` -- for `RunManifest.engine_threads`/`engine_hash_mb`. `Threads`/`Hash` aren't
/// dedicated CLI flags in this codebase; they're conventionally passed through the generic
/// `--engine-option Threads=N --engine-option Hash=M` mechanism like any other USI option.
fn engine_option_u32(options: &[(String, String)], key: &str) -> Option<u32> {
    options
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .and_then(|(_, v)| v.parse().ok())
}

/// Versions the on-disk cache-file *envelope* shape, distinct from `SCHEMA_VERSION` (which
/// versions the JSONL/pack `Observation` data model -- the two change independently: an envelope
/// shape change doesn't imply the `Observation` payload inside it changed, and vice versa).
/// Deliberately does NOT participate in `label_cache_path`'s hash below: an old (v1) file at a
/// given key is still a byte-truthful cache hit for that key's `Observation`, just read through
/// the legacy branch in `parse_cache_entry` -- envelope format is a read-time parsing concern, not
/// a cache-validity concern. `SCHEMA_VERSION` already changes the key (and therefore the file) by
/// construction when it bumps; this constant only needs to change if `CacheEntry`'s own field set
/// changes shape in a future round.
const CACHE_SCHEMA_VERSION: u32 = 1;

/// A cached label, plus the metadata that produced it. Introduced in this round specifically so
/// `cache stats`/`verify`/`prune` can report real distributions (which schema/engine/fingerprint/
/// depth/multipv a cache dir's entries were written under) instead of only corruption/size/age --
/// the cache *key* already encodes all of this (see `label_cache_path`), but a key is a one-way
/// hash: there's no way to recover "what schema version was this?" from the filename alone. Storing
/// it in the payload too costs nothing at write time and unlocks introspection at read time.
#[derive(Serialize, Deserialize)]
pub(super) struct CacheEntry {
    cache_schema_version: u32,
    /// Unix epoch seconds (`SystemTime::now()`). No chrono/time/humantime dependency exists
    /// anywhere in this workspace (`time` is pulled in only transitively via the `csa` crate) --
    /// adding one solely to format a timestamp string isn't justified when a plain integer works.
    created_at: u64,
    pub(super) schema_version: u32,
    engine_name: String,
    engine_version: Option<String>,
    pub(super) engine_fingerprint: Option<u64>,
    engine_fingerprint_mode: EngineFingerprintMode,
    engine_options_hash: u64,
    pub(super) requested_depth: u32,
    /// Additive since the fixed-node-search round, not a rename of `requested_depth` (which
    /// stays `0` for a node-limited entry, same as before this field existed) -- a rename would
    /// make every pre-existing v2 cache entry fail `CacheEntry`'s parse and get misreported as
    /// corrupted by `cache verify` rather than merely predating this field.
    #[serde(default)]
    pub(super) search_limit_kind: SearchLimitKind,
    /// The depth (as u64) or node count this entry's search was limited to, mirroring whichever
    /// of `requested_depth`/`search_limit_kind` applies. `0` for pre-existing entries (defaults
    /// to `Depth`/`0`, exactly what they always were).
    #[serde(default)]
    search_limit_value: u64,
    pub(super) multipv: u32,
    observation: Observation,
}

/// Parsed form of one cache file's JSON, regardless of which format wrote it. v2 (`CacheEntry`) is
/// tried first; v1 (a bare `Observation`, this cache format's shape before this round) is the
/// fallback. A v1 entry is old-format, not corrupted, and must never be misreported as corrupted
/// by `cache verify` -- this is the one place every reader (the cache-hit path in
/// `analyze_record`, and `cache stats`/`verify`/`prune`) goes through, so "v1 vs v2 vs actually
/// corrupted" can't drift into disagreeing answers across those call sites.
pub(super) enum CacheRead {
    V2(CacheEntry),
    V1(Observation),
}

impl CacheRead {
    pub(super) fn observation(&self) -> &Observation {
        match self {
            Self::V2(entry) => &entry.observation,
            Self::V1(obs) => obs,
        }
    }

    fn into_observation(self) -> Observation {
        match self {
            Self::V2(entry) => entry.observation,
            Self::V1(obs) => obs,
        }
    }
}

/// v2 tried first since it's the active write format (most reads will be a v2 hit) -- not because
/// the reverse order would misparse: `CacheEntry`'s required fields (`cache_schema_version`,
/// `observation`, ...) and `Observation`'s (`engine`, `depth`, `score`, `bestmove`, ...) don't
/// overlap, so each format's JSON object correctly fails the other's shape and falls through
/// either way (verified directly: swapping this order doesn't break any test).
pub(super) fn parse_cache_entry(json: &str) -> Option<CacheRead> {
    if let Ok(entry) = serde_json::from_str::<CacheEntry>(json) {
        return Some(CacheRead::V2(entry));
    }
    serde_json::from_str::<Observation>(json)
        .ok()
        .map(CacheRead::V1)
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
    limit: SearchLimit,
) -> PathBuf {
    // Why an explicit discriminant byte for `engine_version`, not a sentinel string: `None` must
    // never collide with `Some("")` (or any other literal an engine could plausibly report).
    let (version_tag, version_bytes): (&[u8], &[u8]) = match engine_version {
        Some(v) => (&[1], v.as_bytes()),
        None => (&[0], &[]),
    };
    let engine_options_hash_bytes = cache.engine_options_hash.to_le_bytes();
    // A kind discriminant byte, not just the numeric value -- without it, `--depths 8` and
    // `--nodes 8` would hash to the same cache key despite meaning completely different searches.
    let (limit_kind_tag, limit_value_bytes): (&[u8], [u8; 8]) = match limit {
        SearchLimit::Depth(d) => (&[0], (d as u64).to_le_bytes()),
        SearchLimit::Nodes(n) => (&[1], n.to_le_bytes()),
    };
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
        limit_kind_tag,
        &limit_value_bytes,
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

/// `--usi-strict`/`--restart-engine-every`/`--restart-on-protocol-error`/`--transcript-on-error`,
/// bundled since they're all per-run-constant USI-lifecycle config a worker needs on every call --
/// same bundling rationale as `LabelContext`, which this is embedded into.
#[derive(Clone, Default)]
struct UsiStrictConfig {
    strict: bool,
    restart_on_protocol_error: bool,
    restart_engine_every: Option<u32>,
    transcript_dir: Option<PathBuf>,
}

/// Per-run values every worker needs but that never change across positions -- bundled so
/// `analyze_record`'s signature doesn't grow one parameter per envelope field.
#[derive(Clone)]
struct LabelContext {
    weight_sha256: Option<String>,
    engine_options_hash_hex: String,
    // Bundled here (rather than as separate analyze_record parameters) partly for the same
    // per-run-constant reasoning as the two fields above, and partly to keep analyze_record's
    // own parameter count under clippy's too_many_arguments threshold.
    timeout_ms: u64,
    multipv: u32,
    existing_policy: ExistingPolicy,
    strict_cfg: UsiStrictConfig,
}

/// Per-worker mutable state that survives across jobs on the same thread (unlike `LabelContext`,
/// which is per-run-constant) -- `worker_id` for transcript filenames/logging, and a counter
/// `--restart-engine-every` compares against. Reset to 0 whenever the worker actually relaunches
/// its engine (including the very first launch).
struct WorkerState {
    worker_id: usize,
    searches_since_restart: u32,
}

/// Shared atomics every worker increments as it processes jobs -- bundled so `analyze_record`'s
/// parameter count doesn't grow one atomic per counter.
struct RunCounters {
    timeout_salvaged: AtomicUsize,
    protocol_violations: AtomicUsize,
    engine_restarts: AtomicUsize,
}

/// Short tag identifying which `UsiError` variant fired, used only for transcript filenames and
/// isn't meant to duplicate `UsiError`'s own `Display` (which is a full sentence).
fn error_kind_label(e: &UsiError) -> &'static str {
    match e {
        UsiError::Io(_) => "io",
        UsiError::Timeout => "timeout",
        UsiError::InvalidResponse => "invalid_response",
        UsiError::NoBestmove => "no_bestmove",
        UsiError::BestmoveWithoutGo => "bestmove_without_go",
        UsiError::DuplicateBestmove => "duplicate_bestmove",
        UsiError::DelayedBestmoveAfterTimeout => "delayed_bestmove_after_timeout",
        UsiError::IllegalBestmove { .. } => "illegal_bestmove",
    }
}

/// Dumps `engine`'s captured transcript (raw USI lines since the last clean exchange) to
/// `{nanos}_worker{worker_id}_{kind}.log` under `dir` -- best-effort, since a transcript is a
/// debugging aid, not something a failed write should itself escalate into a run failure.
fn dump_transcript(dir: &Path, engine: &UsiEngine, worker_id: usize, e: &UsiError) {
    // Nanoseconds, not milliseconds: a burst of same-kind violations on one worker (exactly the
    // case a post-mortem most cares about) can otherwise collide on the same filename within a
    // millisecond and silently overwrite each other.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let kind = error_kind_label(e);
    let _ = fs::create_dir_all(dir);
    let _ = fs::write(
        dir.join(format!("{nanos}_worker{worker_id}_{kind}.log")),
        engine.transcript().join("\n"),
    );
}

/// Handles one search error (either a real `UsiError` from `engine.analyse()`, or a synthesized
/// `UsiError::IllegalBestmove` for an otherwise-`Ok` result whose bestmove failed
/// `is_legal_bestmove_lite`) -- logs it, counts it as a protocol violation (excluding `Io`/
/// `Timeout`/`InvalidResponse`/`NoBestmove`, which predate --usi-strict and aren't protocol
/// violations in that sense), dumps a transcript if `--transcript-on-error` is set, and reports
/// whether the caller must discard the current engine and relaunch: unconditional for `Io` (the
/// pipe itself is broken, nothing to reuse), otherwise only under `--restart-on-protocol-error`
/// (a `Timeout` alone can just mean `--timeout-ms` is too tight, not proof of a dead engine).
fn handle_analyse_error(
    e: &UsiError,
    limit: SearchLimit,
    engine: &UsiEngine,
    strict_cfg: &UsiStrictConfig,
    worker_id: usize,
    counters: &RunCounters,
) -> bool {
    tracing::warn!(worker_id, ?limit, "analysis error: {e}");
    let dead = matches!(e, UsiError::Io(_));
    let is_protocol_violation = matches!(
        e,
        UsiError::BestmoveWithoutGo
            | UsiError::DuplicateBestmove
            | UsiError::DelayedBestmoveAfterTimeout
            | UsiError::IllegalBestmove { .. }
    );
    if is_protocol_violation {
        counters.protocol_violations.fetch_add(1, Ordering::Relaxed);
    }
    if let Some(dir) = &strict_cfg.transcript_dir {
        dump_transcript(dir, engine, worker_id, e);
    }
    dead || strict_cfg.restart_on_protocol_error
}

/// Whether `o` (from `engine_name`) already covers a fresh request for `limit`, i.e. `label
/// --skip-existing` can skip re-running it. Depth mode: achieved depth >= requested (unchanged
/// existing behavior). Nodes mode: achieved `nodes` (the engine's own reported total, since
/// nodes -- like depth -- is "how much work was done") >= requested. Requires matching
/// `search_limit_kind` in both directions: a depth-limited observation never "covers" a
/// node-limited request or vice versa, even if the raw numbers happen to compare favorably --
/// they aren't comparable notions of search thoroughness across modes.
fn observation_multipv(o: &Observation) -> u32 {
    o.candidates
        .iter()
        .map(|candidate| candidate.multipv)
        .max()
        .unwrap_or(1)
}

/// Whether an observation was produced with the requested MultiPV setting. Older JSONL has no
/// explicit request field, so the candidate ranks are the durable evidence: no candidates means
/// ordinary single-PV labeling, while the highest reported rank identifies the requested MultiPV
/// setting. Exact matching is intentional -- a MultiPV=2 result should not silently satisfy a
/// later MultiPV=3 request (or cause a re-label to be treated as the same slot as single-PV).
fn observation_matches_multipv(o: &Observation, multipv: u32) -> bool {
    observation_multipv(o) == multipv.max(1)
}

fn observation_covers_limit(
    o: &Observation,
    engine_name: &str,
    limit: SearchLimit,
    multipv: u32,
) -> bool {
    if o.engine != engine_name {
        return false;
    }
    if !observation_matches_multipv(o, multipv) {
        return false;
    }
    match limit {
        SearchLimit::Depth(d) => o.search_limit_kind == SearchLimitKind::Depth && o.depth >= d,
        SearchLimit::Nodes(n) => {
            o.search_limit_kind == SearchLimitKind::Nodes
                && o.nodes.is_some_and(|actual| actual >= n)
        }
    }
}

/// Whether `o` (an existing observation) and `new` (freshly produced) are "the same request" for
/// dedup/replace purposes. Depth mode: unchanged existing logic (achieved-depth equality +
/// requested_depth match-or-legacy-absent, so "requested 12, reached 8" and "requested 8, reached
/// 8" stay distinct). Nodes mode: exact `requested_nodes` match, not achieved-nodes equality -- an
/// engine's actual node count for a fixed budget can jitter run-to-run in a way achieved depth
/// typically doesn't, so matching on achieved nodes would systematically under-dedupe (never
/// matching) rather than over-dedupe.
fn same_search_request(o: &Observation, new: &Observation) -> bool {
    o.engine == new.engine
        && observation_multipv(o) == observation_multipv(new)
        && o.search_limit_kind == new.search_limit_kind
        && match new.search_limit_kind {
            SearchLimitKind::Depth => {
                o.depth == new.depth
                    && (o.requested_depth.is_none() || o.requested_depth == new.requested_depth)
            }
            SearchLimitKind::Nodes => o.requested_nodes == new.requested_nodes,
        }
}

/// Runs every `limit` in `limits` against `rec` (cache-first, then a live `engine.analyse()`
/// call), same as before --usi-strict existed, plus: counting a `--usi-strict` illegal-bestmove
/// finding as a search failure instead of trusting it, and reporting via the `bool` return
/// whether the caller must discard `engine` and relaunch (see `handle_analyse_error`). Once that
/// becomes true, remaining limits for this record are skipped -- there's no reason to keep
/// feeding a soon-to-be-replaced engine process more work.
fn analyze_record(
    rec: &mut PositionRecord,
    engine: &mut UsiEngine,
    limits: &[SearchLimit],
    cache: Option<&LabelCache>,
    counters: &RunCounters,
    ctx: &LabelContext,
    worker: &mut WorkerState,
) -> bool {
    let mut restart_needed = false;
    for &limit in limits {
        // The engine may stop before reaching `limit` (e.g. a forced mate) — check coverage
        // against what was actually achieved, not the requested limit, before spending a call.
        if matches!(ctx.existing_policy, ExistingPolicy::Skip)
            && rec
                .observations
                .iter()
                .any(|o| observation_covers_limit(o, &engine.engine_name, limit, ctx.multipv))
        {
            continue;
        }

        let cache_path = cache.map(|c| {
            label_cache_path(
                c,
                &rec.sfen,
                &engine.engine_name,
                engine.engine_version.as_deref(),
                limit,
            )
        });
        let cached: Option<Observation> = cache_path.as_ref().and_then(|path| {
            let hit = fs::read_to_string(path)
                .ok()
                .and_then(|s| parse_cache_entry(&s))
                .map(CacheRead::into_observation);
            if let Some(c) = cache {
                c.hits.fetch_add(hit.is_some() as usize, Ordering::Relaxed);
                c.misses
                    .fetch_add(hit.is_none() as usize, Ordering::Relaxed);
            }
            hit
        });

        let observation = match cached {
            Some(obs) => Some(obs),
            None => {
                worker.searches_since_restart += 1;
                match engine.analyse(&rec.sfen, limit, ctx.timeout_ms) {
                    // `bestmove_kind.is_none()` excludes resign/win/none -- those are special
                    // USI tokens, not board moves, and would otherwise always false-positive as
                    // illegal (they never match any entry in `all_legal_moves_partial`).
                    Ok(result)
                        if ctx.strict_cfg.strict
                            && result.bestmove_kind.is_none()
                            && !commands::is_legal_bestmove_lite(&rec.sfen, &result.bestmove)
                                .unwrap_or(true) =>
                    {
                        let err = UsiError::IllegalBestmove {
                            sfen: rec.sfen.clone(),
                            mv: result.bestmove.clone(),
                        };
                        if handle_analyse_error(
                            &err,
                            limit,
                            engine,
                            &ctx.strict_cfg,
                            worker.worker_id,
                            counters,
                        ) {
                            restart_needed = true;
                        }
                        None
                    }
                    Ok(result) => {
                        if result.timed_out {
                            counters.timeout_salvaged.fetch_add(1, Ordering::Relaxed);
                        }
                        let obs = Observation {
                            engine: engine.engine_name.clone(),
                            engine_version: engine.engine_version.clone(),
                            depth: result.depth,
                            requested_depth: match limit {
                                SearchLimit::Depth(d) => Some(d),
                                SearchLimit::Nodes(_) => None,
                            },
                            requested_nodes: match limit {
                                SearchLimit::Depth(_) => None,
                                SearchLimit::Nodes(n) => Some(n),
                            },
                            search_limit_kind: limit.kind(),
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
                            seldepth: result.seldepth,
                            nps: result.nps,
                            hashfull: result.hashfull,
                            pv: result.pv,
                            policy_margin_cp: result.policy_margin_cp,
                            candidates: result.candidates,
                            engine_options_hash: Some(ctx.engine_options_hash_hex.clone()),
                            weight_sha256: ctx.weight_sha256.clone(),
                            was_timeout_salvaged: result.timed_out,
                        };
                        if let Some(path) = &cache_path {
                            if let Some(parent) = path.parent() {
                                let _ = fs::create_dir_all(parent);
                            }
                            // Cache writes always produce v2 (CacheEntry) -- only the read path needs
                            // to understand v1, for cache dirs populated before this round.
                            if let Some(c) = cache {
                                let (cache_requested_depth, search_limit_value) = match limit {
                                    SearchLimit::Depth(d) => (d, d as u64),
                                    SearchLimit::Nodes(n) => (0, n),
                                };
                                let entry = CacheEntry {
                                    cache_schema_version: CACHE_SCHEMA_VERSION,
                                    created_at: std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_secs())
                                        .unwrap_or(0),
                                    schema_version: SCHEMA_VERSION,
                                    engine_name: obs.engine.clone(),
                                    engine_version: obs.engine_version.clone(),
                                    engine_fingerprint: c.engine_fingerprint,
                                    engine_fingerprint_mode: c.engine_fingerprint_mode,
                                    engine_options_hash: c.engine_options_hash,
                                    requested_depth: cache_requested_depth,
                                    search_limit_kind: limit.kind(),
                                    search_limit_value,
                                    multipv: c.multipv,
                                    observation: obs.clone(),
                                };
                                if let Ok(json) = serde_json::to_string(&entry) {
                                    let _ = write_cache_entry_atomically(path, &json);
                                }
                            }
                        }
                        Some(obs)
                    }
                    Err(e) => {
                        if handle_analyse_error(
                            &e,
                            limit,
                            engine,
                            &ctx.strict_cfg,
                            worker.worker_id,
                            counters,
                        ) {
                            restart_needed = true;
                        }
                        None
                    }
                }
            }
        };

        if let Some(obs) = observation {
            if !matches!(ctx.existing_policy, ExistingPolicy::Append) {
                rec.observations.retain(|o| !same_search_request(o, &obs));
            }
            rec.observations.push(obs);
        }

        if restart_needed {
            break;
        }
    }

    if let Some(every) = ctx.strict_cfg.restart_engine_every
        && worker.searches_since_restart >= every
    {
        restart_needed = true;
    }

    restart_needed
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

/// Parse a comma-separated list of positive, distinct numeric limits without silently dropping
/// malformed entries. Search budgets are part of label identity, so accepting `4,bad,8` as
/// `4,8` or running the same budget twice would make the manifest describe a command other than
/// the one that was actually executed.
pub(super) fn parse_positive_csv<T>(value: &str, flag: &str) -> Result<Vec<T>>
where
    T: Copy + Default + Display + FromStr + Ord,
    T::Err: Display,
{
    let mut parsed = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in value.split(',') {
        let item = raw.trim();
        if item.is_empty() {
            anyhow::bail!("{flag} contains an empty value: {value:?}");
        }
        let number = item
            .parse::<T>()
            .map_err(|error| anyhow::anyhow!("invalid value {item:?} in {flag}: {error}"))?;
        if number <= T::default() {
            anyhow::bail!("{flag} values must be greater than zero, got {number}");
        }
        if !seen.insert(number) {
            anyhow::bail!("{flag} values must be distinct, duplicate {number}");
        }
        parsed.push(number);
    }
    Ok(parsed)
}

/// Parse `--engine-option NAME=VALUE` once, preserving the value verbatim while rejecting input
/// that used to disappear through `filter_map`. MultiPV has a dedicated flag because it also
/// changes observation/cache identity; accepting both controls would let those identities drift.
fn parse_engine_options(values: &[String]) -> Result<Vec<(String, String)>> {
    let mut parsed = Vec::with_capacity(values.len());
    let mut names = BTreeSet::new();
    for raw in values {
        let (name, value) = raw.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("invalid --engine-option {raw:?}; expected NAME=VALUE")
        })?;
        if name.trim().is_empty() {
            anyhow::bail!("invalid --engine-option {raw:?}; NAME must not be empty");
        }
        if name.eq_ignore_ascii_case("MultiPV") {
            anyhow::bail!(
                "MultiPV must be configured with --multipv, not --engine-option, so label identity remains consistent"
            );
        }
        let normalized = name.to_ascii_lowercase();
        if !names.insert(normalized) {
            anyhow::bail!("duplicate --engine-option name {name:?}");
        }
        parsed.push((name.to_string(), value.to_string()));
    }
    Ok(parsed)
}

pub(crate) fn cmd_label(args: LabelArgs) -> Result<()> {
    // Bounded-pipeline changes (worker count, cache usage, ordering) can't be judged without
    // measuring their actual effect on throughput -- this run's wall-clock start, used for
    // `records_per_sec` below.
    let run_start = std::time::Instant::now();
    // clap's required_unless_present/conflicts_with on --depths/--nodes guarantees exactly one
    // of these is Some.
    let limits: Vec<SearchLimit> = if let Some(depths) = &args.depths {
        let depths: Vec<u32> = parse_positive_csv(depths, "--depths")?;
        depths.into_iter().map(SearchLimit::Depth).collect()
    } else {
        let nodes: Vec<u64> = parse_positive_csv(
            args.nodes
                .as_ref()
                .expect("clap guarantees --depths or --nodes"),
            "--nodes",
        )?;
        nodes.into_iter().map(SearchLimit::Nodes).collect()
    };
    if args.timeout_ms == 0 {
        anyhow::bail!("--timeout-ms must be greater than zero");
    }
    if args.jobs == 0 {
        anyhow::bail!("--jobs must be greater than zero");
    }
    if args.multipv == 0 {
        anyhow::bail!("--multipv must be greater than zero");
    }
    let weight_sha256 = args
        .weight_file
        .as_ref()
        .map(|path| compute_weight_sha256(path))
        .transpose()?;
    // See `build_resume_index` for why this is an offset index, not the full-record map
    // `load_records` would give -- a missing path just means "nothing to resume yet" (lets a
    // wrapper script pass --resume-from unconditionally from the very first run), same as before.
    let mut resume_source: Option<(commands::ResumeIndex, File)> = match &args.resume_from {
        Some(path) if path.exists() => {
            let index = commands::build_resume_index(path)?;
            let file = File::open(path).with_context(|| format!("cannot open {path:?}"))?;
            Some((index, file))
        }
        _ => None,
    };

    let engine_path = args.engine.clone();
    let engine_name = args.engine_name.clone().unwrap_or_default();
    let timeout_ms = args.timeout_ms;
    let jobs = args.jobs;
    let mut engine_options = parse_engine_options(&args.engine_options)?;
    if args.multipv > 1 {
        engine_options.push(("MultiPV".to_string(), args.multipv.to_string()));
    }
    // --resume-from implies Skip unless --replace-existing is explicit: without this, forgetting
    // --skip-existing alongside --resume-from would merge in the old observations and *still*
    // redundantly re-run the engine for already-covered depths under the default Append policy --
    // no data loss, but it defeats the entire point of resuming.
    let existing_policy = if args.replace_existing {
        ExistingPolicy::Replace
    } else if args.skip_existing || args.resume_from.is_some() {
        ExistingPolicy::Skip
    } else {
        ExistingPolicy::Append
    };
    let strict_cfg = UsiStrictConfig {
        strict: args.usi_strict,
        restart_on_protocol_error: args.restart_on_protocol_error,
        restart_engine_every: args.restart_engine_every,
        transcript_dir: args.transcript_on_error.clone(),
    };
    let label_ctx = LabelContext {
        weight_sha256: weight_sha256.clone(),
        engine_options_hash_hex: engine_options_hash_hex(&engine_options),
        timeout_ms,
        multipv: args.multipv.max(1),
        existing_policy,
        strict_cfg,
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
                engine_fingerprint_mode,
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
    let counters = Arc::new(RunCounters {
        timeout_salvaged: AtomicUsize::new(0),
        protocol_violations: AtomicUsize::new(0),
        engine_restarts: AtomicUsize::new(0),
    });
    let done = Arc::new(AtomicUsize::new(0));

    // Reader: streams the input line-by-line so the whole dataset is never resident in memory --
    // only ever `queue_depth` records at a time, enforced by the permit acquire below. It also
    // accumulates the manifest's `input_hash` over the same lines it's already reading -- calling
    // `hash_file` afterward, like every other manifest-producing command does, would mean
    // re-opening and re-reading the whole input a second time purely to hash it, defeating the
    // point of streaming in the first place.
    let input_path = args.input.clone();
    let reader_handle = std::thread::spawn(move || -> Result<(u64, usize, u64, String, String)> {
        let mut reader = open_hashed_position_jsonl(&input_path)?;
        let mut job_id = 0u64;
        let mut skipped = 0usize;
        let mut resumed_count = 0u64;
        for item in &mut reader {
            match item {
                Ok(PositionRecordLine {
                    line_number: _,
                    mut record,
                }) if Sfen::parse(&record.sfen).is_ok() => {
                    if let Some((index, file)) = resume_source.as_mut()
                        && let Some(&offset) = index.get(&commands::merge_alignment_key(&record))
                    {
                        // Matches `load_records`' tolerance (the full-load path this replaced):
                        // an unreadable/malformed resume line is warned and skipped, not fatal --
                        // this position is simply relabeled from scratch, same as if it had never
                        // been in the resume file at all.
                        match commands::read_resume_observations_at(file, offset) {
                            Ok(resume_obs) => {
                                commands::merge_resume_observations(
                                    &mut record.observations,
                                    resume_obs,
                                );
                                resumed_count += 1;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "resume-from record unreadable at offset {offset}: {e}"
                                );
                            }
                        }
                    }
                    // Blocks here once `queue_depth` jobs are dispatched-but-unwritten.
                    if permit_rx.recv().is_err() || job_tx.send(Job { id: job_id, record }).is_err()
                    {
                        break; // writer/workers gone -- don't hang trying to feed a dead pipeline
                    }
                    job_id += 1;
                }
                Ok(PositionRecordLine { line_number, .. }) => {
                    tracing::warn!(line = line_number, "invalid SFEN, skipping");
                    skipped += 1;
                }
                Err(PositionRecordLineError::Parse {
                    line_number,
                    source,
                }) => {
                    tracing::warn!(line = line_number, "JSON parse error: {source}");
                    skipped += 1;
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("cannot read {input_path:?}"));
                }
            }
        }
        let (input_hash, dataset_sha256) = finish_hashed_position_jsonl(reader);
        Ok((job_id, skipped, resumed_count, input_hash, dataset_sha256))
    });

    // Workers: each owns one long-lived USI engine for its whole lifetime, launched lazily on
    // its first job and reused across every job it picks up after that -- spawning a fresh
    // engine per position would hide true search cost behind repeated process-startup overhead.
    let worker_handles: Vec<_> = (0..jobs)
        .map(|worker_id| {
            let job_rx = Arc::clone(&job_rx);
            let result_tx = result_tx.clone();
            let engine_path = engine_path.clone();
            let engine_name = engine_name.clone();
            let engine_options = engine_options.clone();
            let engine_launch_failures = Arc::clone(&engine_launch_failures);
            let counters = Arc::clone(&counters);
            let done = Arc::clone(&done);
            let limits = limits.clone();
            let label_ctx = label_ctx.clone();
            let cache = cache.clone();
            std::thread::spawn(move || {
                let mut engine: Option<UsiEngine> = None;
                let mut worker = WorkerState {
                    worker_id,
                    searches_since_restart: 0,
                };
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
                        engine = Some(
                            e.with_strict(label_ctx.strict_cfg.strict)
                                .with_transcript_capture(
                                    label_ctx.strict_cfg.transcript_dir.is_some(),
                                ),
                        );
                        worker.searches_since_restart = 0;
                    }
                    if let Some(eng) = engine.as_mut() {
                        let restart_needed = analyze_record(
                            &mut record,
                            eng,
                            &limits,
                            cache.as_deref(),
                            &counters,
                            &label_ctx,
                            &mut worker,
                        );
                        if restart_needed {
                            engine = None;
                            counters.engine_restarts.fetch_add(1, Ordering::Relaxed);
                        }
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

    // Writer: runs on this thread. Default mode writes each job on arrival, in whatever order
    // workers finish -- interrupt-safe, since nothing is ever held unwritten waiting on a
    // straggler. `--preserve-order` instead buffers out-of-order arrivals in `pending` (bounded to
    // `queue_depth` entries by the permit scheme above) and only flushes the next contiguous
    // job_id, so output order matches input order -- at the cost of losing every already-finished
    // job still buffered behind a slow one if the process is killed before it catches up.
    let out_file =
        File::create(&args.out).with_context(|| format!("cannot create {:?}", args.out))?;
    let mut out_writer = Sha256Writer::new(BufWriter::new(out_file));
    let mut manifest = args
        .manifest
        .as_ref()
        .map(|_| new_run_manifest("label", &args.input));
    let mut next_id = 0u64;
    let mut pending: BTreeMap<u64, PositionRecord> = BTreeMap::new();
    let mut written = 0usize;
    // Accumulated here (not a second pass) since write_one already sees every written record's
    // observations once, right before writing.
    let mut time_ms_sum = 0u64;
    let mut time_ms_count = 0usize;

    {
        let mut write_one =
            |record: &PositionRecord, manifest: &mut Option<RunManifest>| -> Result<()> {
                serde_json::to_writer(&mut out_writer, record)?;
                out_writer.write_all(b"\n")?;
                // Flush per record (not just once at the very end of the whole run): engine search
                // time dominates I/O time by orders of magnitude here, so this costs nothing
                // measurable, but it shrinks the window where a killed process loses writes still
                // sitting in this BufWriter -- independent of (and much smaller than) the
                // preserve_order/reorder-buffer loss window below.
                out_writer.flush()?;
                if let Some(m) = manifest.as_mut() {
                    accumulate_coverage(m, std::slice::from_ref(record));
                }
                for obs in &record.observations {
                    if let Some(t) = obs.time_ms {
                        time_ms_sum += t;
                        time_ms_count += 1;
                    }
                }
                // Release the permit only now that the record is durably written -- this is the other
                // half of the dispatch window: it caps how far ahead of the writer the pipeline can get.
                let _ = writer_permit_tx.send(());
                Ok(())
            };

        for job in result_rx {
            if args.preserve_order {
                for record in reorder_push(&mut pending, &mut next_id, job.id, job.record) {
                    write_one(&record, &mut manifest)?;
                    written += 1;
                }
                // Guards the permit scheme's whole reason for existing: without it, `pending`
                // could in principle grow past `queue_depth` and this bounded pipeline would
                // silently revert to the unbounded memory use it replaced.
                debug_assert!(pending.len() <= queue_depth);
            } else {
                write_one(&job.record, &mut manifest)?;
                written += 1;
            }
        }
    }
    out_writer.flush()?;
    let output_sha256 = out_writer.digest_hex();
    eprintln!();

    let (total, skipped, resumed_count, input_hash, dataset_sha256) = reader_handle
        .join()
        .map_err(|_| anyhow::anyhow!("label reader thread panicked"))??;
    for handle in worker_handles {
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("label worker thread panicked"))?;
    }
    let engine_launch_failures = engine_launch_failures.load(Ordering::Relaxed);
    let timeout_salvaged_count = counters.timeout_salvaged.load(Ordering::Relaxed);
    let protocol_violations_count = counters.protocol_violations.load(Ordering::Relaxed);
    let engine_restarts_count = counters.engine_restarts.load(Ordering::Relaxed);
    let cache_counts = cache.as_ref().map(|c| {
        (
            c.hits.load(Ordering::Relaxed),
            c.misses.load(Ordering::Relaxed),
        )
    });

    let elapsed_secs = run_start.elapsed().as_secs_f64();
    let records_per_sec = (elapsed_secs > 0.0).then(|| written as f64 / elapsed_secs);
    let cache_hit_rate = cache_counts.and_then(|(hits, misses)| {
        (hits + misses > 0).then(|| hits as f64 / (hits + misses) as f64)
    });
    let average_engine_time_ms =
        (time_ms_count > 0).then(|| time_ms_sum as f64 / time_ms_count as f64);

    let cache_suffix = cache_counts
        .map(|(hits, misses)| format!(", {hits} cache hits, {misses} cache misses"))
        .unwrap_or_default();
    let throughput_suffix = records_per_sec
        .map(|r| format!(", {r:.1} rec/s"))
        .unwrap_or_default();
    let accepted = usize::try_from(total).context("label record count exceeds this platform")?;
    let records_read = accepted
        .checked_add(skipped)
        .context("label input record count overflow")?;
    eprintln!(
        "done [{engine_display_name}, jobs={jobs}]: {records_read} in, {written} labeled, {skipped} skipped, {engine_launch_failures} engine launch failures{cache_suffix}{throughput_suffix} → {:?}",
        args.out
    );
    if let Some(mut manifest) = manifest {
        manifest.input_hash = input_hash;
        manifest.records_read = records_read;
        manifest.records_kept = written;
        manifest.records_dropped = skipped;
        manifest.engine_name = Some(engine_display_name);
        let depth_values: Vec<u32> = limits
            .iter()
            .filter_map(|l| match l {
                SearchLimit::Depth(d) => Some(*d),
                SearchLimit::Nodes(_) => None,
            })
            .collect();
        let node_values: Vec<u64> = limits
            .iter()
            .filter_map(|l| match l {
                SearchLimit::Nodes(n) => Some(*n),
                SearchLimit::Depth(_) => None,
            })
            .collect();
        manifest.depths = (!depth_values.is_empty()).then_some(depth_values);
        manifest.nodes = (!node_values.is_empty()).then_some(node_values);
        manifest.multipv = (args.multipv > 1).then_some(args.multipv);
        manifest.engine_options = Some(args.engine_options.clone());
        manifest.jobs = Some(jobs);
        manifest.engine_launch_failures = Some(engine_launch_failures);
        manifest.timeout_salvaged_count = Some(timeout_salvaged_count);
        manifest.protocol_violations_count = args.usi_strict.then_some(protocol_violations_count);
        manifest.engine_restarts = Some(engine_restarts_count);
        manifest.records_per_sec = records_per_sec;
        manifest.average_engine_time_ms = average_engine_time_ms;
        manifest.preserve_order = Some(args.preserve_order);
        manifest.resume_from = args.resume_from.clone();
        manifest.resumed_count = args.resume_from.is_some().then_some(resumed_count);
        if let Some((hits, misses)) = cache_counts {
            manifest.cache_hits = Some(hits);
            manifest.cache_hit_rate = cache_hit_rate;
            manifest.cache_misses = Some(misses);
            manifest.engine_fingerprint_mode = Some(engine_fingerprint_mode.as_str().to_string());
        }
        // shogiesa-owned provenance fields: dataset_sha256/binary_sha256/weight_sha256/
        // engine_threads/engine_hash_mb are computed here; everything else is opaque
        // orchestrator-supplied passthrough that shogiesa doesn't interpret.
        manifest.dataset_sha256 = Some(dataset_sha256);
        manifest.output_sha256 = Some(output_sha256);
        manifest.binary_sha256 = compute_binary_sha256(&engine_path);
        manifest.weight_sha256 = weight_sha256;
        manifest.engine_threads = engine_option_u32(&engine_options, "Threads");
        manifest.engine_hash_mb = engine_option_u32(&engine_options, "Hash");
        manifest.experiment_id.clone_from(&args.experiment_id);
        manifest.candidate_id.clone_from(&args.candidate_id);
        manifest.baseline_id.clone_from(&args.baseline_id);
        manifest.lineage_id.clone_from(&args.lineage_id);
        manifest
            .teacher_manifest_sha256
            .clone_from(&args.teacher_manifest_sha256);
        manifest.init_seed = args.init_seed;
        manifest.split_seed = args.split_seed;
        manifest.split_sha256.clone_from(&args.split_sha256);
        manifest.validity.clone_from(&args.validity);
        write_manifest(args.manifest.as_ref().unwrap(), &manifest)?;
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
            super::super::transforms::assign_split_bucket(42, "some-fixed-key", 0.1, 0.1),
            super::super::transforms::SplitBucket::Train
        );
    }

    // seeded_hash_is_a_stable_golden_value moved to stratifykit-core (seeded_hash now lives
    // there).

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
            engine_fingerprint_mode: EngineFingerprintMode::None,
            hits: Arc::new(AtomicUsize::new(0)),
            misses: Arc::new(AtomicUsize::new(0)),
        };
        let path = label_cache_path(
            &cache,
            "startpos",
            "engine",
            Some("1.0"),
            SearchLimit::Depth(8),
        );
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
            engine_fingerprint_mode: if fp.is_some() {
                EngineFingerprintMode::Content
            } else {
                EngineFingerprintMode::None
            },
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
            SearchLimit::Depth(8),
        );
        let path_b = label_cache_path(
            &cache_with_fingerprint(Some(222)),
            "startpos",
            "engine",
            Some("1.0"),
            SearchLimit::Depth(8),
        );
        assert_ne!(
            path_a, path_b,
            "a rebuilt engine binary must not collide with a stale cache entry"
        );
    }
}
