#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SEKIREI_DIR="${ROOT_DIR}/../sekirei"
SEKIREI_REF="v0.3.66"
OUT=""
RUN_TESTS=1

usage() {
  cat <<'EOF'
Usage: scripts/check_sekirei_compat.sh [options]

Verify the released Sekirei trainer against shogiesa's canonical schema fixture without
checking out or modifying the Sekirei working tree.

Options:
  --sekirei-dir PATH  Sekirei git repository (default: sibling ../sekirei)
  --ref REF           Sekirei tag or commit (default: v0.3.66)
  --out PATH          Write a JSON result artifact
  --skip-tests        Check source contract and fixture identity without running Cargo tests
  -h, --help          Show this help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --sekirei-dir) SEKIREI_DIR="$2"; shift 2 ;;
    --ref) SEKIREI_REF="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --skip-tests) RUN_TESTS=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) printf 'error: unknown argument: %s\n' "$1" >&2; usage >&2; exit 2 ;;
  esac
done

for command in git jq cargo tar rg; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'error: required command not found: %s\n' "$command" >&2
    exit 127
  }
done

[[ -d "$SEKIREI_DIR/.git" ]] || {
  printf 'error: not a Sekirei git checkout: %s\n' "$SEKIREI_DIR" >&2
  exit 2
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/shogiesa-sekirei-compat.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT
mkdir -p "$TMP_DIR/sekirei"

SEKIREI_COMMIT="$(git -C "$SEKIREI_DIR" rev-parse "${SEKIREI_REF}^{commit}")"
git -C "$SEKIREI_DIR" archive "$SEKIREI_REF" | tar -x -C "$TMP_DIR/sekirei"

PRODUCER_FIXTURE="$ROOT_DIR/crates/shogiesa-core/tests/fixtures/schema_contract_v11.jsonl"
CONSUMER_FIXTURE="$TMP_DIR/sekirei/crates/sekirei-train/tests/fixtures/shogiesa_schema_contract_v11.jsonl"
[[ -f "$PRODUCER_FIXTURE" ]] || {
  printf 'error: missing producer fixture: %s\n' "$PRODUCER_FIXTURE" >&2
  exit 1
}
[[ -f "$CONSUMER_FIXTURE" ]] || {
  printf 'error: Sekirei %s has no canonical shogiesa fixture\n' "$SEKIREI_REF" >&2
  exit 1
}

PRODUCER_SHA256="$(sha256_file "$PRODUCER_FIXTURE")"
CONSUMER_SHA256="$(sha256_file "$CONSUMER_FIXTURE")"
if ! cmp -s "$PRODUCER_FIXTURE" "$CONSUMER_FIXTURE"; then
  printf 'error: canonical fixture differs (producer=%s consumer=%s)\n' \
    "$PRODUCER_SHA256" "$CONSUMER_SHA256" >&2
  exit 1
fi

if ! rg -q '^shogiesa-core = "0\.11\.0"$' "$TMP_DIR/sekirei/crates/sekirei-train/Cargo.toml"; then
  printf 'error: Sekirei %s does not pin shogiesa-core 0.11.0\n' "$SEKIREI_REF" >&2
  exit 1
fi

TEST_STATUS="skipped"
if [[ "$RUN_TESTS" -eq 1 ]]; then
  TEST_STATUS="pass"
  CARGO_TARGET_DIR="$TMP_DIR/target" cargo test \
    --manifest-path "$TMP_DIR/sekirei/Cargo.toml" \
    --locked -p sekirei-train 'positions::tests::'
fi

SHOGIESA_COMMIT="$(git -C "$ROOT_DIR" rev-parse HEAD)"
SHOGIESA_DIRTY=false
if [[ -n "$(git -C "$ROOT_DIR" status --porcelain)" ]]; then
  SHOGIESA_DIRTY=true
fi

if [[ -n "$OUT" ]]; then
  mkdir -p "$(dirname "$OUT")"
  jq -n \
    --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg shogiesa_commit "$SHOGIESA_COMMIT" \
    --arg sekirei_ref "$SEKIREI_REF" \
    --arg sekirei_commit "$SEKIREI_COMMIT" \
    --arg fixture_sha256 "$PRODUCER_SHA256" \
    --arg test_status "$TEST_STATUS" \
    --argjson shogiesa_dirty "$SHOGIESA_DIRTY" \
    '{
      schema: "shogiesa.sekirei-compat.v1",
      generated_at: $generated_at,
      producer: {
        repository: "kent-tokyo/shogiesa",
        commit: $shogiesa_commit,
        working_tree_dirty: $shogiesa_dirty,
        schema_version: 11,
        fixture_sha256: $fixture_sha256
      },
      consumer: {
        repository: "kent-tokyo/sekirei",
        ref: $sekirei_ref,
        commit: $sekirei_commit,
        shogiesa_core_dependency: "0.11.0",
        vendored_fixture_sha256: $fixture_sha256
      },
      checks: {
        canonical_fixture_byte_identity: "pass",
        typed_positions_tests: $test_status
      }
    }' > "$OUT"
fi

printf 'Sekirei compatibility: PASS\n'
printf '  ref: %s (%s)\n' "$SEKIREI_REF" "$SEKIREI_COMMIT"
printf '  canonical fixture SHA-256: %s\n' "$PRODUCER_SHA256"
printf '  typed positions tests: %s\n' "$TEST_STATUS"
[[ -z "$OUT" ]] || printf '  artifact: %s\n' "$OUT"
