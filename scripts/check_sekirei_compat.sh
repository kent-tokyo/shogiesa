#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SEKIREI_DIR="${ROOT_DIR}/../sekirei"
SEKIREI_REF="v0.3.68"
SHOGIESA_REF=""
EXPECTED_SHOGIESA_CORE="0.11.1"
OUT=""
OUT_TMP=""
RUN_TESTS=1

require_option_value() {
  local option="$1"
  local count="$2"
  local value="${3:-}"
  if [[ "$count" -lt 2 || -z "$value" || "$value" == --* ]]; then
    printf 'error: %s requires a value\n' "$option" >&2
    exit 2
  fi
}

usage() {
  cat <<'EOF'
Usage: scripts/check_sekirei_compat.sh [options]

Verify the released Sekirei trainer against shogiesa's canonical schema fixture without
checking out or modifying the Sekirei working tree.

Options:
  --sekirei-dir PATH  Sekirei git repository (default: sibling ../sekirei)
  --ref REF           Sekirei tag or commit (default: v0.3.68)
  --shogiesa-ref REF  Validate an immutable shogiesa tag or commit instead of the working tree
  --out PATH          Write a JSON result artifact
  --skip-tests        Check source contract and fixture identity without running Cargo tests
  -h, --help          Show this help
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --sekirei-dir)
      require_option_value "$1" "$#" "${2:-}"
      SEKIREI_DIR="$2"
      shift 2
      ;;
    --ref)
      require_option_value "$1" "$#" "${2:-}"
      SEKIREI_REF="$2"
      shift 2
      ;;
    --shogiesa-ref)
      require_option_value "$1" "$#" "${2:-}"
      SHOGIESA_REF="$2"
      shift 2
      ;;
    --out)
      require_option_value "$1" "$#" "${2:-}"
      OUT="$2"
      shift 2
      ;;
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

