# Changelog

All notable changes to shogiesa are documented here.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)

---

## [Unreleased]

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

[Unreleased]: https://github.com/kent-tokyo/shogiesa/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kent-tokyo/shogiesa/releases/tag/v0.1.0
