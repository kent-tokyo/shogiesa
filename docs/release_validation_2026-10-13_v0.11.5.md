# v0.11.5 release validation — 2026-10-13

This log records candidate and publication evidence for the `v0.11.5` release. It does not claim
representative-corpus performance, training generalization, or engine-strength improvement.

| Check | Result | Evidence |
|---|---|---|
| repository contract | PASS | required documentation, fixtures, and measurement artifacts found |
| format and diff check | PASS | `cargo fmt --all -- --check` and `git diff --check` |
| workspace version | PASS | every workspace package resolves to `0.11.5`; nine are publishable |
| workspace tests | PASS | 593 tests passed with `cargo test --workspace --all-targets --all-features --locked` |
| strict Clippy | PASS | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` |
| strict rustdoc | PASS | `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked` |
| benchmark compile | PASS | `cargo bench --workspace --no-run --locked` |
| dependency audit | PASS | `cargo audit` scanned 157 locked dependencies against 1,307 advisories |
| shell checks | PASS | ShellCheck, teacher-calibration self-test, and fixture-backed Python and dogfood integration tests |
| package verification | PASS | `cargo package --workspace --locked` from the clean release commit |
| GitHub CI | PASS | commit `9dfd50119874adcfce71d16809da0a6c4f81fb83`; [CI run 38320498859](https://github.com/kent-tokyo/shogiesa/actions/runs/38320498859) and [CodeQL run 38320499686](https://github.com/kent-tokyo/shogiesa/actions/runs/38320499686) succeeded |
| annotated tag | PASS | remote `v0.11.5^{}` resolves to `9dfd50119874adcfce71d16809da0a6c4f81fb83` |
| crates.io | PASS | all nine publishable crates have version `0.11.5` and `yanked=false` |
| GitHub Release | PASS | [shogiesa v0.11.5](https://github.com/kent-tokyo/shogiesa/releases/tag/v0.11.5) published, not draft or prerelease |
| publication workflow | PASS | [Publish release run 38321019766](https://github.com/kent-tokyo/shogiesa/actions/runs/38321019766) completed successfully |

Version 0.11.5 adds a bounded-memory, line-numbered position JSONL reader. Evaluation
normalization, swing, policy-margin, audit, mining, and bucket calculations now use `i64`
intermediates, and materialized JSONL diagnostics preserve physical line numbers across blank
lines.

Position JSONL schema 11, binary pack format 11, and run-manifest schema 1 are unchanged.
