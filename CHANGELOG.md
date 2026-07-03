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

### Changed
- `PositionRecord::fill_stability()` and `filter --max-score-swing-cp` now share one `score_swing()` implementation
- `SCHEMA_VERSION` bumped to 2 and pack `FORMAT_VERSION` bumped to 2 for the new `Observation.policy_margin_cp` field; old `.shgpk` files are not readable by this version

### Added
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
