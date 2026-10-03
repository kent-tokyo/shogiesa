#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

usage() {
  printf 'usage: %s --out PATH\n' "$0" >&2
}

if [[ "${1:-}" != "--out" || -z "${2:-}" || -n "${3:-}" ]]; then
  usage
  exit 2
fi

OUT_PATH="$2"
SHOGIESA_BIN="${SHOGIESA_BIN:-$ROOT_DIR/target/debug/shogiesa}"
FAKE_ENGINE_BIN="${FAKE_ENGINE_BIN:-$ROOT_DIR/target/debug/fake-usi-engine}"

for tool in jq shasum awk mktemp; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf 'required tool not found: %s\n' "$tool" >&2
    exit 2
  fi
done

if [[ ! -x "$SHOGIESA_BIN" || ! -x "$FAKE_ENGINE_BIN" ]]; then
  cargo build --offline -p shogiesa-cli -p fake-usi-engine
fi

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/shogiesa-repro-matrix.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

sha256_file() {
  shasum -a 256 "$1" | awk '{print $1}'
}

identity_hash() {
  jq -c '{sfen, source}' "$1" | LC_ALL=C sort | shasum -a 256 | awk '{print $1}'
}

require_equal() {
  local label="$1"
  local left="$2"
  local right="$3"
  if [[ "$left" != "$right" ]]; then
    printf '%s mismatch: %s != %s\n' "$label" "$left" "$right" >&2
    exit 1
  fi
}

require_different() {
  local label="$1"
  local left="$2"
  local right="$3"
  if [[ "$left" == "$right" ]]; then
    printf '%s unexpectedly matched: %s\n' "$label" "$left" >&2
    exit 1
  fi
}

run_shuffle() {
  local input="$1"
  local prefix="$2"
  "$SHOGIESA_BIN" shuffle \
    --input "$input" \
    --out "$prefix.jsonl" \
    --seed 20261003 \
    --manifest "$prefix.manifest.json" \
    --order-manifest "$prefix.order.jsonl"
}

mkdir -p "$TMP_DIR/root-a" "$TMP_DIR/root-b"
cp -R tests/fixtures/recursive_extract/. "$TMP_DIR/root-a/"
cp -R tests/fixtures/recursive_extract/. "$TMP_DIR/root-b/"

"$SHOGIESA_BIN" extract --input "$TMP_DIR/root-a" --recursive --out "$TMP_DIR/path-a.jsonl"
"$SHOGIESA_BIN" extract --input "$TMP_DIR/root-b" --recursive --out "$TMP_DIR/path-b.jsonl"
run_shuffle "$TMP_DIR/path-a.jsonl" "$TMP_DIR/path-a-shuffled"
run_shuffle "$TMP_DIR/path-b.jsonl" "$TMP_DIR/path-b-shuffled"

path_a_sha="$(sha256_file "$TMP_DIR/path-a.jsonl")"
path_b_sha="$(sha256_file "$TMP_DIR/path-b.jsonl")"
path_a_identity="$(identity_hash "$TMP_DIR/path-a.jsonl")"
path_b_identity="$(identity_hash "$TMP_DIR/path-b.jsonl")"
path_a_order="$(jq -r '.order_hash' "$TMP_DIR/path-a-shuffled.manifest.json")"
path_b_order="$(jq -r '.order_hash' "$TMP_DIR/path-b-shuffled.manifest.json")"
require_equal "path dataset sha256" "$path_a_sha" "$path_b_sha"
require_equal "path identity hash" "$path_a_identity" "$path_b_identity"
require_equal "path order hash" "$path_a_order" "$path_b_order"

awk '{ lines[NR] = $0 } END { for (i = NR; i >= 1; i--) print lines[i] }' \
  "$TMP_DIR/path-a.jsonl" > "$TMP_DIR/reversed.jsonl"
run_shuffle "$TMP_DIR/path-a.jsonl" "$TMP_DIR/order-forward"
run_shuffle "$TMP_DIR/reversed.jsonl" "$TMP_DIR/order-reversed"

forward_input_sha="$(sha256_file "$TMP_DIR/path-a.jsonl")"
reversed_input_sha="$(sha256_file "$TMP_DIR/reversed.jsonl")"
forward_identity="$(identity_hash "$TMP_DIR/path-a.jsonl")"
reversed_identity="$(identity_hash "$TMP_DIR/reversed.jsonl")"
forward_output_sha="$(sha256_file "$TMP_DIR/order-forward.jsonl")"
reversed_output_sha="$(sha256_file "$TMP_DIR/order-reversed.jsonl")"
forward_order="$(jq -r '.order_hash' "$TMP_DIR/order-forward.manifest.json")"
reversed_order="$(jq -r '.order_hash' "$TMP_DIR/order-reversed.manifest.json")"
require_different "input-order source sha256" "$forward_input_sha" "$reversed_input_sha"
require_equal "input-order identity hash" "$forward_identity" "$reversed_identity"
require_equal "input-order shuffled dataset sha256" "$forward_output_sha" "$reversed_output_sha"
require_equal "input-order order hash" "$forward_order" "$reversed_order"

