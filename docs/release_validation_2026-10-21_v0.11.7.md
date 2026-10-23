# v0.11.7 release validation — 2026-10-21

This log records candidate and publication evidence for the `v0.11.7` release. It does not claim
representative-corpus performance, training generalization, or engine-strength improvement.

| Check | Result | Evidence |
|---|---|---|
| repository contract | PASS | required documentation, fixtures, and measurement artifacts found |
| format and diff check | PASS | `cargo fmt --all -- --check` and `git diff --check` |
| workspace version | PASS | every workspace package resolves to `0.11.7`; nine are publishable |
| workspace tests | PASS | 608 tests passed with `cargo test --workspace --all-targets --all-features --locked` |
| strict Clippy | PASS | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` |
| strict rustdoc | PASS | `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked` |
| benchmark compile | PASS | `cargo bench --workspace --no-run --locked` |
| dependency audit | PASS | `cargo audit` scanned 157 locked dependencies against 1,348 advisories and found no known vulnerabilities |
| shell checks | PASS | ShellCheck, teacher-calibration self-test, and fixture-backed Python integration tests |
| package verification | PASS | `cargo package --workspace --locked` from the clean release commit |
| GitHub CI | PASS | commit `2c3f695f029074d406ace805ebfef72aecd355fc`; [CI run 39609039290](https://github.com/kent-tokyo/shogiesa/actions/runs/39609039290) and [CodeQL run 39609039000](https://github.com/kent-tokyo/shogiesa/actions/runs/39609039000) succeeded |
| annotated tag | PASS | remote `v0.11.7^{}` resolves to `2c3f695f029074d406ace805ebfef72aecd355fc` |
| crates.io | PASS | all nine publishable crates have version `0.11.7` and `yanked=false` |
| GitHub Release | PASS | [shogiesa v0.11.7](https://github.com/kent-tokyo/shogiesa/releases/tag/v0.11.7) published, not draft or prerelease |
| publication workflow | PASS | [Publish release run 39609742973](https://github.com/kent-tokyo/shogiesa/actions/runs/39609742973) completed successfully |

Version 0.11.7 adds the versioned `quality-report` evidence artifact for JSONL and pack inputs. It
records exact raw and decoded identities, stable rejection reasons, search-limit and score
histograms, coverage counts, bounded distinct estimates, optional manifest identities, and
deterministic per-shard summaries. The public v1 parser rejects unknown breaking report versions.

Position JSONL schema 11, binary pack format 11, and run-manifest schema 1 are unchanged. The new
quality-report schema starts at version 1.
