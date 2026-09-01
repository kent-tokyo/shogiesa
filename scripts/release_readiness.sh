#!/usr/bin/env bash
set -u -o pipefail

# Run release checks without converting an unavailable check into a pass. The final exit status is
# non-zero when any check fails, while the full report remains visible for release triage.
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

failed=0
run_check() {
  local name="$1"
  shift
  printf '== %s ==\n' "$name"
  if "$@"; then
    printf 'PASS %s\n' "$name"
  else
    printf 'FAIL %s\n' "$name"
    failed=1
  fi
}

run_check "format" cargo fmt --check
run_check "diff-check" git diff --check
run_check "schema-json" jq empty schema/experiment_envelope.schema.json
run_check "tests" cargo test --workspace
run_check "clippy" cargo clippy --workspace --all-targets -- -D warnings

if (( failed == 0 )); then
  printf 'release readiness: PASS\n'
else
  printf 'release readiness: FAIL (see individual checks; unavailable checks are not passes)\n'
fi
exit "$failed"
