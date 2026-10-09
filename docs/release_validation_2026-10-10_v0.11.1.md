# v0.11.1 release validation — 2026-10-10

This log records validation and publication evidence for the `v0.11.1` release. It does not claim
representative-corpus performance, training generalization, Elo, or native interoperability that
was not measured by a named artifact.

| Check | Result | Evidence |
|---|---|---|
| repository contract | PASS | required documentation, schemas, fixtures, and measurement artifacts found |
| format and diff check | PASS | `cargo fmt --all -- --check`, `git diff --check`, and `actionlint .github/workflows/*.yml` |
| workspace version | PASS | every workspace package resolves to `0.11.1`; nine are publishable |
| workspace tests | PASS | `cargo test --workspace`, including 544 unit and integration tests |
| USI race regression | PASS | non-strict duplicate-bestmove regression passed 40 repeated local runs and final three-OS CI |
| strict Clippy | PASS | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| dependency audit | PASS | `cargo audit` found no known vulnerability in 157 locked dependencies |
| package verification | PASS | `cargo package --workspace --locked` rebuilt the release commit in isolation |
| GitHub CI | PASS | [CI run 37990921767](https://github.com/kent-tokyo/shogiesa/actions/runs/37990921767) and [CodeQL run 37990921950](https://github.com/kent-tokyo/shogiesa/actions/runs/37990921950) passed before tagging |
| annotated tag | PASS | `v0.11.1` peels to release commit `9459fd3d6972a7328becdd0d0d48078abf4c9268` |
| crates.io | PASS | all nine publishable workspace crates have a non-yanked `0.11.1` release |
| GitHub Release | PASS | [shogiesa v0.11.1](https://github.com/kent-tokyo/shogiesa/releases/tag/v0.11.1), published 2026-10-10 JST |
| publication workflow | PASS | [run 37991281050](https://github.com/kent-tokyo/shogiesa/actions/runs/37991281050) packaged and published the tagged checkout |

The release strengthens typed `validate --strict` diagnostics and adds reproducible resource,
Sekirei version-delta, mining, and three-seed training-ablation harnesses with checked-in evidence.
The resource artifacts cover synthetic 100k and 1M streaming runs; the training artifact remains a
small 48-position, one-epoch pilot and is not engine-strength evidence.

The first `0.11.1` candidate CI run exposed a Linux scheduling race in non-strict reused USI
sessions: a duplicate `bestmove` could cross the asynchronous reader boundary after the next `go`.
Release commit `9459fd3` applies the existing `isready`/`readyok` command-order barrier in both
strict and compatibility modes. Strict mode reports the violation; compatibility mode discards it.

JSONL schema 11 and pack format 11 are unchanged.
