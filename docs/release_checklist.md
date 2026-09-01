# Release checklist

Use this checklist before publishing a shogiesa release. Mark each item with a command, artifact,
or explicit `blocked` reason; never convert an unavailable check into a success claim.

The lightweight repository contract check is `bash scripts/check_repository_contract.sh`; run it
before the full wrapper. The repeatable local check wrapper is `bash scripts/release_readiness.sh`.
It reports every check and returns non-zero if any check fails, including dependency/network failures in `cargo test` or
`cargo clippy`.
The 2026-09-01 run is recorded in
[`docs/release_validation_2026-09-01.md`](release_validation_2026-09-01.md); test and clippy
remain blocked before compilation by crates.io DNS failure.

## Code and tests

- [ ] `cargo test` passes on the release checkout and all fixture counts are recorded.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] malformed CSA/KIF/JSONL behavior is checked in normal and strict modes.
- [ ] pack magic/version/endian and JSONL round-trip tests pass.
- [ ] USI timeout, protocol violation, restart, and child-process cleanup tests pass.

## Provenance and recipes

- [ ] label/filter/split/stratify/shuffle manifests contain the required input, output, seed,
      source-root, engine, weight, and option provenance, or explicit `unknown` values.
- [ ] representative dataset recipe and fixed train/valid/test split are retained.
- [ ] recipe artifacts have hashes and exact command lines.

## Documentation and claims

- [ ] README examples match `--help` and current output paths.
- [ ] schema/pack compatibility table is current.
- [ ] interoperability evidence and loss reports are present for every claimed external format.
- [ ] competitor table separates feature fit from measured performance.
- [ ] unmeasured RSS, speed, training effect, and Elo claims remain labeled unverified.
- [ ] release notes state the exact validation environment and any blocked checks.

## Release gate

Release only when required checks have evidence from the same clean checkout. A release may be
described as feature-complete for a scope, but “fastest”, “strongest”, “highest Elo”, or universal
training-quality improvement requires separate reproducible evidence and is not implied by this
checklist.
