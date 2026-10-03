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
| package verification | PASS | `cargo package --workspace --locked --allow-dirty` rebuilt every package in isolation |
| schema contract fixtures | PASS | schema 11 and legacy schema 1 parse, reject invalid versions, and round-trip as specified |
| GitHub CI | PENDING | Linux, macOS, Windows, audit, and CodeQL on the release commit |
| annotated tag | PENDING | `v0.11.0` must peel to the validated release commit |
| crates.io | PENDING | all nine publishable crates at non-yanked `0.11.0` |
| GitHub Release | PENDING | release created from the annotated tag |
| publication workflow | PENDING | tagged checkout packaged and published in dependency order |

The release adds the consumer-facing `shogiesa_core::schema` boundary, explicit schema-version
validation, canonical current/legacy contract fixtures, and CLI readers that apply the same
version gate. JSONL schema 11 and pack format 11 are unchanged.
