# Measurement artifacts

These files record completed runs from identified commits. They support only the fixed inputs and
environment written in each artifact.

| Date | Measurement | Result | Artifact |
|---|---|---|---|
| 2026-10-03 | path, input-order, and worker-count reproducibility | PASS on repository fixtures | [`reproducibility_matrix_2026-10-03.json`](reproducibility_matrix_2026-10-03.json) |
| 2026-10-09 | 100k synthetic-record resource baseline | completed | [`resource_baseline_100k_2026-10-09.json`](resource_baseline_100k_2026-10-09.json) |
| 2026-10-09 | 1M synthetic-record resource baseline | completed | [`resource_baseline_1m_2026-10-09.json`](resource_baseline_1m_2026-10-09.json) |
| 2026-10-09 | Sekirei 0.3.65/0.3.66 fixed-node delta | 114/230 bestmove disagreements | [`sekirei_delta_v0.3.65_v0.3.66_2026-10-09.json`](sekirei_delta_v0.3.65_v0.3.66_2026-10-09.json) |
| 2026-10-10 | uncertain positions from the version delta | 64 valid schema-11 records | [`sekirei_delta_v0.3.65_v0.3.66_2026-10-10_mined.jsonl`](sekirei_delta_v0.3.65_v0.3.66_2026-10-10_mined.jsonl) |
| 2026-10-10 | baseline/filtered/mined/balanced training pilot | four arms, three seeds, one epoch | [`sekirei_learning_ablation_2026-10-10.json`](sekirei_learning_ablation_2026-10-10.json) |

The resource runs are local macOS measurements over synthetic records. The Sekirei delta measures
search-output differences, not which release is stronger. The training pilot is too small to
establish generalization or Elo. See
[`../design/measurement_matrix.md`](../design/measurement_matrix.md) for remaining gates and rerun
commands.
