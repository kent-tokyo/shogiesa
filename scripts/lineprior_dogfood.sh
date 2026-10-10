#!/usr/bin/env bash
set -euo pipefail

# Why: whether historical shogi move priors are worth candidate-ordering inside Sekirei can only
# be answered by measurement -- export real games, tune/eval the external `lineprior` tool against
# them, read coverage/fallback-rate/top-k-hit-rate/MRR -- not by guessing. This script only wires
# `shogiesa lineprior export` -> `lineprior tune` -> `lineprior eval` -> a report.md together; it
# has no opinion of its own on whether the numbers are good enough, and it does not touch Sekirei
# search in any way. `lineprior` is never built or vendored here -- it's the caller's own external
# binary, passed in by path.
#
# The report mapping follows lineprior 0.12.3's EvalReport contract: top-k values live in the
# `topk_hit_rate` array and MRR is `mean_reciprocal_rank`. Strict mode keeps contract drift visible.
#
# Requires: jq (for reading manifest/report JSON fields into report.md).

usage() {
  cat <<'EOF'
Usage: scripts/lineprior_dogfood.sh --games PATH --lineprior PATH --out DIR --source NAME \
         [--shogiesa PATH] [--max-ply N]

Required:
  --games PATH        Directory (or single file) of CSA/KIF game records
  --lineprior PATH    Path to the lineprior binary (external tool, not built by this repo)
  --out DIR           Output directory for this run (created if missing)
  --source NAME       Label written to every exported observation's `source` field

Options:
  --shogiesa PATH   Path to the shogiesa binary (default: "target/release/shogiesa" --
                     run `cargo build --release` first, or pass e.g.
                     "cargo run --quiet --release -p shogiesa-cli --")
  --max-ply N       Max ply to export per game (default: 80)
  --strict-report-fields
                    Fail (after still writing report.md) if any required eval metric
                    (coverage/fallback_rate/top1_hit_rate/k=3/k=5/mean_reciprocal_rank) comes
                    back missing/"n/a" -- catches a lineprior JSON field-name mismatch that would
                    otherwise silently produce an all-"n/a" report and exit 0. Off by default:
                    a field mismatch should stay non-fatal for exploratory runs, where a partial
                    report is still useful while iterating; a reproducible/logged dogfood run
                    wants a hard failure instead of a report nobody double-checks by eye.
  -h, --help        Show this help
EOF
}

GAMES=""
LINEPRIOR_BIN=""
OUT_DIR=""
SOURCE_NAME=""
SHOGIESA_BIN="target/release/shogiesa"
MAX_PLY="80"
STRICT_REPORT_FIELDS=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --games) GAMES="$2"; shift 2 ;;
    --lineprior) LINEPRIOR_BIN="$2"; shift 2 ;;
    --out) OUT_DIR="$2"; shift 2 ;;
    --source) SOURCE_NAME="$2"; shift 2 ;;
    --shogiesa) SHOGIESA_BIN="$2"; shift 2 ;;
    --max-ply) MAX_PLY="$2"; shift 2 ;;
    --strict-report-fields) STRICT_REPORT_FIELDS="1"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 1 ;;
  esac
done

if [[ -z "$GAMES" || -z "$LINEPRIOR_BIN" || -z "$OUT_DIR" || -z "$SOURCE_NAME" ]]; then
  echo "error: --games, --lineprior, --out, and --source are required" >&2
  usage >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required (used to read manifest/report JSON fields into report.md)" >&2
  exit 1
fi
if [[ ! -x "$LINEPRIOR_BIN" ]] && ! command -v "$LINEPRIOR_BIN" >/dev/null 2>&1; then
  echo "error: --lineprior binary not found or not executable: $LINEPRIOR_BIN" >&2
  exit 1
fi

if [[ -x "$LINEPRIOR_BIN" ]]; then
  LINEPRIOR_PATH="$LINEPRIOR_BIN"
else
  LINEPRIOR_PATH="$(command -v "$LINEPRIOR_BIN")"
fi

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    echo "error: sha256sum or shasum is required to identify the lineprior binary" >&2
    return 1
  fi
}

LINEPRIOR_VERSION="$("$LINEPRIOR_PATH" --version | tr -d '\r' | head -n 1)"
LINEPRIOR_SHA256="$(sha256_file "$LINEPRIOR_PATH")"

mkdir -p "$OUT_DIR"
echo "run directory: $OUT_DIR"

shogiesa() { $SHOGIESA_BIN "$@"; }
lineprior() { "$LINEPRIOR_PATH" "$@"; }

OBSERVATIONS="$OUT_DIR/shogi_observations.jsonl"
EXPORT_MANIFEST="$OUT_DIR/export_manifest.json"
BEST_CONFIG="$OUT_DIR/shogi_best_config.json"
TUNE_REPORT="$OUT_DIR/shogi_tune_report.json"
EVAL_REPORT="$OUT_DIR/shogi_eval_report.json"

echo "== export =="
shogiesa lineprior export \
  --input "$GAMES" \
  --out "$OBSERVATIONS" \
  --state-format sfen \
  --action-format usi \
  --max-ply "$MAX_PLY" \
  --source "$SOURCE_NAME" \
  --outcome-mode game-result \
  --score-mode none \
  --manifest "$EXPORT_MANIFEST"

SEQUENCE_COUNT="$(jq -er '.sequence_count | select(type == "number")' "$EXPORT_MANIFEST")"
if (( SEQUENCE_COUNT < 2 )); then
  echo "error: lineprior dogfood requires at least two sequences for a held-out sequence split; got $SEQUENCE_COUNT" >&2
  exit 2
