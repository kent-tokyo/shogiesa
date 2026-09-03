#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

required_files=(
  README.md
  ROADMAP.md
  docs/design/schema_compatibility.md
  docs/THEORY.md
  schema/experiment_envelope.schema.json
  docs/design/dataset_recipe_template.md
  docs/design/training_effect_measurement.md
  docs/design/measurement_matrix.md
  docs/design/experiment_envelope.md
  docs/interop_evidence.md
  docs/competitor_evidence.md
  docs/api_boundary.md
  docs/release_checklist.md
  docs/release_validation_2026-09-03.md
  scripts/release_readiness.sh
  scripts/run_local_measurement_smoke.sh
  tests/fixtures/sample.csa
  tests/fixtures/sample.kif
  tests/fixtures/malformed.csa
  tests/fixtures/malformed.kif
  tests/fixtures/no-terminal.kif
  tests/fixtures/variation.kif
  tests/fixtures/broken.jsonl
  tests/fixtures/pack_input.jsonl
  tests/fixtures/malformed_mixed.jsonl
  tests/fixtures/conflict_report_input.jsonl
  tests/fixtures/conflict_report.golden
  tests/fixtures/block_report_input.jsonl
  tests/fixtures/block_report_size1.golden
  tests/fixtures/block_report_size2.golden
  tests/fixtures/distribution.golden
  tests/fixtures/pack_bad_magic.hex
  tests/fixtures/pack_truncated_header.hex
  tests/fixtures/pack_unsupported_version.hex
  tests/fixtures/pack_trailing_bytes.hex
  tests/fixtures/pack_wrong_endian_version.hex
  tests/fixtures/pack_truncated_record.hex
)

missing=0
for path in "${required_files[@]}"; do
  if [[ -f "$path" ]]; then
    printf 'PASS file %s\n' "$path"
  else
    printf 'FAIL missing %s\n' "$path"
    missing=1
  fi
done

if rg -q 'shogiesa extract|shogiesa label|shogiesa filter' README.md; then
  printf 'PASS quick-start commands\n'
else
  printf 'FAIL quick-start commands missing\n'
  missing=1
fi

check_marker() {
  local path="$1"
  local pattern="$2"
  local label="$3"
  if rg -q -- "$pattern" "$path"; then
    printf 'PASS fixture marker %s\n' "$label"
  else
    printf 'FAIL fixture marker %s\n' "$label"
    missing=1
  fi
}

check_marker tests/fixtures/malformed.csa '^\+BAD$' 'malformed CSA token'
check_marker tests/fixtures/malformed.kif 'これは指し手ではない' 'malformed KIF move'
check_marker tests/fixtures/no-terminal.kif '^   1 ７六歩' 'KIF without terminal result'
check_marker tests/fixtures/variation.kif '^変化：2手' 'KIF variation marker'
check_marker tests/fixtures/broken.jsonl '^not json$' 'broken JSONL line'
check_marker tests/fixtures/pack_input.jsonl '"schema_version":11' 'pack input schema'
check_marker tests/fixtures/malformed_mixed.jsonl '^not json$' 'mixed JSONL malformed suffix'
check_marker tests/fixtures/conflict_report_input.jsonl '"variation_id":"var1"' 'conflict report variation input'
check_marker tests/fixtures/conflict_report.golden '^conflicts         : 1  \(33\.3%\)$' 'conflict report golden summary'
check_marker tests/fixtures/block_report_input.jsonl '"root_id":"game-b.kif"' 'block report variation root input'
check_marker tests/fixtures/block_report_size2.golden '^blocks            : 2$' 'block report size 2 golden'
check_marker tests/fixtures/block_report_size1.golden '^blocks            : 4$' 'block report size 1 golden'
check_marker tests/fixtures/distribution.golden '^positions   : 4$' 'distribution golden summary'
check_marker tests/fixtures/pack_bad_magic.hex '^00000000000000000b00$' 'pack bad magic bytes'
check_marker tests/fixtures/pack_truncated_header.hex '^53484f4749455341$' 'pack truncated header bytes'
check_marker tests/fixtures/pack_unsupported_version.hex '^53484f4749455341ffff$' 'pack unsupported version bytes'
check_marker tests/fixtures/pack_trailing_bytes.hex '^53484f47494553410b00ff$' 'pack trailing bytes'
check_marker tests/fixtures/pack_wrong_endian_version.hex '^53484f4749455341000b$' 'pack wrong-endian version bytes'
check_marker tests/fixtures/pack_truncated_record.hex '^53484f47494553410b000b00$' 'pack truncated record bytes'

if rg -q 'SCHEMA_VERSION = 11|FORMAT_VERSION: u16 = 11' crates/shogiesa-core/src/lib.rs crates/shogiesa-pack/src/lib.rs; then
  printf 'PASS schema/pack version markers\n'
else
  printf 'FAIL schema/pack version markers missing\n'
  missing=1
fi

if rg -q '未測定|unmeasured|unverified|not.*measured' ROADMAP.md README.md docs/release_checklist.md; then
  printf 'PASS unmeasured-claims boundary\n'
else
  printf 'FAIL unmeasured-claims boundary missing\n'
  missing=1
fi

if rg -q '^[-*] `\[BUILD\]`' ROADMAP.md; then
  printf 'FAIL unchecked BUILD items remain\n'
  missing=1
else
  printf 'PASS roadmap BUILD items checked\n'
fi

exit "$missing"
