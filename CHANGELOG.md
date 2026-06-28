# Changelog

All notable changes to shogiesa are documented here.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)

---

## [Unreleased]

### Planned
- `shogiesa filter` command — stability-based position filtering
- Parallel labeling (`--jobs N`)
- `--engine-option Key=Value` for USI option passthrough
- eval bucket distribution in `report` (post-label)
- depth disagreement metric in `report`

---

## [0.3.0] — 2026-06-28

### Added
- `shogiesa label` command — labels positions with USI engine evaluations
  - Streaming JSONL read/write (memory-efficient for large datasets)
  - Multiple depths per run (`--depths 4,6,8`)
  - Per-depth timeout (`--timeout-ms`)
  - Appends to existing observations; safe to re-run
- `shogiesa-usi` crate — USI engine process management
  - `UsiEngine::launch()` and `UsiEngine::launch_command()` (for test injection)
  - stdout reader thread + `mpsc::recv_timeout` for non-blocking timeout
  - `info` line parser: depth, score cp/mate, nodes, time, pv
- `fake-usi-engine` binary — deterministic test double for USI integration tests
  - `--hang` flag simulates a hung engine for timeout testing
- `Score` enum replacing `score_cp: i32`
  - `Score::Cp { value: i32 }` — centipawn score
  - `Score::Mate { moves: i32 }` — mate-in-N score
  - JSON: `{"kind":"cp","value":43}` / `{"kind":"mate","moves":3}`
- `time_ms` and `pv` fields added to `Observation`

### Changed
- `Observation.score_cp: i32` → `Observation.score: Score` (**schema change**)

---

## [0.2.0] — 2026-06-28

### Added
- `SideToMove` enum (`Black` / `White`) replacing `side_to_move: String`
- `GamePhase` enum (`Opening` / `Middlegame` / `Endgame`) replacing `phase: String`
  - Both use `serde(rename_all = "lowercase")` — JSON output unchanged
- `Sfen::parse()` — syntactic SFEN validator in `shogiesa-core::sfen`
  - Checks: field count, rank width (9 squares), side character, hand notation, move count ≥ 1
  - Returns `SideToMove` via `.side_to_move()` method
- `validate --strict` flag — exit 1 only with `--strict` (default: warnings + exit 0)
- `shogiesa validate` now reports `invalid SFENs` count using `Sfen::parse()`
- `shogiesa report` additions: invalid SFENs, source dominance, balance warnings
  (opening ratio > 50%, side imbalance > 65/35, duplicate rate > 5%)
- CLI integration tests (`crates/shogiesa-cli/tests/cli_test.rs`) using `assert_cmd`

### Changed
- `phase_from_ply()` returns `GamePhase` instead of `&'static str`
- `sfen_side()` helper in CLI replaced by `Sfen::parse()` throughout
- `Commands::Validate` now takes `ValidateArgs` (separated from `ReportArgs`)

### Fixed
- `side_to_move` tag was inverted — after Black plays, tag incorrectly said `"black"` instead of `"white"`

---

## [0.1.0] — 2026-06-28

### Added
- `shogiesa extract` — CSA game record → SFEN positions JSONL
  - Options: `--min-ply`, `--max-ply`, `--every-n-plies`, `--dedup`
  - Handles file and directory input (globs `*.csa`)
  - Board state tracker: CSA `Action::Move` → SFEN generation without external shogi crate
  - Drop moves detected via `from.file == 0` (CSA `00` from-square convention)
- `shogiesa report` — dataset statistics
  - Position count, ply range, phase/side distribution, source file counts
- `shogiesa validate` — data integrity check
  - Broken JSON lines, duplicate SFENs, `side_to_move` vs SFEN mismatch
- `shogiesa-core` — shared domain types
  - `PositionRecord`, `SourceInfo`, `PositionTags`, `Observation`
- `shogiesa-csa` — CSA ingestion library
  - Uses `csa = "1"` crate for parsing; own board tracker for SFEN generation
- `shogiesa-cli` — CLI binary
- GitHub Actions CI (fmt + clippy -D warnings + test)
- README and README_ja.md
- `tests/fixtures/sample.csa` — 5-move test game

[Unreleased]: https://github.com/kent-tokyo/shogiesa/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kent-tokyo/shogiesa/releases/tag/v0.1.0
