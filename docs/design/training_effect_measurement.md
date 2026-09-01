# Training-effect measurement record

This document defines the minimum evidence for comparing shogiesa dataset recipes in a real
trainer. It is a measurement protocol, not a claim that any filter improves NNUE strength.

## Experimental controls

Use one fixed source-root train/valid/test split and reuse its manifest for every arm. Keep the
teacher engine binary, weight hash, options, search limit, label schema, model architecture,
optimizer, learning-rate schedule, batch size, total steps/epochs, validation records, and
hardware constant. Run at least three training seeds per arm; record failed or incomplete runs
instead of silently removing them.

The only intended independent variable is the dataset recipe: baseline, filtered, mined, or
balanced. If corpus size differs, report both the raw result and a cost/position-count context;
do not call a larger dataset an unconditional quality improvement.

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
| `validation_loss_final` | final validation loss, with metric definition |
| `validation_loss_best` | best validation loss and step |
| `validation_wdl` | fixed WDL evaluation on the same validation set |
| `label_wall_time_sec` | cost of producing labels |
| `train_wall_time_sec` | cost of training |
| `status` | `complete`, `failed`, or `incomplete` with a reason |

## Analysis rules

Report per-seed values, median, and spread for each arm. Compare arms on the same validation
positions and report the paired difference for each seed where pairing is valid. A lower loss or
higher WDL is evidence about that fixed training setup, not a general claim about engine strength.
Keep data quality diagnostics (`conflict-report`, `block-report`, coverage, bound rate) separate
from training/search outcomes so a search change cannot be misattributed to dataset quality.

The minimum completion gate is: all complete runs use the same split and training budget, at
least three seeds are present per compared arm, failed runs are accounted for, and the conclusion
does not rely on a single seed or a single aggregate score.
