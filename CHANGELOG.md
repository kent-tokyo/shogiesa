# Changelog

Notable changes to shogiesa follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Compare links at the end provide the complete commit history.

## [Unreleased]

## [0.10.1] — 2026-10-03

### Added

- `extract --recursive` discovers nested `.csa`, `.kif`, and `.ki2` files in deterministic
  relative-path order. It records relative provenance, deduplicates across the complete tree,
  reports skipped entries, and never follows symlinks. Shallow traversal remains the default.
- A fixture-backed reproducibility matrix verifies stable identity, order, and dataset hashes
  across absolute input paths, reversed input order, and one/two label workers.

### Documentation

- The experiment-envelope consumer audit now records the incompatible quietset, lineprior, and
  veridict contracts and keeps nested v1 emission deferred until consumers agree.

### Changed

- Updated `encoding_rs` to 0.8.42 and `thiserror` to 2.0.21.

## [0.10.0] — 2026-09-23

### Fixed

- KIF nested variations now replay from their active parent and retain deterministic lineage.
- SFEN and distribution parsing reject oversized values instead of overflowing or looping over
  unreasonable ranges.
- CLI golden fixtures are line-ending independent, and malformed-input fixtures are tracked.
- Public documentation and dependency versions were synchronized for the release.

## [0.9.2] — 2026-09-04

### Added

- `dataset-diff` reports semantic record and distribution changes independently of input order.
- `recipe plan/run/verify` validates typed stage graphs, commits staged outputs atomically,
  checkpoints each stage, supports explicit resume, and verifies output hashes.
- Fixture-backed manifest and interrupted-run tests fix the recipe durability contract.

## [0.9.1] — 2026-09-03

### Added

- Pack corruption fixtures cover bad magic, version, endian, truncation, and trailing bytes.
- Golden CLI outputs cover report, validate, conflict/block reports, distribution, calibration,
  missing buckets, and malformed input.

## [0.9.0] — 2026-07-26

### Added

- `label --nodes` adds fixed-node search beside fixed-depth search.
- Observations record requested limit, search telemetry, engine-option identity, and weight hash.
- Strict USI checks detect unsolicited, duplicate, delayed, or illegal bestmoves; engine restart
  and transcript controls improve unattended labeling.
- Manifests gained dataset, engine, weight, seed, lineage, and gate-opening provenance fields.

### Fixed

- Node-limited observations no longer collide during merge.
- Dead engine processes are relaunched instead of being reused for the remaining queue.

### Compatibility

- JSONL schema and pack format advanced to 11. JSONL additions use defaults; older binary packs
  require conversion through a compatible release.

## [0.8.0] — 2026-07-19

### Added

- Timeout salvage keeps the deepest completed USI result and marks it explicitly.
- `shuffle` writes deterministic training order and an optional order manifest.
- Position records gained game-result provenance and WDL-aware balancing/reporting.
- `make-gate-openings` creates root-diverse, playable SFEN suites with a reproducibility manifest.
- Sekirei dataset-ablation and gate-evaluation runbooks separate data diagnostics from strength
  measurement.

### Changed

- Resume indexing, report, sample, balance, and several selection paths now use bounded or
  streaming memory while preserving golden output order.
- Evaluation buckets use Black-perspective cp; special bestmove tokens are excluded from move
  agreement checks.

### Fixed

- Zobrist dedup no longer maps every invalid SFEN to one sentinel value.
- Label-cache writes are atomic.

### Compatibility

- JSONL schema and pack format advanced through 9 and 10 for timeout and game-result provenance.

## [0.7.0] — 2026-07-07

### Added

- `from-match` accepts `position startpos` and custom `position sfen` logs.
- `label --resume-from`, `distribution`, and group-aware `stratify` support interrupted and
  coverage-driven dataset work.
- `shogiesa --version` reports the workspace version.

### Changed

- Parallel label output is written on completion by default; `--preserve-order` opts into the
  previous reorder-buffer behavior.

### Fixed

- Custom-SFEN match logs continue the SFEN move count and reject kings in hand without panicking.

## [0.6.0] — 2026-07-05

### Added

- `calibrate`, `audit`, and `tune` measure coverage and teacher/student disagreement before a
  filter threshold is selected; presets transfer the chosen configuration without transcription.
- Cache `stats`, `verify`, and dry-run-by-default `prune` commands inspect label caches.
- `from-match` imports match-runner games, and `merge-observations` combines labeling passes.
- Label manifests report throughput, cache use, and engine timing diagnostics.

### Changed

- Cache entries use a backward-compatible metadata envelope.

## [0.5.0] — 2026-07-05

### Added

- Requested-depth provenance, underreach gates, selection strategies, content-addressed label
  cache, engine fingerprinting, variation root IDs, score perspective, and special-bestmove kinds.
- `split --max-open-writers` bounds file descriptors for many-source corpora.

### Changed

- Label and validate paths became streaming; deterministic hashes moved to BLAKE3.
- Train/valid/test split groups KIF variations with their source root.
- JSONL schema and pack format advanced through 6, 7, and 8.

## [0.4.0] — 2026-07-04

### Added

- MultiPV candidates, policy margins, score bounds, cross-engine disagreement, source-aware split,
  quality explanations, dry-run filtering, run manifests, and richer reports.
- Cross-platform CI, cargo-audit, and core benchmarks.

### Fixed

- KIF same-square notation, USI timeout handling, achieved-depth reporting, split I/O errors,
  variation leakage, and bounded-score handling.

### Changed

- Shared quality evaluation replaced duplicated CLI gates.
- JSONL schema and pack format advanced through 2, 3, 4, and 5.

## [0.3.0] — 2026-06-28

### Added

- KIF ingestion, binary pack/unpack, stability calculation, hard-position mining, balancing,
  source split, sampling, parallel labeling, engine options, Zobrist dedup, and position tags.

## [0.2.0] — 2026-06-28

### Added

- Streaming stability/evaluation filtering and labeled-dataset reporting.

## [0.1.0] — 2026-06-28

### Added

- CSA extraction, USI labeling, validation/reporting, shared domain types, CI, fixtures, and dual
  MIT/Apache-2.0 licensing.

[Unreleased]: https://github.com/kent-tokyo/shogiesa/compare/v0.10.1...HEAD
[0.10.1]: https://github.com/kent-tokyo/shogiesa/compare/v0.10.0...v0.10.1
[0.10.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.9.2...v0.10.0
[0.9.2]: https://github.com/kent-tokyo/shogiesa/compare/v0.9.1...v0.9.2
[0.9.1]: https://github.com/kent-tokyo/shogiesa/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kent-tokyo/shogiesa/releases/tag/v0.1.0
