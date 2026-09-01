# Dataset recipe template

This is a reproducibility template for comparing dataset recipes before measuring their effect in
Sekirei training. It describes shogiesa outputs and command arguments; the trainer owns optimizer,
architecture, training seed, and budget. Replace every `<...>` value and keep the generated
manifests beside the corresponding JSONL files.

## Fixed inputs and identities

```text
corpus_input: <absolute-or-pinned-relative-path>
corpus_input_hash: <split-manifest.input_hash>
split_seed: <u64>
valid_frac: <f64>
test_frac: <f64>
teacher_engine: <engine name and version>
teacher_binary_hash: <label manifest binary_sha256>
teacher_weight_hash: <label manifest weight_sha256 or unknown>
teacher_options: <exact --engine-option list>
label_limit: <depths or nodes>
label_multipv: <u32>
```

The source-root split is created once and reused by every arm:

```bash
shogiesa split \
  --input <positions.jsonl> \
  --train <split/train.jsonl> \
  --valid <split/valid.jsonl> \
  --test <split/test.jsonl> \
  --valid-frac <valid_frac> --test-frac <test_frac> --seed <split_seed>
```

Keep `split/manifest.json`. Its `source_root_ids` arrays are the leakage check: pairwise
intersections must be empty, and its input/output hashes identify the exact split.

## Recipe arms

Each arm starts from the same labeled split and uses the same teacher, weight, options, label
limits, training seed(s), validation split, architecture, optimizer, and training budget. Only the
dataset transformation under comparison may change.

| arm | shogiesa transformation | required artifacts |
|---|---|---|
| baseline | no quality filter, or the pre-registered baseline filter | label manifest, output hash |
| filtered | `filter --preset` or fixed filter flags | filter manifest, output hash |
| mined | `mine` from the same labeled input | mining args, output hash |
| balanced | `balance` or `stratify --quota` from the same labeled input | balance/stratify manifest, quota JSON |

For a threshold arm, prefer a machine-readable preset produced by `tune --preset-out`:

```bash
shogiesa filter --input <labeled.jsonl> --out <filtered.jsonl> \
  --preset <tuning.json:balanced> --manifest <filtered.manifest.json>
```

Do not compare a hand-transcribed Markdown threshold against a preset and call them the same arm.
Record the exact command line from each manifest.

## Training and evaluation record

```text
recipe_id: <baseline|filtered|mined|balanced>
input_hash: <manifest input hash>
output_hash: <manifest output hash or command-specific output hash>
split_manifest_hash: <hash of split/manifest.json when used externally>
teacher_manifest_hash: <hash of label manifest>
training_seed: <u64; trainer-owned>
training_steps_or_epochs: <fixed budget>
optimizer_and_schedule: <fixed config>
model_architecture: <fixed config>
validation_split: <split/valid.jsonl hash>
validation_loss: <measured by trainer>
validation_wdl: <measured by trainer>
label_cost: <wall time and resource envelope>
```

`validity` and any single score are not acceptance criteria. Report data volume, label cost,
quality diagnostics, and training/search results separately. A recipe is reproducible only when
all hashes, seeds, teacher/weight/options, and trainer budget fields are present or explicitly
`unknown`.
