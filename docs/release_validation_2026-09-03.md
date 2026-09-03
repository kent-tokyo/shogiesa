# Release validation log — 2026-09-03

This log records the local validation run for the `v0.9.1` release candidate.
It is evidence for this environment only and is not a claim of full release readiness.

| check | result | evidence |
|---|---|---|
| repository contract check | PASS | all required docs, schema, fixtures, and checked BUILD items found |
| `cargo fmt --all -- --check` | PASS | completed successfully |
| `git diff --check` | PASS | completed successfully; macOS `xcrun_db` warnings were non-fatal |
| workspace metadata/version | PASS | all publishable workspace crates resolve to `0.9.1` |
| corruption fixture inventory | PASS | malformed JSONL, bad magic, and truncated header markers are present and checked |
| fixture-backed local measurement smoke | BLOCKED | `cargo test --offline` could not start because `float-cmp v0.10.0` was not cached |
| `cargo test --offline --workspace` | BLOCKED | dependency cache lacks `float-cmp v0.10.0`; no success claim is made |
| `cargo clippy --offline --workspace --all-targets --all-features -- -D warnings` | BLOCKED | dependency cache lacks `float-cmp v0.10.0`; no success claim is made |
| GitHub `main` and `v0.9.1` tag push | PASS | `origin/main` advanced to `10eccc8`; tag points to the release commit |
| crates.io publish | BLOCKED | package/verify passed, then `shogiesa-core v0.9.1` upload returned HTTP 403 authentication failed; no crate was published |

Release commit and tag push completed. Registry publication requires a valid crates.io token and
must be retried separately; the failed upload did not publish any workspace crate.

The smoke script intentionally uses `--offline`: it must not turn a missing dependency or network
failure into a successful measurement. Re-run
`bash scripts/run_local_measurement_smoke.sh` after the dependency cache is complete, then append
or replace this environment-specific result with the exact command output.
