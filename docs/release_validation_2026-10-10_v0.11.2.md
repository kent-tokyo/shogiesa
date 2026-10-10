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
| GitHub CI | PASS | [CI run 38049795908](https://github.com/kent-tokyo/shogiesa/actions/runs/38049795908) and [CodeQL run 38049795587](https://github.com/kent-tokyo/shogiesa/actions/runs/38049795587) |
| annotated tag | PASS | remote `v0.11.2^{}` resolves to `482f7cd711772938fba3338eaee7dc9853a0c255` |
| crates.io | PASS | all nine workspace crates expose `0.11.2` with `yanked = false` |
| GitHub Release | PASS | [shogiesa v0.11.2](https://github.com/kent-tokyo/shogiesa/releases/tag/v0.11.2), published 2026-10-10 |
| publication workflow | PASS | [run 38050090591](https://github.com/kent-tokyo/shogiesa/actions/runs/38050090591) published the crates in dependency order and created the release |

The release rejects malformed or contradictory CLI search/filter parameters, protects inputs and
sidecars from path aliases, and makes non-resumable file outputs transactional. It adds a strict
teacher-calibration harness, a fail-closed Sekirei gate-result reporter, and checked-in 0.3.68
compatibility/calibration/smoke-gate evidence.

The teacher calibration uses 64 fixed positions and the A/B gate contains four games. They verify
the recorded contracts and execution path; they are not representative strength evidence. JSONL
schema 11 and pack format 11 are unchanged.

Two Windows-only candidate failures were fixed before the release tag: staged split files are now
opened with write permission before `sync_all`, and CLI test outputs close their temporary file
handles before atomic replacement. The final Windows, macOS, and Ubuntu jobs all passed.
