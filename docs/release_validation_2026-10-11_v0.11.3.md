# v0.11.3 release validation — 2026-10-11

This log records validation and publication evidence for the `v0.11.3` release. It does not claim
representative-corpus performance, training generalization, or engine-strength improvement.

| Check | Result | Evidence |
|---|---|---|
| repository contract | PASS | required documentation, fixtures, and measurement artifacts found |
| format and diff check | PASS | `cargo fmt --all -- --check` and `git diff --check` |
| workspace version | PASS | every workspace package resolves to `0.11.3`; nine are publishable |
| workspace tests | PASS | `cargo test --workspace --all-targets --all-features` |
| strict Clippy | PASS | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |
| benchmark compile | PASS | `cargo bench --workspace --no-run` |
| dependency audit | PASS | `cargo audit` scanned 157 locked dependencies against 1,296 advisories |
| shell checks | PASS | ShellCheck and dogfood fixture-backed integration tests |
| package verification | PASS | `cargo package --workspace --locked` |
| GitHub CI | PASS | [CI run 38083739715](https://github.com/kent-tokyo/shogiesa/actions/runs/38083739715) and [CodeQL run 38083739825](https://github.com/kent-tokyo/shogiesa/actions/runs/38083739825) succeeded for `9cfe29c43c8fb5dfbc6982416c0dc7d0518ff0f4` |
| annotated tag | PASS | remote `v0.11.3^{}` resolves to `9cfe29c43c8fb5dfbc6982416c0dc7d0518ff0f4` |
| crates.io | PASS | all nine publishable crates expose `0.11.3` with `yanked = false`; registry checksums were read directly from the version API |
| GitHub Release | PASS | [shogiesa v0.11.3](https://github.com/kent-tokyo/shogiesa/releases/tag/v0.11.3) is public, non-draft, and non-prerelease |
| publication workflow | PASS | [Publish release run 38084232520](https://github.com/kent-tokyo/shogiesa/actions/runs/38084232520) packaged and published the tagged source, then created the release |

Version 0.11.3 adds an independent `manifest_schema_version: 1` contract for run sidecars and a
canonical fixture. It retires the unused shared experiment-envelope draft in favor of
producer-owned manifests connected by explicit hashes. The lineprior dogfood runner now follows
the released EvalReport shape, preserves external binary identity, rejects mixed run bundles, and
writes its report atomically.

Position JSONL schema 11 and binary pack format 11 are unchanged.
