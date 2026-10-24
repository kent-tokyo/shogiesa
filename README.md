# shogiesa

> Shogi training-data forge for NNUE and other USI consumers.

shogiesa turns CSA, KIF, KI2, and match-runner records into inspectable training datasets. It
extracts SFEN positions, labels them with USI engines, filters unstable samples, and writes
reproducible JSONL or binary-pack artifacts for downstream engines and trainers.

The workspace release is `0.11.8`. See the
[GitHub Release](https://github.com/kent-tokyo/shogiesa/releases/tag/v0.11.8), the
[changelog](CHANGELOG.md), and
[the v0.11.8 validation log](docs/release_validation_2026-10-24_v0.11.8.md), which records
candidate and publication evidence separately.

## Scope

shogiesa owns the data pipeline: ingestion, extraction and conservative validation, deduplication,
external-USI labeling, quality signals, selection, reproducible manifests, JSONL/pack export, and
diagnostics. Its public boundaries are CSA/KIF/KI2, SFEN, USI, versioned JSONL, and versioned pack.

It is not an engine, search implementation, NNUE trainer, GUI, tournament manager, or cloud
training service. No specific engine, trainer, or weight file is a core dependency, and dataset
diagnostics alone do not establish training gain or Elo.

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

shogiesa label --input positions.jsonl --engine ./your-usi-engine \
  --depths 4,6,8 --out labeled.jsonl

shogiesa report --input labeled.jsonl
shogiesa filter --input labeled.jsonl --max-score-swing-cp 150 --out train.jsonl
```

Directory extraction is shallow by default. `--recursive` reads `.csa`, `.kif`, and `.ki2` in
deterministic relative-path order without following symlinks.

CLI help is the option-level authority: use `shogiesa <command> --help`.

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

For reproducible runs, retain hashes, commands, seeds, engine/weight identities, options, and
manifests. Missing identities remain `unknown`. Record, pack, manifest, and report versions are
independent; use the typed `shogiesa-core` parsers instead of copying their layouts.

## Commands

| Area | Commands | Purpose |
|---|---|---|
| Ingest | `extract`, `from-match` | Convert game records to JSONL positions. |
| Label | `label`, `cache`, `merge-observations` | Run USI teachers and manage observations. |
| Quality | `stability`, `filter`, `calibrate`, `audit`, `tune` | Attach, inspect, and calibrate quality signals. |
| Select | `select`, `mine`, `balance`, `stratify`, `sample` | Select hard or underrepresented positions. |
| Reproduce | `split`, `shuffle`, `recipe`, `dataset-diff` | Preserve roots, order, and artifact identity. |
| Diagnose | `report`, `quality-report`, `distribution`, `validate`, `conflict-report`, `block-report` | Report statistics and integrity problems. |
| Exchange | `pack`, `unpack`, `lineprior export`, `make-gate-openings` | Convert data or prepare external-tool inputs. |

`recipe` runs typed shogiesa stages only; it does not execute arbitrary shell commands.

## Data contract

JSONL is canonical. Each record contains a schema version, post-move SFEN, source, tags, and
optional observations, stability, and result data.

```json
{
  "schema_version": 11,
  "sfen": "lnsgkgsnl/1r5b1/p1ppppppp/1p7/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL b - 2",
  "source": { "kind": "csa", "path": "games/example.csa", "ply": 24 },
  "tags": { "phase": "middlegame", "side_to_move": "black", "in_check": false },
  "observations": []
}
```

Rust consumers should depend one-way on `shogiesa-core` and call
`shogiesa_core::schema::parse_json_line`. It applies documented legacy defaults and rejects
unsupported schema versions without coupling shogiesa to a trainer.
For large datasets, `PositionRecordJsonlReader<R: BufRead>` provides incremental, line-numbered
iteration without collecting the input.

Binary pack is a versioned transport format. Convert it back to JSONL for inspection or diffing.
See [schema and pack compatibility](docs/design/schema_compatibility.md).

For automated qualification, emit the same versioned semantic summary from JSONL or pack:

```bash
shogiesa quality-report --input train.jsonl --out quality.json
shogiesa quality-report --input train.shgpk --out packed-quality.json
```

The report describes corpus identity, coverage, and parse failures; it does not claim training
quality or engine strength. See [quality-report compatibility](docs/design/quality_report_compatibility.md).

## Documentation

Start with the [documentation index](docs/README.md). It separates current user guides, public
contracts, measurement protocols, and historical release evidence.

## Evidence boundary

- SFEN validation checks syntax and conservative material constraints, not full legal-move reachability.
- The checked-in 100k/1M resource runs use synthetic records on one macOS host. They do not prove
  representative-corpus or cross-platform performance.
- The three-seed training run is a 48-position, one-epoch pilot. Its small loss differences do not
  establish generalization or playing strength.
- Native external-format adapters, 10M scale, match transfer, and Elo are unmeasured.
- Provenance manifests remain producer-owned. Cross-tool chains use explicit hashes and adapters;
  shogiesa does not publish a shared cross-repository envelope.

## Development

```bash
bash scripts/check_repository_contract.sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
