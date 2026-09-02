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
  tests/fixtures/variation.kif
  tests/fixtures/broken.jsonl
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
