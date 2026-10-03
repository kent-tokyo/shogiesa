# shogiesa

> Shogi training-data feed for NNUE engines.

shogiesa turns CSA, KIF, KI2, and match-runner records into inspectable training datasets. It
extracts SFEN positions, labels them through USI engines, filters unstable samples, and writes
reproducible JSONL or binary-pack artifacts for external trainers such as Sekirei.

The workspace version is `0.10.1`; the latest verified published release is `0.10.0` until the
0.10.1 publication workflow completes. See the [changelog](CHANGELOG.md) and
[the v0.10.1 validation log](docs/release_validation_2026-10-03.md).

## Scope

shogiesa creates and inspects data. It is not a Shogi engine, NNUE trainer, GUI, tournament
manager, or cloud service. Dataset diagnostics are not evidence of training gain or Elo.

Main capabilities:

- CSA/KIF/KI2 and match-runner ingestion;
- SFEN validation, deduplication, variation-aware provenance, and diagnostics;
- USI labeling with MultiPV, timeouts, restart handling, cache, and resume;
- stability, quality, conflict, distribution, and integrity reports; and
- deterministic split, sampling, shuffle, recipe, JSONL, and pack workflows.

## Install

```bash
git clone https://github.com/kent-tokyo/shogiesa.git
cd shogiesa
cargo build --release
```

The CLI binary is `target/release/shogiesa`.

## Quick start

```bash
shogiesa extract --input ./games --recursive --out positions.jsonl \
  --min-ply 20 --every-n-plies 2

shogiesa label --input positions.jsonl --engine ./sekirei \
  --depths 4,6,8 --out labeled.jsonl

shogiesa report --input labeled.jsonl
shogiesa filter --input labeled.jsonl --max-score-swing-cp 150 --out train.jsonl
```

Directory extraction is shallow unless `--recursive` is specified. Recursive mode reads `.csa`,
`.kif`, and `.ki2` files in deterministic relative-path order, records relative source paths,
and does not follow symlinks. An input root that is itself a symlink is rejected.

The executable is the option-level authority:

```bash
shogiesa --help
shogiesa extract --help
shogiesa label --help
shogiesa recipe run --help
```

## Workflow

```text
CSA / KIF / KI2 / match kifu
        ↓
extract / from-match → label → stability / audit / calibrate / tune
        ↓
filter / select / mine / balance / stratify → split / shuffle → pack
        ↓
report / distribution / validate / dataset-diff
```

For reproducible runs, retain the input hash, command line, seed, engine binary/options, weight,
and generated manifests. Missing identities remain `unknown`; they are never inferred.

## Commands

| Area | Commands | Purpose |
|---|---|---|
| Ingest | `extract`, `from-match` | Convert game records to JSONL positions. |
| Label | `label`, `cache`, `merge-observations` | Run USI teachers and manage observations. |
| Quality | `stability`, `filter`, `calibrate`, `audit`, `tune` | Attach, inspect, and calibrate quality signals. |
| Select | `select`, `mine`, `balance`, `stratify`, `sample` | Select hard or underrepresented positions. |
| Reproduce | `split`, `shuffle`, `recipe`, `dataset-diff` | Preserve roots, order, and artifact identity. |
| Diagnose | `report`, `distribution`, `validate`, `conflict-report`, `block-report` | Report statistics and integrity problems. |
| Exchange | `pack`, `unpack`, `lineprior export`, `make-gate-openings` | Convert data or prepare external-tool inputs. |

`recipe` runs typed shogiesa stages only; it does not execute arbitrary shell commands.

## Data contract

JSONL is the canonical format. Each record contains a schema version, post-move SFEN, source,
tags, and optional observations, stability, and result data.

```json
{
  "schema_version": 11,
  "sfen": "lnsgkgsnl/1r5b1/p1ppppppp/1p7/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL b - 2",
  "source": { "kind": "csa", "path": "games/example.csa", "ply": 24 },
  "tags": { "phase": "middlegame", "side_to_move": "black", "in_check": false, "has_capture": true },
  "observations": []
}
```

Binary pack is a versioned transport format. Convert it back to JSONL for inspection or diffing.
See [schema and pack compatibility](docs/design/schema_compatibility.md).

## Documentation

| Need | Document |
|---|---|
| Signal definitions and limits | [THEORY.md](docs/THEORY.md) |
| JSONL and pack compatibility | [schema_compatibility.md](docs/design/schema_compatibility.md) |
| Rust API boundary | [api_boundary.md](docs/api_boundary.md) |
| Local interoperability evidence | [interop_evidence.md](docs/interop_evidence.md) |
| Reproducible recipe record | [dataset_recipe_template.md](docs/design/dataset_recipe_template.md) |
| Scale and training measurements | [measurement_matrix.md](docs/design/measurement_matrix.md), [training_effect_measurement.md](docs/design/training_effect_measurement.md) |
| Completed measurement artifacts | [reproducibility matrix](docs/measurements/reproducibility_matrix_2026-10-03.json) |
| Sekirei and lineprior runbooks | [SEKIREI_GATE_EVALUATION.md](docs/SEKIREI_GATE_EVALUATION.md), [LINEPRIOR_DOGFOOD.md](docs/LINEPRIOR_DOGFOOD.md) |
| Release checks and evidence | [release_checklist.md](docs/release_checklist.md), [v0.10.1 validation](docs/release_validation_2026-10-03.md) |

## Evidence boundary

- SFEN validation checks syntax and conservative material constraints, not full legal-move reachability.
- Native interoperability with GenSfen, rshogi, cshogi, rsshogi, and python-shogi is unmeasured.
- Throughput, RSS, training effect, match results, and Elo remain unverified without a dated run
  that records corpus, commit, hardware, engine/weight, and budget.
- The experiment envelope is a shogiesa-owned draft, not a shared cross-repository standard.

## Development

```bash
bash scripts/check_repository_contract.sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
