#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

failed=0

check_max_lines() {
  local path="$1"
  local maximum="$2"
  local lines
  lines="$(wc -l < "$path" | tr -d ' ')"
  if (( lines <= maximum )); then
    printf 'PASS lines %s %s/%s\n' "$path" "$lines" "$maximum"
  else
    printf 'FAIL lines %s %s/%s\n' "$path" "$lines" "$maximum"
    failed=1
  fi
}

required_files=(
  crates/shogiesa-cli/src/commands/cache.rs
  crates/shogiesa-cli/src/commands/diagnostics.rs
  crates/shogiesa-cli/src/commands/ingest.rs
  crates/shogiesa-cli/src/commands/label.rs
  crates/shogiesa-cli/src/commands/quality.rs
  crates/shogiesa-cli/src/commands/selection.rs
  crates/shogiesa-cli/src/commands/transforms.rs
  crates/shogiesa-cli/src/commands/transport.rs
  crates/shogiesa-cli/src/io_support/atomic_output.rs
  crates/shogiesa-cli/src/io_support/hashing.rs
  crates/shogiesa-cli/src/io_support/path_roles.rs
  crates/shogiesa-cli/src/io_support/position_jsonl.rs
  crates/shogiesa-cli/tests/cli/adapters.rs
  crates/shogiesa-cli/tests/cli/diagnostics.rs
  crates/shogiesa-cli/tests/cli/distribution.rs
  crates/shogiesa-cli/tests/cli/extract.rs
  crates/shogiesa-cli/tests/cli/ingest_extra.rs
  crates/shogiesa-cli/tests/cli/label.rs
  crates/shogiesa-cli/tests/cli/regressions.rs
  crates/shogiesa-cli/tests/cli/workflows.rs
  crates/shogiesa-core/src/extraction.rs
  crates/shogiesa-core/src/model.rs
  crates/shogiesa-core/src/quality.rs
  crates/shogiesa-core/src/score.rs
)

for path in "${required_files[@]}"; do
  if [[ -f "$path" ]]; then
    printf 'PASS module %s\n' "$path"
  else
    printf 'FAIL missing module %s\n' "$path"
    failed=1
  fi
done

check_max_lines crates/shogiesa-cli/src/main.rs 1900
check_max_lines crates/shogiesa-cli/tests/cli_test.rs 150
check_max_lines crates/shogiesa-core/src/lib.rs 900

for path in crates/shogiesa-cli/src/commands/*.rs; do
  check_max_lines "$path" 1700
done
for path in crates/shogiesa-cli/tests/cli/*.rs; do
  check_max_lines "$path" 5600
done
for path in \
  crates/shogiesa-core/src/extraction.rs \
  crates/shogiesa-core/src/model.rs \
  crates/shogiesa-core/src/quality.rs \
  crates/shogiesa-core/src/score.rs; do
  check_max_lines "$path" 500
done

if rg -n 'include!\s*\(' crates/shogiesa-cli/src/main.rs crates/shogiesa-cli/tests/cli_test.rs; then
  printf 'FAIL include!-based module splitting is not allowed\n'
  failed=1
else
  printf 'PASS no include!-based module splitting\n'
fi

if rg -n 'serde_json::from_(str|slice|value)::<PositionRecord>' crates/shogiesa-cli/src; then
  printf 'FAIL direct PositionRecord JSON deserialization bypasses the typed reader\n'
  failed=1
else
  printf 'PASS typed PositionRecord JSONL boundary\n'
fi

exit "$failed"
