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

The output directory must not contain artifacts from an earlier run. The script refuses to
overwrite them, preventing a failed rerun from mixing new observations with stale evaluation
results. A successful directory contains the exported observations and manifest, lineprior
tune/eval JSON, the selected configuration, and atomically written `report.md`. The report retains
shell-quoted rerun commands plus the external lineprior version and binary SHA-256.

At least two distinct sequences are required for the held-out sequence split. Use a larger corpus
in practice: even with two or more sequences, a deterministic 80/20 assignment can leave one side
empty, in which case the script reports an actionable tune/eval error.

## Read the result

Inspect `coverage`, `fallback_rate`, `top1_hit_rate`, `top3_hit_rate`, `top5_hit_rate`, and `mrr`.
The Markdown labels map to lineprior's `topk_hit_rate[]` entries and
`mean_reciprocal_rank`; they are not expected as separate top-level JSON fields.
For candidate ordering, `top5_hit_rate` and `mrr` matter more than top-1 accuracy. A useful prior
must also cover enough searched positions without relying excessively on fallback.

Use `--strict-report-fields` for recorded runs. It fails when coverage, fallback, top-1, k=3, k=5,
or MRR is absent or non-numeric. Without it, a contract mismatch may render a metric as `n/a`
while the script still exits successfully. Do not interpret `n/a` as zero.

`unknown_outcome_count` is not automatically an error. KIF variation moves intentionally have an
unknown game outcome because the branch was not the played game.

## Decision gate

Prototype prior-guided Sekirei move ordering only when repeated runs show adequate coverage,
reasonable fallback, and useful top-5/MRR results. Otherwise adjust the corpus, source, ply range,
split, or lineprior threshold first. Search integration and strength testing belong to Sekirei and
remain outside this repository.
