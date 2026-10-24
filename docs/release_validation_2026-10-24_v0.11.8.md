# v0.11.8 release validation — 2026-10-24

This log records candidate and publication evidence for the `v0.11.8` release. It does not claim
representative-corpus performance, training generalization, or engine-strength improvement.

| Check | Result | Evidence |
|---|---|---|
| repository contract | PASS | required documentation, fixtures, refactoring boundaries, and measurement artifacts found |
| format and diff check | PASS | `cargo fmt --all -- --check` and `git diff --check` |
| workspace version | PASS | every workspace package resolves to `0.11.8`; nine are publishable |
| workspace tests | PASS | 621 tests passed with `cargo test --workspace` |
| strict Clippy | PASS | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| strict rustdoc | PASS | `RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps --locked`; Cargo also emitted its known same-name bin/lib output-path warning |
| benchmark compile | PASS | `cargo bench --workspace --no-run --locked` |
| dependency audit | PASS | `cargo audit` scanned 157 locked dependencies against 1,352 advisories and found no known vulnerabilities |
| script checks | PASS | ShellCheck, teacher-calibration self-test, and 12 fixture-backed Python tests |
| package verification | PASS | `cargo package --workspace --locked` completed from clean release commit `fc6e99b` |
| GitHub CI | PASS | [run 40099142446](https://github.com/kent-tokyo/shogiesa/actions/runs/40099142446): Linux, macOS, Windows, lint, audit, and benchmark compile |
| CodeQL | PASS | [run 40099142040](https://github.com/kent-tokyo/shogiesa/actions/runs/40099142040): Actions, Python, and Rust analysis |
| annotated tag | PASS | remote tag object `ecaa32b` peels to release commit `fc6e99b` |
| crates.io | PASS | all nine publishable crates are available at `0.11.8` with `yanked=false` |
| GitHub Release | PASS | [`v0.11.8`](https://github.com/kent-tokyo/shogiesa/releases/tag/v0.11.8) is public, non-draft, and non-prerelease |
| publication workflow | PASS | [run 40099607479](https://github.com/kent-tokyo/shogiesa/actions/runs/40099607479) published the crates in dependency order and created the release |

The first pushed candidate (`42d5e37`) was not tagged: hosted CI exposed an undeclared `rg`
dependency in repository checks and ANSI-decorated diagnostics under captured stderr. Release commit
`fc6e99b` added a portable `grep` fallback and disabled ANSI output for non-terminal stderr before
the successful hosted checks above.

Version 0.11.8 hardens failure boundaries without changing the position JSONL or binary pack
version. Pack encoding now rejects out-of-range fields before emitting a record, preserves the
supported historical JSONL schema range, rejects future schemas, and treats noncanonical binary
flags as corruption. Train/valid/test outputs and their manifest commit as one recoverable bundle,
recipe outputs with equal basenames receive distinct staging paths, and extreme numeric CLI inputs
fail with actionable diagnostics.

The release also includes the preceding source-layout refactor: typed position readers and atomic
I/O helpers are centralized, CLI commands and integration tests are split by responsibility, core
domain modules retain their public re-exports, and repository checks enforce those boundaries.
