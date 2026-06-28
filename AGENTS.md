# AGENTS.md

This document defines the development guidance for AI coding agents working on **shogiesa**.

## Project Overview

**shogiesa** is a Shogi training-data feed for NNUE engines.

In short: **将棋の餌です。**

The goal of shogiesa is to build clean, balanced, reproducible training datasets for Shogi engines such as **Sekirei**. It extracts positions from game records, labels them with USI engines, filters unstable samples, and exports training-ready datasets.

shogiesa is not a Shogi engine.
shogiesa is not an NNUE trainer.
shogiesa is a data forge for making better training material.

## Core Concept

shogiesa exists to answer one question:

> What positions should Sekirei learn from?

The library should help produce high-quality datasets by combining:

* game record ingestion
* position extraction
* deduplication
* position tagging
* teacher evaluation
* instability filtering
* hard-position mining
* dataset balancing
* reproducible export
* dataset diagnostics

The project should remain focused on data quality, reproducibility, and maintainability.

## Design Principles

### 1. Keep the Scope Sharp

shogiesa should focus on dataset creation and inspection.

Good responsibilities:

* parse game records
* extract SFEN positions
* deduplicate positions
* classify positions
* run USI engines as teachers
* collect evaluation observations
* compute stability-related metadata
* export JSONL / binary datasets
* report dataset statistics

Avoid expanding shogiesa into:

* a full Shogi engine
* a search algorithm playground
* a GUI application
* a general tournament manager
* a full NNUE trainer
* a distributed training framework

If a feature belongs to engine strength testing, SPSA tuning, or tournament orchestration, consider a separate tool such as `usirank`.

### 2. Prefer Simple, Typed, Inspectable Data

Training data pipelines are easy to corrupt silently.

Prefer explicit structs, versioned schemas, and readable intermediate formats.

Recommended intermediate format:

```json
{
  "schema_version": 1,
  "sfen": "lnsgkgsnl/1r5b1/p1ppppppp/1p7/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL b - 2",
  "source": {
    "kind": "csa",
    "path": "games/example.csa",
    "ply": 24
  },
  "tags": {
    "phase": "middlegame",
    "side_to_move": "black",
    "in_check": false,
    "has_capture": true
  },
  "observations": [
    {
      "engine": "sekirei",
      "engine_version": "0.1.0",
      "depth": 8,
      "score_cp": 43,
      "bestmove": "7g7f",
      "nodes": 123456
    }
  ]
}
```

JSONL is preferred for early-stage development because it is easy to inspect, diff, stream, and debug.

Binary packed formats may be added later, but they must have:

* schema version
* magic header
* endian definition
* compatibility tests
* conversion tools back to JSONL or summary text

### 3. Reproducibility Matters

Every generated dataset should be explainable.

Dataset outputs should record:

* input files
* command-line arguments
* random seed
* engine path
* engine options
* engine version if available
* shogiesa version
* schema version
* filter settings
* split settings

Avoid hidden randomness.
When shuffling or sampling, require a seed or emit the generated seed in metadata.

### 4. Never Trust Raw Positions Blindly

Position extraction should be conservative.

The pipeline should validate:

* legal SFEN syntax
* side to move
* hand pieces
* move count
* duplicate positions
* impossible material
* malformed records
* illegal moves where detectable

Invalid records should not panic by default.
They should be reported with clear diagnostics and skipped unless `--strict` is enabled.

### 5. Stability Is a First-Class Signal

shogiesa should treat unstable positions as suspicious.

Examples of instability:

* best move changes across depths
* evaluation swings heavily across depths
* different teacher engines disagree strongly
* tactical positions where shallow labels are misleading
* positions in check
* immediate capture-heavy positions
* positions with very low label margin
* positions with high label entropy

The project may integrate with `quietset` or emit data suitable for quietset.

Do not hard-code one definition of “good position.”
Instead, expose stable metadata so users can filter by their own thresholds.

### 6. Hard Positions Are Valuable

shogiesa should support hard-position mining.

Sources of hard positions:

* games lost by Sekirei
* positions where evaluation changed before a blunder
* positions where teacher and student disagree
* positions with high policy disagreement
* positions where search depth changes the best move
* positions with poor validation performance

The goal is not only to collect common positions.
The goal is to collect positions that help Sekirei improve.

### 7. Dataset Balance Is Important

Avoid producing datasets dominated by:

