# Measurement artifacts

These files record completed runs from identified commits. They support only the fixed inputs and
environment written in each artifact; they are not product-strength claims.

| Date | Measurement | Result |
|---|---|---|
| 2026-10-03 | path, input-order, and worker-count reproducibility | PASS on repository fixtures |
| 2026-10-09 | 100k synthetic-record resource baseline | completed |
| 2026-10-09 | 1M synthetic-record resource baseline | completed |
| 2026-10-10 | external-engine and downstream-training pilot | completed as a limited pipeline check |
| 2026-10-10 | teacher/search/quality calibration | 64 positions, 2 teachers, 16 cases |

The resource runs are local macOS measurements over synthetic records. The external-engine delta,
training, and calibration pilots are too small to establish generalization, universal thresholds,
or Elo. See [`../design/measurement_matrix.md`](../design/measurement_matrix.md) for remaining
gates and rerun commands.
