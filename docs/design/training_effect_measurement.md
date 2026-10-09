# Training-effect measurement record

This document defines the minimum evidence for comparing dataset recipes in a real trainer. A
lower validation loss does not by itself establish NNUE playing strength.

## Completed pilot

The 2026-10-10 Sekirei 0.3.66 run compared baseline, filtered, uncertain-mined, and phase-balanced
arms. Each arm used 48 positions, three paired seeds, one epoch, and the same 106-position
validation set. Mean `valid_cp_mse` ranged from 170821 to 170847, less than 0.02% apart. This proves
the comparison path runs end to end; it does not establish an arm ranking. See the
[`sekirei_learning_ablation_2026-10-10.json`](../measurements/sekirei_learning_ablation_2026-10-10.json)
artifact for per-seed values and hashes.

## Experimental controls

Reuse one source-root train/valid/test split for every arm. Hold the teacher, weight, options,
search limit, schema, model, optimizer, schedule, budget, validation records, and hardware fixed.
Run at least three seeds per arm and retain failed or incomplete runs.

Only the dataset recipe should change. If arm sizes differ, report the sizes and cost instead of
calling the larger arm an unconditional quality improvement.

## Required result table

One row is one `(recipe_id, training_seed)` run. `unknown` is allowed only when the measurement
was not collected; blank values must not mean both zero and missing.

| field | meaning |
|---|---|
| `recipe_id` | stable arm name from `dataset_recipe_template.md` |
| `training_seed` | trainer initialization/shuffle seed |
| `input_hash` / `output_hash` | shogiesa artifact identities |
| `split_manifest_hash` | exact shared split identity |
| `teacher_manifest_hash` | exact labeling run identity |
| `positions_train` / `positions_valid` | records consumed by the trainer |
| validation metric(s) | final/best value with an exact definition |
| `label_wall_time_sec` | cost of producing labels |
| `train_wall_time_sec` | cost of training |
| `status` | `complete`, `failed`, or `incomplete` with a reason |

## Analysis rules

Report per-seed values and spread. Use paired differences only when seeds and validation positions
match. Keep quality diagnostics separate from training/search outcomes.

Completion requires one split and budget, at least three seeds per arm, failed-run accounting, and
a conclusion that does not depend on one seed or one aggregate.
