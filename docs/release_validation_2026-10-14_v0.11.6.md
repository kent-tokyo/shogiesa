# v0.11.6 release validation — 2026-10-14

This log records candidate and publication evidence for the `v0.11.6` release. It does not claim
representative-corpus performance, training generalization, or engine-strength improvement.

| Check | Result | Evidence |
|---|---|---|
| repository contract | PASS | required documentation, fixtures, and measurement artifacts found |
| format and diff check | PASS | `cargo fmt --all -- --check` and `git diff --check` |
| workspace version | PASS | every workspace package resolves to `0.11.6`; nine are publishable |
| workspace tests | PASS | 598 tests passed with `cargo test --workspace --all-targets --all-features --locked` |
| strict Clippy | PASS | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` |
| strict rustdoc | PASS | `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked` |
| benchmark compile | PASS | `cargo bench --workspace --no-run --locked` |
| dependency audit | PASS | `cargo audit` scanned 157 locked dependencies against 1,310 advisories and found no known vulnerabilities |
| shell checks | PASS | ShellCheck, teacher-calibration self-test, and fixture-backed Python and dogfood integration tests |
| package verification | PASS | `cargo package --workspace --locked` from the clean release commit |
| GitHub CI | PASS | commit `acb5c22a1b69fb3d7ae92fe0b8d45b260f4cf4cc`; [CI run 38463286549](https://github.com/kent-tokyo/shogiesa/actions/runs/38463286549) and [CodeQL run 38463286511](https://github.com/kent-tokyo/shogiesa/actions/runs/38463286511) succeeded |
| annotated tag | PASS | remote `v0.11.6^{}` resolves to `acb5c22a1b69fb3d7ae92fe0b8d45b260f4cf4cc` |
| crates.io | PASS | all nine publishable crates have version `0.11.6` and `yanked=false` |
| GitHub Release | PASS | [shogiesa v0.11.6](https://github.com/kent-tokyo/shogiesa/releases/tag/v0.11.6) published, not draft or prerelease |
| publication workflow | PASS | [Publish release run 38463933324](https://github.com/kent-tokyo/shogiesa/actions/runs/38463933324) completed successfully |

Version 0.11.6 makes CLI JSONL ingestion consistently use the public typed, schema-gated reader.
Materializing commands no longer retain the complete input text, reporting and streaming
transforms reject unsupported future schemas consistently, and `filter` plus both `split` modes
now enforce the same schema boundary.

Position JSONL schema 11, binary pack format 11, and run-manifest schema 1 are unchanged.
