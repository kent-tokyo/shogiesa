# Changelog

All notable changes to shogiesa are documented here.

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)

---

## [Unreleased]

### Planned
- Parallel labeling (`--jobs N`)
- `--engine-option Key=Value` for USI option passthrough
- `shogiesa-kif` — KIF format ingestion
- `shogiesa split` / `shogiesa sample` commands

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

[Unreleased]: https://github.com/kent-tokyo/shogiesa/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kent-tokyo/shogiesa/releases/tag/v0.1.0
