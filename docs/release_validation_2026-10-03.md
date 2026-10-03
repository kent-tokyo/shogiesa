# v0.10.1 release validation — 2026-10-03

This log records validation and publication evidence for the `v0.10.1` release. It does not claim
throughput, training effect, Elo, or native interoperability that was not measured by a named
artifact.

| Check | Result | Evidence |
|---|---|---|
| repository contract | PASS | required documentation, schemas, fixtures, and measurement artifacts found |
| format and diff check | PASS | `cargo fmt --all -- --check` and `git diff --check` |
| workspace version | PASS | every workspace package resolves to `0.10.1`; nine are publishable |
| workspace tests | PASS | `cargo test --workspace`, including 537 unit and integration tests |
| strict Clippy | PASS | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| dependency audit | PASS | `cargo audit` found no known vulnerability in the locked dependency graph |
| package verification | PASS | `cargo package --workspace --locked` rebuilt every publishable crate in isolation |
| reproducibility matrix | PASS | path, reversed-input, and 1/2-worker axes retain the recorded identity/order hashes |
| GitHub CI | PASS | [CI run 37082830736](https://github.com/kent-tokyo/shogiesa/actions/runs/37082830736) and [CodeQL run 37082830135](https://github.com/kent-tokyo/shogiesa/actions/runs/37082830135) passed before tagging |
| annotated tag | PASS | `v0.10.1` peels to release commit `b0494e401c3e8e19ddbf1903b0464b4bc5ae5451` |
| crates.io | PASS | all nine publishable workspace crates have a non-yanked `0.10.1` release |
| GitHub Release | PASS | [shogiesa v0.10.1](https://github.com/kent-tokyo/shogiesa/releases/tag/v0.10.1), published 2026-10-03 JST |
| publication workflow | PASS | [run 37083375665](https://github.com/kent-tokyo/shogiesa/actions/runs/37083375665) packaged and published the tagged checkout |

The release adds opt-in recursive CSA/KIF/KI2 discovery with portable provenance, a fixture-backed
path/order/worker reproducibility matrix, the completed experiment-envelope consumer audit, concise
current documentation, and patch dependency updates. JSONL schema 11 and pack format 11 are
unchanged.
