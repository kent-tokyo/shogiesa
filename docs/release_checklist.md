# Release checklist

Use this checklist for publishing shogiesa `v0.10.0`. Mark each item with a command, artifact,
or explicit `blocked` reason; never convert an unavailable check into a success claim.

The lightweight repository contract check is `bash scripts/check_repository_contract.sh`; run it
before the full wrapper. The repeatable local check wrapper is `bash scripts/release_readiness.sh`.
It reports every check and returns non-zero if any check fails, including dependency/network failures in `cargo test` or
`cargo clippy`.
The authoritative local run for `v0.10.0` is recorded in
[`docs/release_validation_2026-09-23.md`](release_validation_2026-09-23.md). This checklist keeps
local evidence separate from tag, GitHub Release, and registry publication results.

For this release, workspace compilation, metadata, tests, all-target clippy, and cross-platform
GitHub CI passed. Publication is not evidence until each external operation completes.

## Code and tests

- [x] `cargo test` passes on the release checkout and all fixture counts are recorded.
- [x] `cargo fmt --check` passes.
- [x] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [x] `cargo package --workspace --locked` packages and verifies every workspace crate in an
      isolated temporary registry.
- [x] malformed CSA/KIF/JSONL behavior is checked in normal and strict modes; contract and
      fixture inventories cover the release boundary.
- [x] pack magic/version/endian and JSONL round-trip fixtures are present and contract-checked.
- [x] USI timeout, protocol violation, restart, and child-process cleanup coverage is present in
      the repository test suite and passed in the release validation run.

## Provenance and recipes

- [x] label/filter/split/stratify/shuffle manifests contain the required input, output, seed,
      source-root, engine, weight, and option provenance, or explicit `unknown` values.
- [x] representative dataset recipe and fixed train/valid/test split are retained.
- [x] recipe artifacts have hashes and exact command lines.

## Documentation and claims

- [x] README examples match current command/output paths and release evidence links.
- [x] schema/pack compatibility table is current.
- [x] interoperability evidence and loss reports are present for every claimed external format.
- [x] competitor table separates feature fit from measured performance.
- [x] unmeasured RSS, speed, training effect, and Elo claims remain labeled unverified.
- [x] release notes state the exact validation environment and blocked checks.

## Publication status

- [ ] annotated `v0.10.0` tag pushed to GitHub and verified to name the release commit.
- [ ] GitHub Release created from `v0.10.0` with these release notes.
- [ ] crates.io publication verified for every publishable workspace crate. The previous 0.9.2
      attempt failed with authentication HTTP 403; record any new registry result rather than
      inferring success.

## Release gate

Release only when required checks have evidence from the same clean checkout. A release may be
described as feature-complete for a scope, but “fastest”, “strongest”, “highest Elo”, or universal
training-quality improvement requires separate reproducible evidence and is not implied by this
checklist.