for jobs in 1 2; do
  "$SHOGIESA_BIN" label \
    --input "$TMP_DIR/path-a.jsonl" \
    --engine "$FAKE_ENGINE_BIN" \
    --engine-name reproducibility-matrix-fake \
    --depths 1 \
    --jobs "$jobs" \
    --timeout-ms 2000 \
    --preserve-order \
    --out "$TMP_DIR/jobs-$jobs.jsonl" \
    --manifest "$TMP_DIR/jobs-$jobs.manifest.json"
  run_shuffle "$TMP_DIR/jobs-$jobs.jsonl" "$TMP_DIR/jobs-$jobs-shuffled"
done

jobs_1_sha="$(sha256_file "$TMP_DIR/jobs-1.jsonl")"
jobs_2_sha="$(sha256_file "$TMP_DIR/jobs-2.jsonl")"
jobs_1_identity="$(identity_hash "$TMP_DIR/jobs-1.jsonl")"
jobs_2_identity="$(identity_hash "$TMP_DIR/jobs-2.jsonl")"
jobs_1_order="$(jq -r '.order_hash' "$TMP_DIR/jobs-1-shuffled.manifest.json")"
jobs_2_order="$(jq -r '.order_hash' "$TMP_DIR/jobs-2-shuffled.manifest.json")"
require_equal "worker-count dataset sha256" "$jobs_1_sha" "$jobs_2_sha"
require_equal "worker-count identity hash" "$jobs_1_identity" "$jobs_2_identity"
require_equal "worker-count order hash" "$jobs_1_order" "$jobs_2_order"

source_commit="$(git rev-parse HEAD)"
if git diff --quiet && git diff --cached --quiet && [[ -z "$(git ls-files --others --exclude-standard)" ]]; then
  worktree_clean=true
else
  worktree_clean=false
fi
record_count="$(wc -l < "$TMP_DIR/path-a.jsonl" | tr -d ' ')"
generated_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
script_sha="$(sha256_file scripts/run_reproducibility_matrix.sh)"
shogiesa_version="$($SHOGIESA_BIN --version | awk '{print $2}')"

mkdir -p "$(dirname "$OUT_PATH")"
jq -n \
  --arg generated_at "$generated_at" \
  --arg source_commit "$source_commit" \
  --arg script_sha256 "$script_sha" \
  --arg shogiesa_version "$shogiesa_version" \
  --argjson worktree_clean "$worktree_clean" \
  --argjson record_count "$record_count" \
  --arg path_sha "$path_a_sha" \
  --arg path_identity "$path_a_identity" \
  --arg path_order "$path_a_order" \
  --arg forward_input_sha "$forward_input_sha" \
  --arg reversed_input_sha "$reversed_input_sha" \
  --arg order_identity "$forward_identity" \
  --arg order_output_sha "$forward_output_sha" \
  --arg input_order_hash "$forward_order" \
  --arg worker_output_sha "$jobs_1_sha" \
  --arg worker_identity "$jobs_1_identity" \
  --arg worker_order "$jobs_1_order" \
  '{
    schema_version: 1,
    generated_at: $generated_at,
    source_commit: $source_commit,
    worktree_clean: $worktree_clean,
    harness_sha256: $script_sha256,
    shogiesa_version: $shogiesa_version,
    fixture: "tests/fixtures/recursive_extract",
    record_count: $record_count,
    axes: [
      {
        axis: "input_path",
        variants: ["independent absolute root A", "independent absolute root B"],
        dataset_sha256: $path_sha,
        identity_hash: $path_identity,
        order_hash: $path_order,
        result: "pass",
        difference_reason: "absolute fixture roots differ; recursive extraction records portable relative source paths"
      },
      {
        axis: "input_order",
        variants: ["fixture order", "reversed JSONL order"],
        input_sha256: [$forward_input_sha, $reversed_input_sha],
        identity_hash: $order_identity,
        shuffled_dataset_sha256: $order_output_sha,
        order_hash: $input_order_hash,
        result: "pass",
        difference_reason: "raw input byte order differs; seeded shuffle canonicalizes by stable sample identity"
      },
      {
        axis: "worker_count",
        variants: [1, 2],
        labeled_dataset_sha256: $worker_output_sha,
        identity_hash: $worker_identity,
        order_hash: $worker_order,
        result: "pass",
        difference_reason: "jobs and runtime throughput diagnostics differ; preserve-order keeps dataset content and order identical"
      }
    ],
    overall: "pass"
  }' > "$OUT_PATH"

printf 'reproducibility matrix: PASS (%s)\n' "$OUT_PATH"
