# v0.11.4 release validation — 2026-10-12

This log records candidate and publication evidence for the `v0.11.4` release. It does not claim
representative-corpus performance, training generalization, or engine-strength improvement.

| Check | Result | Evidence |
|---|---|---|
| repository contract | PASS | required documentation, fixtures, and measurement artifacts found |
| format and diff check | PASS | `cargo fmt --all -- --check` and `git diff --check` |
| workspace version | PASS | every workspace package resolves to `0.11.4`; nine are publishable |
| workspace tests | PASS | 584 tests passed with `cargo test --workspace --all-targets --all-features --locked` |
| strict Clippy | PASS | `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` |
| benchmark compile | PASS | `cargo bench --workspace --no-run --locked` |
| dependency audit | PASS | `cargo audit` scanned 157 locked dependencies against 1,296 advisories |
| shell checks | PASS | ShellCheck and fixture-backed dogfood integration tests |
| package verification | PASS | `cargo package --workspace --locked` from the clean release commit |
| GitHub CI | PENDING | release commit has not been pushed |
| annotated tag | PENDING | `v0.11.4` has not been created |
| crates.io | PENDING | `0.11.4` has not been published |
| GitHub Release | PENDING | `v0.11.4` has not been published |
| publication workflow | PENDING | publication has not started |

Version 0.11.4 adds the typed `RunManifestV1` API and label-artifact verification to
`shogiesa-core`. Label manifests now bind both input and output byte streams and account for
skipped non-empty input rows while preserving checked aggregate-count invariants.

Position JSONL schema 11, binary pack format 11, and run-manifest schema 1 are unchanged.
