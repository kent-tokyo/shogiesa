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
| package verification | PENDING | requires a clean release commit |
| GitHub CI | PENDING | verify the pushed release commit across hosted checks |
| annotated tag | PENDING | create only after the release commit passes local and hosted checks |
| crates.io | PENDING | verify all nine publishable crates at `0.11.8` with `yanked=false` |
| GitHub Release | PENDING | verify public, non-draft, and non-prerelease status |
| publication workflow | PENDING | dispatch only for the pushed annotated tag |

Version 0.11.8 hardens failure boundaries without changing the position JSONL or binary pack
version. Pack encoding now rejects out-of-range fields before emitting a record, preserves the
supported historical JSONL schema range, rejects future schemas, and treats noncanonical binary
flags as corruption. Train/valid/test outputs and their manifest commit as one recoverable bundle,
recipe outputs with equal basenames receive distinct staging paths, and extreme numeric CLI inputs
fail with actionable diagnostics.

The release also includes the preceding source-layout refactor: typed position readers and atomic
I/O helpers are centralized, CLI commands and integration tests are split by responsibility, core
domain modules retain their public re-exports, and repository checks enforce those boundaries.
