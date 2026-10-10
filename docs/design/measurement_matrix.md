# Measurement status and remaining gates

This table separates completed measurements from open gates. Results apply only to the environment
and inputs recorded in their artifacts.

For changes that do not require external engines, start with
`bash scripts/run_local_measurement_smoke.sh`. It validates the repository contract, formatting,
and deterministic fixture-backed regressions. A PASS is local regression evidence; it does not
complete scale, training, or external-interoperability gates.

| Status | Area | Evidence or completion requirement |
|---|---|---|
| complete | path/order/worker reproducibility | [`../measurements/reproducibility_matrix_2026-10-03.json`](../measurements/reproducibility_matrix_2026-10-03.json) |
| complete | 100k/1M local resource use | dated clean-release artifacts in [`../measurements/`](../measurements/README.md) |
| complete | Sekirei 0.3.65/0.3.66 fixed-node delta | 230 paired positions and 64 retained mined records |
| pilot complete | four-arm training comparison | 48 positions per arm, three seeds, one epoch, shared validation |
| complete | Sekirei 0.3.68 typed compatibility | immutable shogiesa 0.11.1 producer, fixture identity, 13 typed reader tests |
| pilot complete | teacher/search/quality calibration | 64 fixed positions, two teachers, 16 cases, acknowledged NNUE loads |
| complete | Sekirei A/B artifact interoperability | four-game schema-v1 smoke plus fail-closed shogiesa report |
| open | USI flakiness and rerun behavior | repeated real-engine runs, failure rate, residue, cache/output identity |
| representative run open | threshold calibration | broaden the completed 64-position pilot before choosing general defaults |
| open | representative training effect | larger corpus and budget, paired seeds, failed-run accounting |
| open | match transfer | fixed openings/opponent/budget with interval or SPRT rule |
| open | external native formats | named tool/version with round-trip and loss report |
| deferred | 10M and cross-platform resources | run only with adequate disk and matched hosts |

Run the resource baseline against a prebuilt release binary:

```bash
cargo build --release -p shogiesa-cli
python3 scripts/run_resource_baseline.py \
  --records 100000 \
  --out docs/measurements/resource_baseline_100k.json
```

The harness records wall time, sampled peak RSS/FD count, disk headroom, hashes, and output sizes.
It rejects dirty trees. At 1M it removes the hashed synthetic input after `pack` to bound disk use;
the deletion is outside every measured command.

`scripts/run_sekirei_version_delta.py` compares immutable Sekirei tags with one sampled corpus,
fixed nodes, `Threads=1`, and `SpecTopN=0`. It records selected-input hashes, search disagreement,
cp deltas, and optional mined JSONL. This measures output differences, not strength.

`scripts/run_sekirei_learning_ablation.py` creates equal-size baseline, filtered, uncertain-mined,
and phase-balanced arms from one source split. It runs three paired seeds against frozen
validation using a completed fixed-depth teacher search. The checked-in one-epoch result is
pipeline evidence, not a generalization or playing-strength result.

`scripts/run_teacher_calibration.py` labels one deterministic sample across depth/node limits,
MultiPV values, and an ordered list of teachers. It records per-case coverage, score bounds,
timeouts, depth underreach, policy-margin coverage, teacher-prefix agreement and CP spread, plus
drop reasons for four fixed quality profiles. Optional `--weight NAME=PATH` arguments bind weight
hashes to teachers. With `--require-weight-ack`, it also fails unless every teacher reports a
successful NNUE load and the configured EvalFile bytes match every declared weight. Before
sampling, it runs strict validation over the complete input and rejects
pre-labeled records, so malformed or stale observations cannot disappear outside the selected
sample. The final JSON artifact is replaced atomically. A representative run still needs real
teachers and a fixed corpus; the harness alone does not complete the calibration gate.

```bash
python3 scripts/run_teacher_calibration.py \
  --input positions.jsonl \
  --teacher sekirei-0.3.68=/path/to/sekirei \
  --weight sekirei-0.3.68=/path/to/weights.bin \
  --depths 4,8 --nodes 1000,10000 --multipv 1,3 \
  --engine-option EvalFile=/path/to/weights.bin \
  --engine-option Threads=1 --engine-option SpecTopN=0 \
  --require-weight-ack --out teacher-calibration.json
```

`scripts/report_sekirei_gate.py` consumes Sekirei's `sekirei.ab_gate_result.v1`. It validates
terminal state, W/D/L totals, process completion, clean runner identity, binary/weight/opening
hashes, and then passes through the upstream status without deriving its own verdict.

Every run must retain commands, commits, input/output hashes, engine/weight identity, options,
seeds, hardware/OS, and failure reasons. Missing measurements remain `unverified`.
