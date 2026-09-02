# Release validation log — 2026-09-03

This log records the local, version-preserving validation run after the `v0.9.0` publication.
It is evidence for this environment only and is not a claim of full release readiness.

| check | result | evidence |
|---|---|---|
| repository contract check | PASS | all required docs, schema, fixtures, and checked BUILD items found |
| `cargo fmt --all -- --check` | PASS | completed successfully |
| `git diff --check` | PASS | completed successfully; macOS `xcrun_db` warnings were non-fatal |
| fixture-backed local measurement smoke | BLOCKED | `cargo test --offline` could not start because `float-cmp v0.10.0` was not cached |

The smoke script intentionally uses `--offline`: it must not turn a missing dependency or network
failure into a successful measurement. Re-run
`bash scripts/run_local_measurement_smoke.sh` after the dependency cache is complete, then append
or replace this environment-specific result with the exact command output.