* opening book positions
* repeated quiet early-game positions
* one-sided winning positions
* nearly identical positions
* one phase of the game
* one source engine
* one evaluation range

Useful balancing dimensions include:

* game phase
* ply range
* material balance
* side to move
* evaluation bucket
* tactical / quiet flag
* in-check flag
* capture availability
* promotion-zone pressure
* source type

Balancing should be configurable, not magical.

## Expected Repository Structure

A good initial structure:

```text
shogiesa/
  Cargo.toml
  README.md
  AGENTS.md
  crates/
    shogiesa-core/
    shogiesa-csa/
    shogiesa-usi/
    shogiesa-cli/
```

Possible future crates:

```text
crates/
  shogiesa-kif/
  shogiesa-pack/
  shogiesa-report/
  shogiesa-mine/
```

Start small.
Do not create many empty crates before the core workflow works.

## Core Crates

### shogiesa-core

Responsible for shared domain types.

Examples:

* `PositionRecord`
* `PositionId`
* `Sfen`
* `Observation`
* `TeacherEval`
* `PositionTags`
* `DatasetManifest`
* `SchemaVersion`

This crate should avoid heavy dependencies.

### shogiesa-csa

Responsible for CSA ingestion.

It should parse game records, replay moves if needed, and emit extracted positions with source metadata.

Malformed records should produce structured errors.

### shogiesa-usi

Responsible for interacting with USI engines.

It should support:

* launching an engine
* sending `usi`
* sending `isready`
* setting options
* sending `position sfen ...`
* sending `go depth N`
* parsing `info`
* parsing `bestmove`
* timeout handling
* clean shutdown

This crate must be robust.
Engine processes can hang, crash, or emit unexpected output.

### shogiesa-cli

Responsible for the user-facing command line.

Initial commands:

```bash
shogiesa extract
shogiesa label
shogiesa filter
shogiesa report
```

Future commands:

```bash
shogiesa mine
shogiesa pack
shogiesa split
shogiesa sample
```

## Suggested CLI Design

### Extract Positions

```bash
shogiesa extract \
  --input ./games \
  --format csa \
  --out positions.jsonl
```

Useful options:

```bash
--min-ply 20
--max-ply 180
--every-n-plies 2
--skip-in-check
--dedup zobrist
--strict
```

### Label Positions

```bash
shogiesa label \
  --input positions.jsonl \
  --engine ./target/release/sekirei \
  --depths 4,6,8,10 \
  --out observations.jsonl
```

Useful options:

```bash
--engine-name sekirei
--engine-option Threads=1
--engine-option Hash=128
--timeout-ms 10000
--multipv 3
--jobs 4
```

### Filter Positions

```bash
shogiesa filter \
  --input observations.jsonl \
  --min-stability 0.85 \
  --exclude-in-check \
  --out filtered.jsonl
```

Useful options:

```bash
--eval-min -1200
--eval-max 1200
--max-depth-swing 150
--require-bestmove-agreement
--phase opening,middlegame,endgame
```

### Report Dataset

```bash
shogiesa report \
  --input filtered.jsonl
```

Report should include:

* number of positions
* duplicate rate
* phase distribution
* ply distribution
* evaluation distribution
* in-check ratio
* capture ratio
* average stability
* depth disagreement
* source file counts
* skipped / invalid record counts

## Coding Style

Use idiomatic Rust.

Prefer:

* explicit error types
* small modules
* clear ownership
* streaming readers for large files
* deterministic outputs
* integration tests for CLI behavior

Avoid:

* large god modules
* panics in normal data processing
* hidden global state
* unnecessary async complexity
* premature binary format optimization
* overfitted code that only works for one local dataset

## Error Handling

Use structured errors.

Recommended crates:

* `thiserror` for library errors
* `anyhow` for CLI-level error aggregation

Library crates should not return plain strings as errors.

Bad:

```rust
return Err("invalid sfen".into());
```

Better:

```rust
return Err(SfenError::InvalidBoard { input });
```

CLI output should be human-readable and actionable.

## Testing Policy

Every important transformation should have tests.

Minimum expected tests:

* CSA parsing basics
* SFEN validation
* position extraction from a tiny known game
* deduplication behavior
* JSONL round trip
* USI info parsing
* bestmove parsing
* timeout handling
* filter threshold behavior
* report summary generation

Use small fixture files under:

```text
tests/fixtures/
```

Do not rely on large private datasets in tests.

## Performance Policy

