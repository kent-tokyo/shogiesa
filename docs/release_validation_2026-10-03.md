# v0.10.1 release validation — 2026-10-03

This log records validation for the `v0.10.1` release candidate. It does not claim throughput,
training effect, Elo, or native interoperability that was not measured by a named artifact.

| Check | Result | Evidence |
|---|---|---|
| repository contract | PASS | required documentation, schemas, fixtures, and measurement artifacts found |
| format and diff check | PASS | `cargo fmt --all -- --check` and `git diff --check` |
| workspace version | PASS | every workspace package resolves to `0.10.1`; nine are publishable |
| workspace tests | PASS | `cargo test --workspace`, including 537 unit and integration tests |
| strict Clippy | PASS | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| dependency audit | PASS | `cargo audit` found no known vulnerability in the locked dependency graph |
| package verification | PASS | `cargo package --workspace --locked` rebuilt every publishable crate in isolation |
| reproducibility matrix | PASS | path, reversed-input, and 1/2-worker axes retain the recorded identity/order hashes |
| GitHub CI | PASS | release commit passed Linux, macOS, Windows, lint, audit, and CodeQL before tagging |
| annotated tag | PENDING | verify `v0.10.1` after local and hosted validation |
| crates.io | PENDING | verify all nine non-yanked `0.10.1` packages after the publish workflow |
| GitHub Release | PENDING | verify the non-draft, non-prerelease release after publication |

The release adds opt-in recursive CSA/KIF/KI2 discovery with portable provenance, a fixture-backed
path/order/worker reproducibility matrix, the completed experiment-envelope consumer audit, concise
current documentation, and patch dependency updates. JSONL schema 11 and pack format 11 are
unchanged.
