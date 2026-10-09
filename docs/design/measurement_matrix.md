# Measurement matrix for remaining roadmap gates

This is an execution plan, not benchmark evidence. A row is complete only after its listed
artifact contains the measured result and the environment is recorded.

For changes that do not require external engines, start with
`bash scripts/run_local_measurement_smoke.sh`. It validates the repository contract, formatting,
and eight deterministic fixture-backed regression points (streaming report output, conflict
exclusions, semantic dataset diff, recipe planning/run verification, split reproducibility, pack
manifest hashes, and the path/order/worker matrix). A PASS here is local regression evidence only; it does not
complete any scale, cross-platform, training, or external-interoperability row below.

| area | fixed input/control | record | completion artifact |
|---|---|---|---|
| USI flakiness | fixture, command, runner OS/load, no retry | runs, failures, flaky rate, child-process residue | repeated-run log |
| label rerun | same corpus, engine, depths/nodes, MultiPV, options, weight | skip/replace/cache counts and output identity | paired manifest table |
| split identity | same records, reordered input, alternate path, same seed | input/output hashes, root overlap, bucket distributions | split comparison manifest |
| streaming/resource | 100k, 1M, and if feasible 10M records; fixed command and jobs | wall time, RSS, FD count, output size, disk headroom | resource report |
| threshold calibration | fixed corpus and teacher reference | coverage, agreement, bound rate, drop reasons by threshold | calibrate/audit report |
| training effect | fixed split, teacher/weight, trainer, budget, at least 3 seeds | validation loss/WDL, data size, label cost, variance | recipe comparison report |
| match transfer | fixed opening suite, opponent, games, seed and SPRT/interval rule | game count, result, confidence interval, comparison setup | match report |
| external interoperability | named tool/version and fixture | import/export result, loss report, legality, provenance, time | per-tool evidence row |

The fixture-backed path/order/worker comparison was completed on 2026-10-03. Its machine-readable
artifact is [`../measurements/reproducibility_matrix_2026-10-03.json`](../measurements/reproducibility_matrix_2026-10-03.json),
and `scripts/run_reproducibility_matrix.sh` regenerates it.

Run the local streaming/resource baseline against a prebuilt release binary with:

```bash
cargo build --release -p shogiesa-cli
python3 scripts/run_resource_baseline.py \
  --records 100000 \
  --out docs/measurements/resource_baseline_100k.json
```

The harness records command wall time, sampled peak RSS and open file descriptors, disk headroom,
binary and dataset hashes, output sizes, and explicit measurement limits. A dirty-tree run is a
provisional diagnostic; the roadmap row is complete only for an artifact built from an identified
clean commit. To bound peak disk use at 1M, the harness hashes and removes its regenerable
synthetic input after `pack` and before `unpack`; no measured command includes that deletion.

For a fixed-node delta between two immutable Sekirei releases, use
`scripts/run_sekirei_version_delta.py`. It builds both tags outside their working tree, fixes
`Threads=1` and `SpecTopN=0`, labels the identical sampled corpus, and records disagreement and
absolute cp-delta summaries. Input games are selected in sorted-path order up to `--max-games`,
with every selected path and SHA-256 retained; an empty extraction fails the run. This is a
search-output diagnostic, not an Elo or teacher-quality claim.

For the bounded training pilot, use `scripts/run_sekirei_learning_ablation.py`. It creates
equal-size baseline, filtered, uncertain-mined, and phase-balanced arms from one source-level
split, then runs each arm with three identical seeds against one frozen validation set. Compare
the recorded `valid_cp_mse` distributions only within that artifact. The trainer uses a completed
fixed-depth teacher search so a node-budget abort cannot inject an inexact label. One epoch on a
small corpus is pipeline evidence, not a playing-strength or generalization result.

Every run must retain the exact command line, repository commit, input/output hashes, engine and
weight identity, options, seed, hardware/OS, and any blocked dependency or network reason. Missing
measurements remain `unverified`; small fixtures do not substitute for scale or training evidence.
