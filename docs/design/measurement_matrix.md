# Measurement matrix for remaining roadmap gates

This is an execution plan, not benchmark evidence. A row becomes `[x]` in `ROADMAP.md` only
after its listed artifact contains the measured result and the environment is recorded.

For changes that do not require external engines, start with
`bash scripts/run_local_measurement_smoke.sh`. It validates the repository contract, formatting,
and four deterministic fixture-backed regression points (streaming report output, conflict
exclusions, split reproducibility, and pack manifest hashes). A PASS here is local regression evidence only; it does not
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

Every run must retain the exact command line, repository commit, input/output hashes, engine and
weight identity, options, seed, hardware/OS, and any blocked dependency or network reason. Missing
measurements remain `unverified`; small fixtures do not substitute for scale or training evidence.
