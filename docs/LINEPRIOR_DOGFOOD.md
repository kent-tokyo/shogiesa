# lineprior dogfood runbook

This runbook measures whether historical move priors are useful enough to test in Sekirei.
shogiesa exports data; it does not integrate `lineprior` into search. `lineprior` is an external
binary and is not a dependency of this repository.

## Run

Prerequisites: a release build of shogiesa, a separately built `lineprior` binary, and `jq`.

```bash
scripts/lineprior_dogfood.sh \
  --games ./games \
  --lineprior /path/to/lineprior \
  --out runs/lineprior-shogi-001 \
  --source teacher_v012 \
  --max-ply 80 \
  --strict-report-fields
```

The output directory contains the exported observations and manifest, lineprior tune/eval JSON,
the selected configuration, and `report.md`. The report retains the commands needed to rerun the
experiment.

## Read the result

Inspect `coverage`, `fallback_rate`, `top1_hit_rate`, `top3_hit_rate`, `top5_hit_rate`, and `mrr`.
For candidate ordering, `top5_hit_rate` and `mrr` matter more than top-1 accuracy. A useful prior
must also cover enough searched positions without relying excessively on fallback.

Use `--strict-report-fields` for recorded runs. Without it, a lineprior JSON-schema mismatch may
render a metric as `n/a` while the script still exits successfully. When that happens, inspect
`shogi_eval_report.json` and update the report field mapping; do not interpret `n/a` as zero.

`unknown_outcome_count` is not automatically an error. KIF variation moves intentionally have an
unknown game outcome because the branch was not the played game.

## Decision gate

Prototype prior-guided Sekirei move ordering only when repeated runs show adequate coverage,
reasonable fallback, and useful top-5/MRR results. Otherwise adjust the corpus, source, ply range,
split, or lineprior threshold first. Search integration and strength testing belong to Sekirei and
remain outside this repository.
