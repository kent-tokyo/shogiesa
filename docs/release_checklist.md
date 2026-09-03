# Release checklist

Use this checklist for publishing shogiesa `v0.9.1`. Mark each item with a command, artifact,
or explicit `blocked` reason; never convert an unavailable check into a success claim.

The lightweight repository contract check is `bash scripts/check_repository_contract.sh`; run it
before the full wrapper. The repeatable local check wrapper is `bash scripts/release_readiness.sh`.
It reports every check and returns non-zero if any check fails, including dependency/network failures in `cargo test` or
`cargo clippy`.
The latest local run for `v0.9.1` is recorded in
[`docs/release_validation_2026-09-03.md`](release_validation_2026-09-03.md); fixture tests remain
blocked when `float-cmp` is absent from the offline dependency cache. The earlier full-wrapper
result remains in [`docs/release_validation_2026-09-01.md`](release_validation_2026-09-01.md).

For this release, workspace compilation and metadata passed locally. Workspace tests and
all-target clippy were attempted and remain explicitly blocked by the uncached `float-cmp
v0.10.0` dependency; the unchecked items below must not be read as passed.

## Code and tests

- [ ] `cargo test` passes on the release checkout and all fixture counts are recorded.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [x] malformed CSA/KIF/JSONL behavior is checked in normal and strict modes; contract and
      fixture inventories cover the release boundary.
- [x] pack magic/version/endian and JSONL round-trip fixtures are present and contract-checked.
- [x] USI timeout, protocol violation, restart, and child-process cleanup coverage is present in
      the repository test suite; full execution remains blocked by the uncached dev dependency.

## Provenance and recipes

- [ ] label/filter/split/stratify/shuffle manifests contain the required input, output, seed,
      source-root, engine, weight, and option provenance, or explicit `unknown` values.
- [ ] representative dataset recipe and fixed train/valid/test split are retained.
- [ ] recipe artifacts have hashes and exact command lines.

## Documentation and claims

- [x] README examples match current command/output paths and release evidence links.
- [x] schema/pack compatibility table is current.
- [x] interoperability evidence and loss reports are present for every claimed external format.
- [x] competitor table separates feature fit from measured performance.
- [x] unmeasured RSS, speed, training effect, and Elo claims remain labeled unverified.
- [x] release notes state the exact validation environment and blocked checks.

## Release gate

Release only when required checks have evidence from the same clean checkout. A release may be
described as feature-complete for a scope, but “fastest”, “strongest”, “highest Elo”, or universal
training-quality improvement requires separate reproducible evidence and is not implied by this
checklist.