git -C "$SEKIREI_DIR" rev-parse --git-dir >/dev/null 2>&1 || {
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
cleanup() {
  rm -rf "$TMP_DIR"
  [[ -z "$OUT_TMP" ]] || rm -f "$OUT_TMP"
}
trap cleanup EXIT
mkdir -p "$TMP_DIR/sekirei"

SEKIREI_COMMIT="$(git -C "$SEKIREI_DIR" rev-parse "${SEKIREI_REF}^{commit}")"
git -C "$SEKIREI_DIR" archive "$SEKIREI_REF" | tar -x -C "$TMP_DIR/sekirei"

SHOGIESA_COMMIT="$(git -C "$ROOT_DIR" rev-parse HEAD)"
SHOGIESA_DIRTY=false
PRODUCER_ROOT="$ROOT_DIR"
PRODUCER_SOURCE="working-tree"
if [[ -n "$SHOGIESA_REF" ]]; then
  SHOGIESA_COMMIT="$(git -C "$ROOT_DIR" rev-parse "${SHOGIESA_REF}^{commit}")"
  PRODUCER_ROOT="$TMP_DIR/shogiesa"
  PRODUCER_SOURCE="$SHOGIESA_REF"
  mkdir -p "$PRODUCER_ROOT"
  git -C "$ROOT_DIR" archive "$SHOGIESA_REF" | tar -x -C "$PRODUCER_ROOT"
elif [[ -n "$(git -C "$ROOT_DIR" status --porcelain)" ]]; then
  SHOGIESA_DIRTY=true
fi

LOCAL_PRODUCER_FIXTURE="$ROOT_DIR/crates/shogiesa-core/tests/fixtures/schema_contract_v11.jsonl"
PRODUCER_FIXTURE="$PRODUCER_ROOT/crates/shogiesa-core/tests/fixtures/schema_contract_v11.jsonl"
CONSUMER_FIXTURE="$TMP_DIR/sekirei/crates/sekirei-train/tests/fixtures/shogiesa_schema_contract_v11.jsonl"
[[ -f "$PRODUCER_FIXTURE" ]] || {
  printf 'error: missing producer fixture: %s\n' "$PRODUCER_FIXTURE" >&2
  exit 1
}
[[ -f "$CONSUMER_FIXTURE" ]] || {
  printf 'error: Sekirei %s has no canonical shogiesa fixture\n' "$SEKIREI_REF" >&2
  exit 1
}

resolve_output_symlink() {
  local path="$1"
  local target
  local links=0
  while [[ -L "$path" ]]; do
    target="$(readlink "$path")"
    if [[ "$target" = /* ]]; then
      path="$target"
    else
      path="$(dirname "$path")/$target"
    fi
    links=$((links + 1))
    if [[ "$links" -gt 40 ]]; then
      printf 'error: too many symlinks in artifact output: %s\n' "$1" >&2
      exit 1
    fi
  done
  printf '%s\n' "$path"
}

OUT_DEST=""
if [[ -n "$OUT" ]]; then
  OUT_DEST="$(resolve_output_symlink "$OUT")"
  if [[ -e "$OUT_DEST" && "$OUT_DEST" -ef "$PRODUCER_FIXTURE" ]]; then
    printf 'error: --out must not overwrite the canonical producer fixture\n' >&2
    exit 2
  fi
  if [[ -e "$OUT_DEST" && "$OUT_DEST" -ef "$LOCAL_PRODUCER_FIXTURE" ]]; then
    printf 'error: --out must not overwrite the canonical producer fixture\n' >&2
    exit 2
  fi
  if [[ -d "$OUT_DEST" ]]; then
    printf 'error: --out is a directory: %s\n' "$OUT" >&2
    exit 2
  fi
fi

PRODUCER_SHA256="$(sha256_file "$PRODUCER_FIXTURE")"
CONSUMER_SHA256="$(sha256_file "$CONSUMER_FIXTURE")"
if ! cmp -s "$PRODUCER_FIXTURE" "$CONSUMER_FIXTURE"; then
  printf 'error: canonical fixture differs (producer=%s consumer=%s)\n' \
    "$PRODUCER_SHA256" "$CONSUMER_SHA256" >&2
  exit 1
fi

if ! rg -q "^shogiesa-core = \"${EXPECTED_SHOGIESA_CORE//./\\.}\"$" \
  "$TMP_DIR/sekirei/crates/sekirei-train/Cargo.toml"; then
  printf 'error: Sekirei %s does not pin shogiesa-core %s\n' \
    "$SEKIREI_REF" "$EXPECTED_SHOGIESA_CORE" >&2
  exit 1
fi

TEST_STATUS="skipped"
if [[ "$RUN_TESTS" -eq 1 ]]; then
  TEST_STATUS="pass"
  CARGO_TARGET_DIR="$TMP_DIR/target" cargo test \
    --manifest-path "$TMP_DIR/sekirei/Cargo.toml" \
    --locked -p sekirei-train 'positions::tests::'
fi

if [[ -n "$OUT" ]]; then
  mkdir -p "$(dirname "$OUT_DEST")"
  OUT_TMP="$(mktemp "$(dirname "$OUT_DEST")/.sekirei-compat.XXXXXX")"
  jq -n \
    --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg shogiesa_commit "$SHOGIESA_COMMIT" \
    --arg shogiesa_source "$PRODUCER_SOURCE" \
    --arg sekirei_ref "$SEKIREI_REF" \
    --arg sekirei_commit "$SEKIREI_COMMIT" \
    --arg shogiesa_core_dependency "$EXPECTED_SHOGIESA_CORE" \
    --arg fixture_sha256 "$PRODUCER_SHA256" \
    --arg test_status "$TEST_STATUS" \
    --argjson shogiesa_dirty "$SHOGIESA_DIRTY" \
    '{
      schema: "shogiesa.sekirei-compat.v1",
      generated_at: $generated_at,
      producer: {
        repository: "kent-tokyo/shogiesa",
        commit: $shogiesa_commit,
        source: $shogiesa_source,
        working_tree_dirty: $shogiesa_dirty,
        schema_version: 11,
        fixture_sha256: $fixture_sha256
      },
      consumer: {
        repository: "kent-tokyo/sekirei",
        ref: $sekirei_ref,
        commit: $sekirei_commit,
        shogiesa_core_dependency: $shogiesa_core_dependency,
        vendored_fixture_sha256: $fixture_sha256
      },
      checks: {
        canonical_fixture_byte_identity: "pass",
        typed_positions_tests: $test_status
      }
    }' > "$OUT_TMP"
  mv -f "$OUT_TMP" "$OUT_DEST"
  OUT_TMP=""
fi

printf 'Sekirei compatibility: PASS\n'
printf '  ref: %s (%s)\n' "$SEKIREI_REF" "$SEKIREI_COMMIT"
printf '  canonical fixture SHA-256: %s\n' "$PRODUCER_SHA256"
printf '  typed positions tests: %s\n' "$TEST_STATUS"
[[ -z "$OUT" ]] || printf '  artifact: %s\n' "$OUT"
