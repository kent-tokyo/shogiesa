# shogiesa

> Shogi training-data feed for NNUE engines.

shogiesa turns game records into inspectable, reproducible training data. It extracts SFEN
positions, labels them through USI, records quality/provenance signals, and prepares datasets for
an external trainer such as Sekirei.

Current source version: `0.10.0`. The authoritative local validation record is
[`docs/release_validation_2026-09-23.md`](docs/release_validation_2026-09-23.md); tag, GitHub
Release, and crates.io publication are separately verified release operations.

## Scope

shogiesa is a data forge, not a Shogi engine, NNUE trainer, GUI, tournament manager, or cloud
service. Its outputs are JSONL, SFEN, USI moves, manifests, and an optional binary pack—not a
claim that a dataset improves playing strength.

It provides:

- CSA/KIF and match-runner record ingestion;
- conservative SFEN validation, deduplication, source-root provenance, and diagnostics;
- USI teacher labeling with timeouts, restart handling, cache, and resume support;
- stability, quality, conflict, block, distribution, and integrity diagnostics; and
- deterministic split, quota sampling, shuffle, recipe, JSONL, and pack workflows.

## Install

```bash
git clone https://github.com/kent-tokyo/shogiesa.git
cd shogiesa
cargo build --release
# binary: target/release/shogiesa
```

## Quick start

```bash
# 1. Extract post-move positions from CSA or KIF records.
shogiesa extract --input ./games --out positions.jsonl --min-ply 20 --every-n-plies 2

# 2. Label them with a USI engine.
shogiesa label --input positions.jsonl --engine ./sekirei --depths 4,6,8 --out labeled.jsonl

# 3. Inspect and filter without guessing what the signals mean.
shogiesa report --input labeled.jsonl
shogiesa filter --input labeled.jsonl --min-stability 0.85 --out train.jsonl
```

Every option is defined by the executable:

```bash
shogiesa --help
shogiesa label --help
shogiesa recipe run --help
```

## Workflow

```text
CSA / KIF / match kifu
        ↓
extract / from-match → label → stability / audit / calibrate / tune
        ↓
filter / select / mine / balance / stratify → split / shuffle → pack
        ↓
report / distribution / validate
```

Use a fixed input, engine binary/options, weight, seed, and command line for any result you need
to reproduce. Commands that write manifests record the available identities and hashes; a missing
value remains `unknown`, not inferred.

## Commands

| Area | Commands | Purpose |
|---|---|---|
| Ingest | `extract`, `from-match` | Read CSA/KIF or match-runner kifu into JSONL positions. |
| Label | `label`, `cache`, `merge-observations` | Run USI teachers, manage cached observations, or combine passes. |
| Quality | `stability`, `filter`, `calibrate`, `audit`, `tune` | Attach and inspect instability/quality signals; choose gates from data. |
| Select | `select`, `mine`, `balance`, `stratify`, `sample` | Find hard/underrepresented positions or make a bounded sample. |
| Reproduce | `split`, `shuffle`, `recipe plan/run/verify`, `dataset-diff` | Keep source roots together, preserve deterministic order, and compare artifacts. |
| Diagnose | `report`, `distribution`, `validate`, `conflict-report`, `block-report` | Summarize data, surface missing buckets, and report integrity or proxy diagnostics. |
| Exchange | `pack`, `unpack`, `lineprior export`, `make-gate-openings` | Convert data or prepare inputs for external tools. |

`recipe` accepts only typed shogiesa stages; it does not run arbitrary shell commands. `recipe run`
writes stage output through staging and reuses only matching successful artifacts. See
[`docs/design/dataset_recipe_template.md`](docs/design/dataset_recipe_template.md) for the
experiment record to keep alongside a run.

## Data contract

JSONL is the canonical format because it is streamable and reviewable. Each position has a schema
version, post-move SFEN, source information, tags, and optional observations/stability/result data.

```json
{
  "schema_version": 11,
  "sfen": "lnsgkgsnl/1r5b1/p1ppppppp/1p7/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL b - 2",
  "source": { "kind": "csa", "path": "games/example.csa", "ply": 24 },
  "tags": { "phase": "middlegame", "side_to_move": "black", "in_check": false, "has_capture": true },
  "observations": []
}
```

Binary pack is a transport format with a magic header, version, endian definition, and unpack
path. Inspect or diff JSONL; do not edit pack bytes as a primary format. Compatibility and error
classes are in [`docs/design/schema_compatibility.md`](docs/design/schema_compatibility.md).

## Reading diagnostics safely

`score.cp`, policy margin, stability, agreement, and `QualityDecision.score` are diagnostics—not
probabilities, labels of move correctness, or evidence of engine strength. Thresholds must be
calibrated against a fixed corpus and teacher configuration. See [`docs/THEORY.md`](docs/THEORY.md).

KIF variation moves are preserved as separate source paths and share a `root_id` with the mainline.
An indented nested `変化` replays from its parent and keeps its full lineage (for example,
`#var1@2#var2@3` and `variation_id: "var1.var2"`); equal-or-shallower markers are mainline-rooted
siblings. KIF branch outcomes are `unknown` because a branch is not the played game.

## Documentation map

| Need | Document |
|---|---|
| Current work and explicit measurement gates | [`ROADMAP.md`](ROADMAP.md) |
| Schema and pack compatibility | [`docs/design/schema_compatibility.md`](docs/design/schema_compatibility.md) |
| Rust API boundary | [`docs/api_boundary.md`](docs/api_boundary.md) |
| Metrics and quality-signal limits | [`docs/THEORY.md`](docs/THEORY.md) |
| Interoperability claims and gaps | [`docs/interop_evidence.md`](docs/interop_evidence.md) |
| Training-effect and gate protocols | [`docs/design/training_effect_measurement.md`](docs/design/training_effect_measurement.md), [`docs/SEKIREI_GATE_EVALUATION.md`](docs/SEKIREI_GATE_EVALUATION.md) |
| External lineprior experiment | [`docs/LINEPRIOR_DOGFOOD.md`](docs/LINEPRIOR_DOGFOOD.md) |
| Release evidence and checklist | [`docs/release_validation_2026-09-23.md`](docs/release_validation_2026-09-23.md), [`docs/release_checklist.md`](docs/release_checklist.md) |
| Feature-fit comparison | [`docs/competitor_evidence.md`](docs/competitor_evidence.md) |

## Limits and evidence boundary

- SFEN checks syntax and conservative material constraints; it is not full legal-move generation.
- Native interoperability with GenSfen, rshogi, cshogi, rsshogi, and python-shogi is unmeasured.
- Throughput, RSS, training effect, match results, and Elo are unmeasured unless a dated result
  records corpus, commit, hardware, engine/weight, and budget.
- The cross-repository experiment envelope is a shogiesa-owned draft, not a shared standard.

## Development

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
bash scripts/check_repository_contract.sh
```

The contract check is lightweight. `scripts/release_readiness.sh` additionally runs the cargo
checks and reports unavailable dependencies or network failures as failures, not success.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
