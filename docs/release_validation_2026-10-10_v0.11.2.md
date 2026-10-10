# v0.11.2 release validation — 2026-10-10

This log records validation and publication evidence for the `v0.11.2` release. It does not claim
representative-corpus performance, training generalization, or engine-strength improvement beyond
the named artifacts.

| Check | Result | Evidence |
|---|---|---|
| repository contract | PASS | required documentation, schemas, fixtures, and measurement artifacts found |
| format and diff check | PASS | `cargo fmt --all -- --check` and `git diff --check` |
| workspace version | PASS | every workspace package resolves to `0.11.2`; nine are publishable |
| workspace tests | PASS | `cargo test --workspace --all-targets --all-features` |
| strict Clippy | PASS | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| measurement helpers | PASS | Python unit tests, ShellCheck, local smoke, and reproducibility matrix |
| package verification | PASS | `cargo package --workspace --locked` |
| GitHub CI | PENDING | release commit has not yet completed remote CI |
| annotated tag | PENDING | `v0.11.2` will be created after candidate CI passes |
| crates.io | PENDING | nine workspace crates will be verified after publication |
| GitHub Release | PENDING | release will be created from the annotated tag |
| publication workflow | PENDING | tagged publish workflow has not yet completed |

The release rejects malformed or contradictory CLI search/filter parameters, protects inputs and
sidecars from path aliases, and makes non-resumable file outputs transactional. It adds a strict
teacher-calibration harness, a fail-closed Sekirei gate-result reporter, and checked-in 0.3.68
compatibility/calibration/smoke-gate evidence.

The teacher calibration uses 64 fixed positions and the A/B gate contains four games. They verify
the recorded contracts and execution path; they are not representative strength evidence. JSONL
schema 11 and pack format 11 are unchanged.
