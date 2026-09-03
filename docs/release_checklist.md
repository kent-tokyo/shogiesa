# Release checklist

Use this checklist for publishing shogiesa `v0.9.2`. Mark each item with a command, artifact,
or explicit `blocked` reason; never convert an unavailable check into a success claim.

The lightweight repository contract check is `bash scripts/check_repository_contract.sh`; run it
before the full wrapper. The repeatable local check wrapper is `bash scripts/release_readiness.sh`.
It reports every check and returns non-zero if any check fails, including dependency/network failures in `cargo test` or
`cargo clippy`.
The latest local run for `v0.9.2` is recorded in
[`docs/release_validation_2026-09-04.md`](release_validation_2026-09-04.md); fixture tests remain
available after the dependency cache was completed. The earlier full-wrapper result remains in
[`docs/release_validation_2026-09-01.md`](release_validation_2026-09-01.md).

For this release, workspace compilation, metadata, tests, and all-target clippy passed locally.
The unchecked publication items below are not release evidence until their separate operations
complete.

## Code and tests

- [x] `cargo test` passes on the release checkout and all fixture counts are recorded.
- [x] `cargo fmt --check` passes.
- [x] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [x] malformed CSA/KIF/JSONL behavior is checked in normal and strict modes; contract and
      fixture inventories cover the release boundary.
- [x] pack magic/version/endian and JSONL round-trip fixtures are present and contract-checked.
- [x] USI timeout, protocol violation, restart, and child-process cleanup coverage is present in
      the repository test suite; full execution remains blocked by the uncached dev dependency.

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

- [x] `v0.9.2` release commit `94f2a412afe08cbd2eccc1d585d2c5ec3438afdf` is tagged and pushed
      to GitHub as `v0.9.2`.
- [ ] crates.io publication: BLOCKED by HTTP 403 authentication failure on
      `shogiesa-core v0.9.2`; no workspace crate was published. Retry after configuring a valid
      crates.io token.

## Release gate

Release only when required checks have evidence from the same clean checkout. A release may be
described as feature-complete for a scope, but “fastest”, “strongest”, “highest Elo”, or universal
training-quality improvement requires separate reproducible evidence and is not implied by this
checklist.
