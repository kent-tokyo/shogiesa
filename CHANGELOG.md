# Changelog

Notable changes to shogiesa follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Compare links at the end provide the complete commit history.

## [Unreleased]

## [0.11.3] — 2026-10-11

### Added

- Run manifests now emit `manifest_schema_version: 1`, independently from the
  position-record and pack versions. The compatibility policy and canonical
  label-manifest fixture let downstream consumers accept additive fields while
  failing closed on unknown breaking versions.

### Changed

- Retired the unused nested experiment-envelope draft. Provenance fields remain part of
  shogiesa-owned manifests, while downstream tools keep versioned, producer-owned contracts and
  connect artifacts through explicit hashes.

### Fixed

- The lineprior dogfood report now reads the real `topk_hit_rate` and
  `mean_reciprocal_rank` EvalReport fields, validates missing k=3/k=5/MRR independently, records
  the external binary identity, and rejects single-sequence held-out runs before tuning.
- The dogfood runner now handles executable paths containing spaces, rejects missing option
  values cleanly, refuses to mix a rerun with existing artifacts, and writes its report atomically.

## [0.11.2] — 2026-10-10

### Added

- Added a teacher-calibration harness that runs a deterministic depth/node, MultiPV, and
  teacher-count matrix, records per-case coverage/bounds/timeouts, and evaluates fixed quality
  profiles with machine-readable drop reasons. Dirty-tree runs are explicitly marked as
  candidates and cannot be mistaken for release evidence.
- Added a fail-closed Sekirei `ab_gate_result.v1` reporter with fixture-backed schema, terminal
  state, game-count, and provenance validation. It preserves the upstream verdict instead of
  recomputing one from Elo fields.

### Changed

- Updated the pinned Sekirei compatibility target to v0.3.68 and its released
  `shogiesa-core` 0.11.1 contract. New version-delta and learning-ablation runs default to the
  v0.3.67/v0.3.68 pair; historical 0.3.65/v0.3.66 artifacts remain unchanged.

### Fixed

- `label` now rejects malformed, duplicate, empty, and non-positive depth/node limits instead of
  silently dropping or repeating parts of the requested search matrix. Zero timeouts, jobs, and
  MultiPV values are also rejected.
- `label --engine-option` now fails on malformed or duplicate names and requires the dedicated
  `--multipv` flag for MultiPV, keeping engine options and observation/cache identity aligned.
- `filter`, `calibrate`, and `tune` now reject unknown, empty, or duplicate phase names and
  impossible quality ranges instead of silently changing or emptying the selected dataset.
- `audit` and `tune` now require positive, distinct student depths that are strictly shallower
  than the teacher depth; calibration sweeps reject duplicate values and negative score swings.
- File-producing CLI commands now reject input/output aliases and colliding sidecars before
  opening any output. Relative aliases, symlinks, and hard links can no longer truncate a source
  dataset or replace a completed output with its manifest.
- `split --by-source` now disambiguates source paths that sanitize to the same file name and
  rejects dynamically generated outputs that alias the input or reserved manifest path.
- `split --by-source` now opens staged bundle files with the writable handle required by Windows
  before calling `sync_all`, preserving the transactional commit path across supported systems.
- Recipe planning now rejects JSON reports or declared stage outputs that alias the recipe file,
  including aliases reached through filesystem links.
- Dataset transforms, reports, manifests, and pack conversion now write through same-directory
  temporary files and replace their destinations only after a successful flush. Failed commands
  preserve previous outputs; `split --by-source` stages and validates its complete file/manifest
  bundle before a backup-backed commit. Resumable `label` output remains intentionally
  incremental.
- Teacher calibration now validates the complete input before sampling, rejects any pre-labeled
  source records, reports concise failures, and writes the final artifact atomically. An optional
  strict preflight verifies that each teacher acknowledges the declared NNUE weight load.
- Shell validation no longer stops at a malformed ShellCheck directive, and release readiness now
  exits immediately if it cannot enter the repository root.
- Measurement artifacts now use shared same-directory atomic replacement and reject aliases with
  their corpus, shogiesa binary, or sibling output, including symlink and hard-link aliases.
- The Sekirei compatibility check now rejects missing option values and canonical-fixture output
  collisions, supports Git worktree checkouts, and publishes JSON artifacts atomically without
  replacing an existing output symlink. It can validate an immutable shogiesa tag independently
  of a dirty development checkout.

## [0.11.1] — 2026-10-10

- `validate --strict` now rejects valid JSON that is not a valid typed `PositionRecord`, including
  unsupported future schema versions and missing required fields, and reports those separately
  from malformed JSON.
- Added a tag-pinned Sekirei compatibility harness that compares the canonical schema fixture and
  runs the released trainer's typed position-reader tests without modifying its checkout.
- Added a deterministic local resource-baseline harness for 100k/1M streaming measurements with
  wall-time, sampled RSS/FD, disk, binary, and artifact provenance.
- Recorded clean-release 100k and 1M resource baselines, a fixed-node Sekirei 0.3.65/0.3.66
  comparison with 64 retained mined positions, and a four-arm three-seed training pilot.
- Added reproducible harnesses for the Sekirei version delta and training ablation; both pin source
  tags, dataset identities, search/training controls, and claim limits.
- Reused USI sessions now place a command-order barrier between completed searches in both strict
  and compatibility modes, preventing a late duplicate `bestmove` from being mistaken for the
  next position's response.

## [0.11.0] — 2026-10-03

### Added

- Added the consumer-facing `shogiesa_core::schema` boundary with typed JSONL parsing,
  explicit schema-version validation, and actionable unsupported-version diagnostics.
- Added canonical schema 11 and legacy schema 1 JSONL fixtures covering cp and mate scores,
  score perspective and bounds, stability, game-result provenance, and KIF variation metadata.

### Changed

- Routed CLI position-record readers through the version-checked schema parser.

### Fixed

- Strict USI mode now uses a command-order barrier between searches so an immediately delayed
  duplicate `bestmove` cannot race the next position.

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

[Unreleased]: https://github.com/kent-tokyo/shogiesa/compare/v0.11.3...HEAD
[0.11.3]: https://github.com/kent-tokyo/shogiesa/compare/v0.11.2...v0.11.3
[0.11.2]: https://github.com/kent-tokyo/shogiesa/compare/v0.11.1...v0.11.2
[0.11.1]: https://github.com/kent-tokyo/shogiesa/compare/v0.11.0...v0.11.1
[0.11.0]: https://github.com/kent-tokyo/shogiesa/compare/v0.10.1...v0.11.0
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