shogiesa may process millions of positions, but correctness comes first.

Performance guidelines:

* use streaming IO
* avoid loading huge datasets fully into memory unless necessary
* expose progress logs for long-running commands
* allow parallel labeling, but keep output deterministic when possible
* benchmark only after the basic pipeline is correct

## Logging

Use `tracing`.

Default CLI output should be concise.

Verbose mode should expose:

* current input file
* processed record counts
* skipped record reasons
* engine startup status
* labeling progress
* filter counts
* output path

Avoid noisy per-position logs unless `--trace` is enabled.

## Compatibility with Sekirei

shogiesa should integrate naturally with Sekirei.

Primary output target:

```text
sekirei-train compatible JSONL or dataset format
```

Expected flow:

```bash
shogiesa extract --input ./games --out positions.jsonl

shogiesa label \
  --input positions.jsonl \
  --engine ./target/release/sekirei \
  --depths 4,6,8,10 \
  --out observations.jsonl

shogiesa filter \
  --input observations.jsonl \
  --min-stability 0.85 \
  --out train.jsonl

cargo run --release -p sekirei-train -- \
  --scored train.jsonl \
  --stability-weighted
```

Do not make shogiesa depend tightly on Sekirei internals.
Prefer stable formats such as SFEN, JSONL, and USI.

## Public API Guidelines

The Rust API should be usable without the CLI.

Good API shape:

```rust
let records = Extractor::new(config).extract_from_reader(reader)?;
let labeled = Labeler::new(engine_config).label(records)?;
```

Avoid exposing internal implementation details that would make future schema changes painful.

Version public data structures carefully.

## Documentation Requirements

README should explain:

* what shogiesa is
* what it is not
* basic pipeline
* installation
* quick start
* JSONL schema
* relationship with Sekirei
* relationship with quietset
* limitations

Each CLI command should have examples.

Every non-obvious filter should explain why it exists.

## Security and Safety

Do not execute arbitrary shell commands from dataset files.

When running USI engines:

* treat engine path as user-provided executable
* do not interpolate shell strings
* spawn processes directly
* handle timeouts
* kill child processes on failure
* avoid leaking zombie processes

Do not read or write outside requested paths.

## Dependency Policy

Keep dependencies modest.

Good candidates:

* `clap`
* `serde`
* `serde_json`
* `thiserror`
* `anyhow`
* `tracing`
* `tracing-subscriber`
* `rayon`
* `tempfile`

Avoid heavy dependencies unless they clearly improve the project.

Before adding a dependency, ask:

1. Is this needed now?
2. Can this be implemented simply?
3. Does the dependency increase compile time significantly?
4. Is the dependency actively maintained?
5. Does it complicate portability?

## Initial Milestone

The first useful version should do this:

```bash
shogiesa extract --input ./sample.csa --out positions.jsonl
shogiesa report --input positions.jsonl
```

Then:

```bash
shogiesa label --input positions.jsonl --engine ./sekirei --depths 4,6 --out observations.jsonl
```

Then:

```bash
shogiesa filter --input observations.jsonl --out filtered.jsonl
```

Do not build advanced packing, mining, or balancing before this basic loop works.

## Definition of Done

A feature is done when:

* it has tests
* it has CLI help text if user-facing
* it handles invalid input gracefully
* it has at least one fixture or example
* it does not break existing commands
* `cargo test` passes
* `cargo fmt` passes
* `cargo clippy` passes with no obvious warnings

## Agent Instructions

When working on shogiesa:

1. Inspect the existing code before making changes.
2. Prefer small, reviewable commits.
3. Keep the pipeline reproducible.
4. Add tests for every parser, converter, and filter.
5. Do not silently drop data without reporting counts.
6. Do not introduce a binary format before JSONL is stable.
7. Do not couple the project too tightly to Sekirei.
8. Keep USI integration robust against engine failures.
9. Keep README examples in sync with CLI behavior.
10. Preserve the core identity: **将棋の餌**.

## Non-Goals

shogiesa should not become:

* a full Shogi engine
* a GUI
* a cloud training service
* a general-purpose database
* a replacement for Sekirei
* a replacement for quietset
* a complete tournament platform
* a general chess/shogi variant framework

## Tagline

Recommended tagline:

```text
Shogi training data feed for NNUE engines.
```

Alternative tagline:

```text
将棋の餌。Sekirei に良質な教師局面を食わせるためのデータ生成ツール。
```
