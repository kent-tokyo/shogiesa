# Release validation log — 2026-09-04

This log records the local validation run for the `v0.9.2` release candidate. It is evidence for
this environment and does not claim external performance, training, Elo, or native interoperability.

| check | result | evidence |
|---|---|---|
| repository contract check | PASS | required docs, schema, fixtures, recipe markers, and checked BUILD items found |
| `cargo fmt --all -- --check` | PASS | completed successfully |
| `git diff --check` | PASS | completed successfully |
| workspace metadata/version | PASS | all publishable workspace crates resolve to `0.9.2` |
| `cargo test --workspace` | PASS | all workspace unit, integration, and doc tests passed; 39 CLI unit tests and 254 CLI integration tests included |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | PASS | completed successfully |
| fixture-backed recipe run/verify | PASS | run, verify, reuse, partial checkpoint resume, output tamper, and manifest topology checks passed in the targeted integration tests |
| external measurements | UNMEASURED | 1M/10M throughput, multi-OS behavior, training effect, Elo, and native interoperability remain outside this local validation |
| GitHub tag/push | PENDING | to be performed after the release commit |
| crates.io publish | PENDING | to be performed after package verification |

The release readiness wrapper initially exposed two stale fixture expectations; they were updated to
the current `conflict-report` and omitted-empty-`candidates` contracts, and the affected tests then
passed individually and in the complete workspace run.
