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
| complete | external-engine fixed-node delta | 230 paired positions and 64 retained mined records |
| pilot complete | four-arm training comparison | 48 positions per arm, three seeds, one epoch, shared validation |
| complete | external typed compatibility | immutable producer, fixture identity, and typed reader tests |
| pilot complete | teacher/search/quality calibration | 64 fixed positions, two teachers, 16 cases, acknowledged NNUE loads |
| complete | external A/B artifact interoperability | small schema-v1 smoke plus fail-closed shogiesa report |
| open | USI flakiness and rerun behavior | repeated real-engine runs, failure rate, residue, cache/output identity |
| representative run open | threshold calibration | broaden the completed 64-position pilot before choosing general defaults |
| open | representative training effect | larger corpus and budget, paired seeds, failed-run accounting |
| open | match transfer | fixed openings/opponent/budget with interval or SPRT rule |
| open | external native formats | named tool/version with round-trip and loss report |
| deferred | 10M and cross-platform resources | run only with adequate disk and matched hosts |

Rerun the local resource baseline against a clean, prebuilt release binary:

```bash
cargo build --release -p shogiesa-cli
python3 scripts/run_resource_baseline.py \
  --records 100000 \
  --out docs/measurements/resource_baseline_100k.json
```

The harness records wall time, sampled RSS/FD peaks, disk headroom, hashes, and output sizes. It
rejects dirty trees. The 1M mode deletes its already-hashed synthetic input after `pack` to bound
disk use; deletion is outside the measured commands.

For external comparisons, pin the engine source/version, binary and weight hashes, corpus hash,
nodes, `Threads=1`, and any engine-specific options. The version-delta harness measures output
differences, not strength. The learning-ablation harness compares equal-size recipe arms with
paired seeds; the checked-in one-epoch result is only pipeline evidence.

Teacher calibration uses one deterministic sample across declared teachers, depth/node limits,
and MultiPV values. Use `--require-weight-ack` for recorded NNUE runs so every teacher must confirm
the configured weight. Run `python3 scripts/run_teacher_calibration.py --help` for the full command.

The gate reporter consumes `ab_gate_result.v1`, validates terminal/process/count and artifact
identity fields, and passes through the upstream status without inventing a verdict.

Every run must retain commands, commits, input/output hashes, engine/weight identity, options,
seeds, hardware/OS, and failure reasons. Missing measurements remain `unverified`.
