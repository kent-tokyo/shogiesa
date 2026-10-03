# v0.11.0 release validation — 2026-10-03

This log records validation and publication evidence for the `v0.11.0` release. It does not claim
throughput, training effect, Elo, or native interoperability that was not measured by a named
artifact.

| Check | Result | Evidence |
|---|---|---|
| repository contract | PASS | required documentation, schemas, fixtures, and measurement artifacts found |
| format and diff check | PASS | `cargo fmt --all -- --check`, `git diff --check`, and `actionlint .github/workflows/*.yml` |
| workspace version | PASS | every workspace package resolves to `0.11.0`; nine are publishable |
| workspace tests | PASS | `cargo test --workspace`, including 542 unit and integration tests |
| strict Clippy | PASS | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| dependency audit | PASS | `cargo audit` found no known vulnerability in 157 locked dependencies |
| package verification | PASS | `cargo package --workspace --locked` rebuilt the tagged checkout in isolation |
| schema contract fixtures | PASS | schema 11 and legacy schema 1 parse, reject invalid versions, and round-trip as specified |
| GitHub CI | PASS | [CI run 37087640962](https://github.com/kent-tokyo/shogiesa/actions/runs/37087640962) and [CodeQL run 37087640883](https://github.com/kent-tokyo/shogiesa/actions/runs/37087640883) passed before tagging |
| annotated tag | PASS | `v0.11.0` peels to release commit `ca3662b276435dd1ce2b30859e3557e971491110` |
| crates.io | PASS | all nine publishable workspace crates have a non-yanked `0.11.0` release |
| GitHub Release | PASS | [shogiesa v0.11.0](https://github.com/kent-tokyo/shogiesa/releases/tag/v0.11.0), published 2026-10-03 JST |
| publication workflow | PASS | [run 37087978126](https://github.com/kent-tokyo/shogiesa/actions/runs/37087978126) packaged and published the tagged checkout |

The release adds the consumer-facing `shogiesa_core::schema` boundary, explicit schema-version
validation, canonical current/legacy contract fixtures, and CLI readers that apply the same
version gate. JSONL schema 11 and pack format 11 are unchanged.

The first candidate CI run exposed a Linux scheduling race in duplicate-`bestmove` detection.
Release commit `ca3662b` adds an `isready`/`readyok` command-order barrier; the regression passed
25 repeated local runs and the final three-OS CI run.
