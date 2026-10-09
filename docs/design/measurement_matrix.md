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
| open | USI flakiness and rerun behavior | repeated real-engine runs, failure rate, residue, cache/output identity |
| open | threshold calibration | fixed corpus/teacher; coverage, agreement, bounds, and drop reasons |
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

Every run must retain commands, commits, input/output hashes, engine/weight identity, options,
seeds, hardware/OS, and failure reasons. Missing measurements remain `unverified`.
