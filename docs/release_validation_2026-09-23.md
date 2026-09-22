# Release validation log — 2026-09-23

This log records local and CI validation for the `v0.10.0` release candidate. It does not claim
external performance, training effect, Elo, or native interoperability.

| Check | Result | Evidence |
|---|---|---|
| repository contract | PASS | required docs, schema, fixtures, recipe markers, and checked roadmap items found |
| format and diff check | PASS | `cargo fmt --all -- --check` and `git diff --check` completed successfully |
| workspace metadata/version | PASS | every publishable workspace crate resolves to `0.10.0` |
| workspace tests | PASS | `cargo test --workspace` passed, including 39 CLI unit and 254 CLI integration tests |
| Clippy | PASS | `cargo clippy --workspace --all-targets --all-features -- -D warnings` completed successfully |
| package verification | PASS | `cargo package --workspace --locked` packaged and rebuilt every workspace crate in isolation |
| GitHub CI | PASS | Ubuntu, macOS, Windows, lint, audit, and CodeQL passed on merged PR #19 |
| external measurements | UNMEASURED | 1M/10M throughput, training effect, Elo, and native interoperability remain outside this release validation |
| tag, GitHub Release, crates.io | PENDING | verified separately as release operations; neither tag nor local tests imply registry publication |

The candidate adds nested KIF variation parentage, input-boundary hardening, portable fixture
checks, and dependency updates. It keeps the public-data schema and versioned pack format stable.