fi

echo "== tune =="
if ! lineprior tune "$OBSERVATIONS" \
  --split-by sequence \
  --train-ratio 0.8 \
  --param confidence-mode=heuristic,wilson-lower-bound,hybrid \
  --param min-confidence=0.0,0.3,0.5,0.7 \
  --param smoothing-alpha=1.0,5.0,10.0 \
  --objective covered-mrr \
  --save-best-config "$BEST_CONFIG" \
  --out "$TUNE_REPORT"
then
  echo "error: lineprior tune failed; verify that the deterministic sequence split has non-empty train and test partitions" >&2
  exit 1
fi

echo "== eval =="
if ! lineprior eval "$OBSERVATIONS" \
  --config "$BEST_CONFIG" \
  --calibration-bins 10 \
  --thresholds 0.3,0.5,0.7,0.9 \
  --out "$EVAL_REPORT"
then
  echo "error: lineprior eval failed; verify that the deterministic sequence split has non-empty train and test partitions" >&2
  exit 1
fi

eval_metric() {
  case "$1" in
    coverage|fallback_rate|top1_hit_rate)
      jq -r --arg field "$1" '.[$field] | if type == "number" then . else "n/a" end' "$EVAL_REPORT"
      ;;
    top3_hit_rate)
      jq -r '[.topk_hit_rate[]? | select(.k == 3) | .hit_rate | select(type == "number")][0] // "n/a"' "$EVAL_REPORT"
      ;;
    top5_hit_rate)
      jq -r '[.topk_hit_rate[]? | select(.k == 5) | .hit_rate | select(type == "number")][0] // "n/a"' "$EVAL_REPORT"
      ;;
    mrr)
      jq -r '.mean_reciprocal_rank | if type == "number" then . else "n/a" end' "$EVAL_REPORT"
      ;;
  esac
}

echo "== report =="
report="$OUT_DIR/report.md"
# Populated below inside the report block -- `{ ... } > "$report"` is a group command, not a
# subshell, so this array's mutations are visible after the block closes.
missing_fields=()
{
  echo "# lineprior dogfood report"
  echo
  echo "- generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "- games: \`$GAMES\`"
  echo "- source: $SOURCE_NAME, max-ply: $MAX_PLY"
  echo "- lineprior version: $LINEPRIOR_VERSION"
  echo "- lineprior binary SHA-256: \`$LINEPRIOR_SHA256\`"
  echo
  echo "## Export"
  echo
  records=$(jq -r '.records_exported // "n/a"' "$EXPORT_MANIFEST")
  sequences=$(jq -r '.sequence_count // "n/a"' "$EXPORT_MANIFEST")
  unknown=$(jq -r '.unknown_outcome_count // "n/a"' "$EXPORT_MANIFEST")
  outcomes=$(jq -c '.outcome_distribution // "n/a"' "$EXPORT_MANIFEST")
  echo "- observation count: $records"
  echo "- sequence count: $sequences"
  echo "- outcome distribution: \`$outcomes\`"
  echo "- unknown outcome count: $unknown"
  echo
  echo "## Eval metrics"
  echo
  echo "| metric | value |"
  echo "|---|---|"
  for field in coverage fallback_rate top1_hit_rate top3_hit_rate top5_hit_rate mrr; do
    value="$(eval_metric "$field")"
    echo "| $field | $value |"
    [[ "$value" == "n/a" ]] && missing_fields+=("$field")
  done
  echo
  echo "Especially watch \`top5_hit_rate\` and \`mrr\` over \`top1_hit_rate\` -- the intended"
  echo "Sekirei use case is candidate-set move ordering, not picking a single best move."
  echo
  echo "## Best config"
  echo
  echo "\`$BEST_CONFIG\`:"
  echo '```json'
  cat "$BEST_CONFIG"
  echo '```'
  echo
  echo "## Commands run"
  echo
  echo '```bash'
  echo "shogiesa lineprior export --input $GAMES --out $OBSERVATIONS \\"
  echo "  --state-format sfen --action-format usi --max-ply $MAX_PLY \\"
  echo "  --source $SOURCE_NAME --outcome-mode game-result --score-mode none \\"
  echo "  --manifest $EXPORT_MANIFEST"
  echo
  echo "$LINEPRIOR_PATH tune $OBSERVATIONS --split-by sequence --train-ratio 0.8 \\"
  echo "  --param confidence-mode=heuristic,wilson-lower-bound,hybrid \\"
  echo "  --param min-confidence=0.0,0.3,0.5,0.7 \\"
  echo "  --param smoothing-alpha=1.0,5.0,10.0 \\"
  echo "  --objective covered-mrr --save-best-config $BEST_CONFIG --out $TUNE_REPORT"
  echo
  echo "$LINEPRIOR_PATH eval $OBSERVATIONS --config $BEST_CONFIG \\"
  echo "  --calibration-bins 10 --thresholds 0.3,0.5,0.7,0.9 --out $EVAL_REPORT"
  echo '```'
} > "$report"

echo "done: $report"

if [[ -n "$STRICT_REPORT_FIELDS" && ${#missing_fields[@]} -gt 0 ]]; then
  echo "error: --strict-report-fields set and required eval metric(s) missing/n-a: ${missing_fields[*]}" >&2
  echo "error: check $EVAL_REPORT directly and fix the jq field names in the report step of this script if lineprior's actual output uses different keys" >&2
  exit 1
fi
