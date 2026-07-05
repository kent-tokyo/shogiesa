# Changelog

All notable changes to shogiesa are documented here.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)

---

## [Unreleased]

### Added
- `Observation.requested_depth: Option<u32>` — the depth `label` asked the engine to search to, distinct from `depth` (what it actually reached). `None` on records labeled before this field existed.
- `filter --require-requested-depth-reached` excludes positions where any non-mate observation's achieved depth fell short of its own `requested_depth`; a no-op on observations with no recorded `requested_depth`. Mate observations are exempt, same rationale as `--min-depth-reached`.
- `report` shows a requested-depth underreach rate (how many observations with a `requested_depth` fell short of it) when any are present in the dataset
- `label`/`filter`/etc. `--manifest` gains `requested_depth_total`/`requested_depth_underreach` counters
- `select` command — picks positions worth a closer look/re-label instead of re-labeling an entire dataset at higher depth. `--strategy uncertain` ranks by `evaluate_quality`'s pass-fraction (reusing `filter`'s exact gates); `--strategy hard` ranks by eval swing/bestmove disagreement/blunder-adjacency (reusing `mine`'s blunder-window detection); `--strategy coverage` prioritizes the thinnest phase/side/eval-bucket combinations (reusing `balance`'s bucket key). Outputs in ranked order, not restored to input order.
- `label --cache-dir PATH` caches each observation as a sharded, content-addressed JSON file keyed on `(sfen, engine name, engine version, engine options, requested depth, multipv, schema version)`, so repeated experiments over the same positions reuse a cached observation instead of re-running the engine. No database — plain files. Cache hit/miss counts appear in `--manifest`.
- `SourceInfo.root_id`/`variation_id`/`branch_from_ply` (all `Option`) — `root_id` is shared by a KIF game's mainline and every variation branching from it; `variation_id`/`branch_from_ply` are set on variation records only. `None` on CSA-extracted positions (no variation concept) and on JSONL predating this field.
- `label --engine-fingerprint-mode content|metadata|none` (default `content`) folds the engine binary itself into the `--cache-dir` cache key, on top of its USI-reported `id name`/`id version` — those strings are controlled by the engine and aren't guaranteed to change after a local rebuild, so relying on them alone risked a cache hit silently reusing labels produced by a different executable. `content` hashes the binary's bytes (read once at startup); `metadata` hashes its canonical path/size/mtime instead (cheaper, but invalidates on every rebuild into a fresh path even when the bytes are unchanged); `none` restores the original identity-strings-only behavior. If the engine path can't be read/stat'd (e.g. `--engine` is a bare name resolved via `PATH`, which `fs::read`/`fs::canonicalize` can't follow), fingerprinting degrades gracefully to `none` for that run with a warning, rather than failing `label` outright over a case that worked before this flag existed. `--manifest` gains `engine_fingerprint_mode` when `--cache-dir` is used.
- `Observation.score_perspective: ScorePerspective` (`side_to_move`/`black`) makes explicit which side a `cp` value's sign is relative to. USI's `info score cp` is side-to-move-relative by protocol convention and `label` never converts it, so every observation `label` produces is `side_to_move`; `#[serde(default)]` (no `skip_serializing_if`) loads older JSONL missing this field as `side_to_move` too, which is exactly what that data always meant.
- `Observation.bestmove_kind: Option<BestMoveKind>` (`resign`/`win`/`no_move`) classifies a `bestmove` that's a special USI token rather than an ordinary move. `None` (absent on the wire) for the common case of an ordinary move, including on JSONL predating this field.
- `shogiesa_core::{effective_bestmove_kind, bestmove_agreement, has_special_bestmove}` — shared helpers that classify a `bestmove` (falling back to classifying the literal string when `bestmove_kind` is absent, so older JSONL benefits too) and compute agreement while excluding special tokens from the comparison. `report` shows a special-bestmove rate (fraction of labeled positions with at least one `resign`/`win`/`none` observation) when any are present.

### Changed
- `SCHEMA_VERSION` bumped to 6 and pack `FORMAT_VERSION` bumped to 6 for the new `Observation.requested_depth` field; old `.shgpk` files are not readable by this version
- `label --replace-existing`'s dedup now also matches on `requested_depth` (treating a legacy `None` as a wildcard), so "requested 12, reached 8" and "requested 8, reached 8" are no longer collapsed into the same entry
- `validate` now reads its input line-by-line instead of loading the whole file into memory, so it stays memory-flat on multi-GB JSONL
- `label` now streams its input and output through a bounded reader/worker-pool/writer pipeline instead of loading the whole dataset into memory and collecting the whole labeled result before writing anything; memory now scales with `--jobs`, not with dataset size. Output order matches input order by default; `label --unordered-output` opts out of that for higher throughput. `label` no longer depends on `rayon`.
- `SCHEMA_VERSION` bumped to 7 and pack `FORMAT_VERSION` bumped to 7 for the new `SourceInfo` fields; old `.shgpk` files are not readable by this version
- `split --train/--valid/--test` now groups by `source.root_id` when present, falling back to stripping the `path`'s `#varN@ply` suffix (its previous, sole mechanism) for records without `root_id`
- Every persistent/reproducibility-critical hash (`label --cache-dir` cache keys, `--manifest`'s `input_hash`, `split`'s train/valid/test bucket assignment, `sample`/`select`'s seeded tie-breaks) now uses `blake3` instead of `std::collections::hash_map::DefaultHasher`. `DefaultHasher` is deterministic within one build, but std's own docs disclaim stability *across Rust toolchain versions* — for a tool whose purpose is reproducible splits/samples/caches, that was a latent risk. **Consequence:** re-running `split`/`sample`/`select` with the same `--seed` after upgrading will not reproduce output made before this upgrade (expected — the new hash no longer depends on toolchain internals going forward; re-run once on this version and results are stable across every future toolchain). Every existing `label --cache-dir` entry becomes a permanent, silent miss after upgrading (old cache dirs can be deleted). `RunManifest.input_hash` changes from a 16-hex-char digest to a 64-hex-char one; a new `fingerprint_algorithm` field (`"blake3"`) distinguishes manifests written after this change from ones written before (which lack the field)
- `SCHEMA_VERSION` bumped to 8 and pack `FORMAT_VERSION` bumped to 8 for the new `Observation.score_perspective`/`bestmove_kind` fields; old `.shgpk` files are not readable by this version
- **`filter --eval-min`/`--eval-max` now compare against Black-perspective cp instead of the raw side-to-move-relative value USI reports.** Previously, "+300" meant "good for whoever's turn it was" — so the same absolute position could pass or fail an eval-range gate depending only on whose turn it was, and a dataset's eval-range filtering was inconsistent across roughly half its positions (whichever side wasn't Black). `balance --by eval-bucket` and `report`'s eval histogram/cross-tabs are normalized the same way, fixing the same inconsistency there (previously `eval bucket x side`'s row axis mixed both perspectives under one bucket, which couldn't actually compare Black-to-move vs. White-to-move eval distributions on a shared scale). New `cp_from_black_perspective`/`cp_from_side_to_move_perspective` utilities in `shogiesa-core` centralize the conversion. **Consequence:** `filter --eval-min/--eval-max` may keep/drop a different set of positions than before for any dataset containing White-to-move positions; `balance --by eval-bucket` may form different buckets. This flag's 0.2.0-era description as an "absolute cp range gate" was already inaccurate (it was always side-to-move-relative) — this change makes it actually absolute.
- **Every bestmove-agreement check now excludes special tokens (`resign`/`win`/`none`) from the comparison instead of treating them as an ordinary move string.** Previously, one engine/observation resigning while another returned a real move registered as a *disagreement* — a false positive unrelated to actual position ambiguity, since giving up isn't an opinion about which move is best. This affected `filter --require-bestmove-agreement`, `evaluate_quality`'s inline gate, `stability`'s serialized `StabilityInfo.bestmove_agreement`/`engine_bestmove_agreement` fields, `select --strategy hard`'s hardness ranking, and `report`'s "depth disagree"/"engine disagree" counters — all five now route through shared `shogiesa_core::bestmove_agreement`/`engine_bestmove_agreement` instead of five independent raw-string comparisons. Falls back to classifying the literal `bestmove` string when `bestmove_kind` is absent, so older JSONL benefits immediately, not just newly-labeled data. **Consequence:** any of the above may keep/rank/report positions differently than before for datasets containing resign/win/none observations.
- `sample` and `select --strategy uncertain/coverage` now stream their input and keep a bounded top-`--count` heap instead of materializing the whole dataset into memory first; memory now scales with `--count`, not with dataset size. `coverage` reads its input twice (tally bucket sizes, then rank) since a bucket's size isn't known until every position naming it has been seen. Output is provably identical to the previous full-materialize-sort-truncate code (same tie-break chain: primary rank, then `seeded_hash`, then original index) — confirmed by golden-output tests captured against the pre-refactor binary, including a fixture that forces a genuine tie contest via a duplicated sfen. `select --strategy hard` is unchanged (still fully materializes; its blunder-adjacency signal fundamentally needs a whole game's positions grouped together).

### Fixed
- `extract --dedup-zobrist` no longer collapses every unparseable SFEN into a single sentinel hash (`0`); each unparseable position is now individually warned about and counted as skipped, instead of the first bad SFEN silently absorbing all later, unrelated bad SFENs as "duplicates"
- `label --cache-dir` writes are now atomic (temp file + rename) instead of a direct `fs::write`, so a crash/kill/disk-full mid-write can no longer leave a torn JSON file visible to a concurrent `label` process sharing the same cache dir

---

## [0.4.0] — 2026-07-04

### Fixed
- KIF: support `同` (same-square) notation; previously truncated extraction of any game containing it
- KIF: stop cleanly at `変化` (variation) blocks instead of misapplying moves and truncating extraction
- USI: `analyse()`/`handshake()` timeouts are now elapsed-time based, so an engine that streams `info` without ever sending `bestmove` can no longer hang `label` forever
- USI: `analyse()` now reports the depth the engine actually reached instead of blindly echoing the requested depth, so an engine that stops early (e.g. a forced mate) no longer mislabels a shallow observation as the target depth
- `split`: propagate per-file I/O errors instead of panicking
- `label`: warn (instead of silently dropping) when a worker thread's USI engine fails to launch
- USI: `analyse()`'s `policy_margin_cp` now also checks the bestmove's (rank 1's) own `ScoreBound`, not just the runner-up's — a lowerbound/upperbound-tagged bestmove score (possible with aspiration-window searches) was still being used as a confirmed evaluation for the margin subtraction
- `split --train/--valid/--test`: a KIF variation's positions now hash into the same split bucket as the mainline game it branched from, instead of independently by its suffixed `source.path` — previously a variation and its mainline (which share a parent position) could land in different splits, leaking correlated positions across train/valid/test
- `shogiesa-pack`'s module doc comment was still describing format version 4 and didn't mention the per-observation `score_bound` byte added when `FORMAT_VERSION` bumped to 5 — both fixed to match the actual encoding

### Changed
- `PositionRecord::fill_stability()` and `filter --max-score-swing-cp` now share one `score_swing()` implementation
- `SCHEMA_VERSION` bumped to 2 and pack `FORMAT_VERSION` bumped to 2 for the new `Observation.policy_margin_cp` field; old `.shgpk` files are not readable by this version
- `SCHEMA_VERSION` bumped to 3 and pack `FORMAT_VERSION` bumped to 3 for the new `StabilityInfo.engine_bestmove_agreement`/`engine_score_swing_cp` fields; old `.shgpk` files are not readable by this version
- `SCHEMA_VERSION` bumped to 4 and pack `FORMAT_VERSION` bumped to 4 for the new `Observation.candidates`/`CandidateMove.score_bound` fields; old `.shgpk` files are not readable by this version
- `SCHEMA_VERSION` bumped to 5 and pack `FORMAT_VERSION` bumped to 5 for the new `Observation.score_bound` field; old `.shgpk` files are not readable by this version
- `filter`'s gate-checking (min observations, phase, mate/in-check/capture exclusion, eval range, score swing, policy margin, bestmove/engine agreement) moved into `shogiesa_core::evaluate_quality()`, driven by a new `QualityConfig`/`QualityDecision`, so the decision logic lives in one place instead of being closed inside the CLI. `filter`'s stderr drop-reason output is unchanged.
- `label` now runs on a local rayon thread pool instead of a process-global one

### Added
- `filter --min-depth-reached N` excludes positions where any non-mate observation's achieved depth is below `N`; mate observations are exempt since an engine stopping short of the requested depth is dominantly caused by finding a forced mate (a confirmed result), not a weak search — gating on depth without this exemption would penalize the most reliable observations
- `filter --explain-out PATH` writes every rejected record to a JSONL file as `{"record": ..., "quality": ...}`, pairing the dropped record with its full `QualityDecision` (every failing reason, not just the first one used for the stderr breakdown); `QualityDecision`/`QualityReason` gained `Serialize` for this
- `Observation.score_bound: ScoreBound` — whether the bestmove's own score is a confirmed evaluation or a search bound, populated from the engine's rank-1 `info` line independent of MultiPV. Previously only `CandidateMove.score_bound` carried this, so a plain single-PV label whose score was a lowerbound/upperbound (e.g. an aspiration-window fail-high/low) silently lost the information.
- `filter --require-exact-score` excludes positions where any observation's score is a search bound rather than a confirmed evaluation
- `filter --require-policy-margin` excludes positions where no observation has a computed `policy_margin_cp` at all — unlike `--min-policy-margin-cp` (a no-op when every margin is unset), this requires a margin to have been computed in the first place
- `report` shows an observation-level `score_bound` distribution (distinct from the existing MultiPV-candidate-level one, which is unaffected and stays conditional on MultiPV usage) — this one is unconditional, so it surfaces label confidence for plain single-PV-labeled datasets too
- `filter --dry-run` reports what would be kept/dropped (and why) without writing `--out`, which becomes optional in this mode; combine with `--manifest` for a structured preview of a filter config's effect with no output file
- `report` shows MultiPV-candidate coverage and a `score_bound` (exact/lowerbound/upperbound) distribution when `label --multipv N` (N≥2) was used, shared with the `--manifest` fields of the same name via one `candidate_coverage_stats()` helper instead of duplicating the tally
- `filter`/`balance`/`sample`/`pack`/`label --manifest PATH` writes an opt-in run manifest (JSON): shogiesa version, git sha, schema/pack-format version, full command args, input path + a non-cryptographic content hash (change-detection only, not a verifiable checksum), records read/kept/dropped, drop-reason counts, labeled/unlabeled record counts, MultiPV-candidate coverage, score-bound distribution, and (for `filter`) the resolved `QualityConfig` or (for `label`) engine name/depths/MultiPV/engine options/jobs/engine-launch-failure count. `split` is not covered — it already has its own tailored `manifest.json`.
- `Observation.candidates: Vec<CandidateMove>` — every MultiPV rank from a `label --multipv N` (N≥2) pass (not just the top-2 used for `policy_margin_cp`), each with its own `multipv`/`bestmove`/`score`/`score_bound`/`pv`. Populated only when MultiPV≥2 was actually used, matching `policy_margin_cp`'s existing convention (empty otherwise, so ordinary single-PV labeling gains no output size). `ScoreBound` (`exact`/`lowerbound`/`upperbound`) distinguishes a confirmed evaluation from a search bound, replacing the internal boolean that only asked "is this a bound at all".
- KIF: `変化` (variation/branch) blocks are now extracted as additional positions, not just cleanly skipped. Each variation always branches from the mainline (never from another variation — nested variations are out of scope), and gets its own `source.path` suffix (`game.kif#varN@ply`) so it can't collide with the mainline's positions or with a sibling variation, keeping `split --by-source`/`mine`'s per-source-path grouping correct.
- Cross-engine (teacher ensemble) disagreement signal: `stability`/`filter`/`report` now distinguish disagreement *between distinct engines* (each engine's deepest observation as its vote) from disagreement across depths of the same engine, which the existing `bestmove_agreement`/`score_swing_cp` metrics conflated. New `filter --require-engine-agreement` / `--max-engine-score-swing-cp` gates (no-op on positions labeled by only one engine — see `label --engine-name` to label with multiple engines against the same file) and a `report` engine-disagreement rate.
- `label --skip-existing` / `--replace-existing` (mutually exclusive) — skip or overwrite an observation from the same engine at a depth already covered, instead of always appending a duplicate. Both key off the depth the engine *actually achieved*, not the one requested, so they behave correctly even when an engine stops early (e.g. a forced mate) and under-reaches the target depth across repeated runs.
- `split --train/--valid/--test` — a source-aware, seeded ratio split (`--valid-frac`/`--test-frac`) that assigns each source game's positions to exactly one of the three splits, so near-duplicate positions from the same game can't leak across train/valid/test. Writes a `manifest.json` with the seed, requested fractions, and actual per-split position/source counts (which deviate from the requested fractions since games vary in length — that's correct no-leakage behavior).
- `report` shows cp/mate ratio, average score swing (plus a histogram of the existing `score_swing_cp` metric — not a new composite score), average `policy_margin_cp`, and eval-bucket × phase / eval-bucket × side cross-tabs
- `label --multipv N` (N≥2) sends `setoption name MultiPV`, parses the runner-up `info` line, and populates each observation's `policy_margin_cp` (bestmove's cp score minus the runner-up's) — a low margin means a weak teacher label even when a bestmove exists. Lowerbound/upperbound-tagged runner-up lines are ignored rather than trusted as a real evaluation.
- `filter --min-policy-margin-cp`, excluding positions whose margin is too small; observations without a computed margin never trigger this gate
- `filter --exclude-in-check` / `--exclude-capture`, wiring the existing `tags.in_check`/`tags.has_capture` into filtering
- `filter` prints a per-reason drop-count breakdown to stderr, not just an aggregate skipped count
- `report` shows in-check ratio and capture ratio
- `split --by-source` writes a `manifest.json` (input path, schema version, shogiesa version, per-file counts) alongside the split output files
- `cargo-audit` CI job; `dependabot.yml` for weekly cargo/github-actions updates
- Cross-platform CI test matrix (ubuntu/windows/macos)
- `criterion` benchmarks for `shogiesa-core`'s `Sfen::parse` / `Board::apply_normal` / `Board::to_sfen`

---

## [0.3.0] — 2026-06-28

### Added
- `shogiesa-kif` crate — KIF format ingestion (kanji ranks, full-width file digits, promotions, drops, handicap boards); `shogiesa-core` gains a shared `Board`/`PieceType` used by both `shogiesa-csa` and `shogiesa-kif`
- `shogiesa-pack` crate — compact binary encoding (`b"SHOGIESA"` magic + length-prefixed LE fields) with `shogiesa pack` / `shogiesa unpack` CLI commands
- `shogiesa stability` — computes `score_swing_cp` / `bestmove_agreement` and attaches `StabilityInfo` to each record
- `shogiesa mine` — hard-position mining via blunder detection (eval swing) and/or a losing-eval threshold
- `shogiesa balance` — rebalances a dataset by phase/side/eval-bucket
- `shogiesa split --by-source` / `shogiesa sample --count --seed` — dataset slicing
- `label --jobs N` — parallel labeling (one engine process per worker thread)
- `label --engine-option Key=Value` — USI option passthrough (repeatable)
- `extract --dedup-zobrist` — Zobrist-hash-based dedup
- `in_check` / `has_capture` tags are now computed (`Board::is_in_check` / `is_capture`) instead of always `false`
- `report`: eval-bucket histogram, depth-disagreement count, per-depth observation counts

---

## [0.2.0] — 2026-06-28

### Added
- `shogiesa filter` command — stability-based position filtering
  - `--require-bestmove-agreement` — all observations must agree on bestmove
  - `--max-score-swing-cp N` — cap on cp difference across observations
  - `--exclude-mate` — drop positions with any `Score::Mate` observation
  - `--eval-min` / `--eval-max` — absolute cp range gate
  - `--min-observations N` — require at least N observations
  - `--phase opening,middlegame,endgame` — game phase filter
  - Streaming read/write; JSON errors warned and skipped
- `shogiesa report` — eval bucket distribution
  - 200cp-width histogram of deepest-observation scores (ASCII bars)
  - Labeled / unlabeled position counts
  - Depth disagreement count (bestmove differs across depths)
- 8 new filter CLI integration tests

---

## [0.1.0] — 2026-06-28

### Added
- `shogiesa extract` — CSA game records → SFEN positions JSONL
  - `--min-ply`, `--max-ply`, `--every-n-plies`, `--dedup`
  - Board state tracker: CSA `Action::Move` → SFEN without external shogi crate
  - Drop moves: `from.file == 0` (CSA `00` from-square convention)
- `shogiesa label` — USI engine evaluation labeling
  - `shogiesa-usi` crate: stdout reader thread + `mpsc::recv_timeout` for timeout
  - `Score` enum: `Cp { value: i32 }` / `Mate { moves: i32 }`
    - JSON: `{"kind":"cp","value":43}` / `{"kind":"mate","moves":3}`
  - `Observation` fields: `score`, `bestmove`, `nodes`, `time_ms`, `pv`
  - `fake-usi-engine` binary for integration testing (`--hang` for timeout tests)
  - Appends to existing observations; re-labelable
- `shogiesa report` — dataset statistics
  - Phase/side distribution, ply range, source file counts
  - Duplicate SFENs, tag mismatches, source dominance, balance warnings
- `shogiesa validate` — data integrity check
  - Broken JSON, invalid SFENs (`Sfen::parse()`), duplicate SFENs, tag mismatches
  - `--strict` flag: exit 1 on any issue (CI mode)
- `shogiesa-core` domain types
  - `SideToMove` / `GamePhase` enums (`serde(rename_all = "lowercase")`, JSON unchanged)
  - `Sfen::parse()` — syntactic validator (field count, rank width, side, hand, move count)
- `shogiesa` meta crate re-exporting core/csa/usi
- GitHub Actions CI (fmt + clippy -D warnings + test)
- CLI integration tests (`assert_cmd` / `predicates` / `tempfile`)
- `LICENSE-MIT` and `LICENSE-APACHE`

[Unreleased]: https://github.com/kent-tokyo/shogiesa/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kent-tokyo/shogiesa/releases/tag/v0.1.0
