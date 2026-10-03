# Sekirei opening-suite gate runbook

This protocol measures whether openings produced by `make-gate-openings` make Sekirei gates more
stable. shogiesa can build and describe the suite; only a real Sekirei checkout and match runner
can measure the gate effect.

## Comparison arms

| Arm | Opening source |
|---|---|
| A | `startpos` only |
| B | Sekirei production `data/gate/openings_standard.sfen` |
| C | `make-gate-openings --count 100` |
| D | `make-gate-openings --count 400` |

Build C and D from the same real position corpus. Hold `--min-ply`, `--max-ply`, and `--seed`
fixed, and retain each command and manifest. The only intended difference between C and D is suite
size.

## Measurements

Repeat every arm under the same engine, opponent, time control, hardware, and match budget. Record:

- wins/draws/losses and run-to-run variance;
- Elo estimate and confidence interval, when the runner provides them;
- Black/White win-rate bias;
- pass/fail decision stability; and
- for C/D, `distinct_roots_kept` and `max_root_share_in_any_bucket` from the manifest.

A single run cannot establish lower variance or a more reliable gate. Opening diversity is useful
only if repeated outcomes become more stable without introducing side or source-root bias.

## Execution boundary

`scripts/sekirei_dataset_ablation.sh` already defines external `SEKIREI_TRAIN_CMD` and
`SEKIREI_GATE_CMD` hooks, but its built-in arms compare dataset filters rather than opening suites.
Use the same `gate_result.json` contract in a dedicated wrapper or run the four arms manually.

This is external, compute-heavy evidence. Repository fixtures can verify suite construction and
manifest fields, but cannot verify Elo, variance reduction, or Sekirei gate decisions.
