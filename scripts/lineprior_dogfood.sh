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
# The exact `lineprior tune`/`lineprior eval` flag names and JSON output field names below reflect
# the CLI as described when this script was written, not a verified local install (none exists on
# this machine at the time of writing). Every report.md field is read via `jq '.field // "n/a"'`,
# so a field-name mismatch degrades to a readable "n/a" instead of crashing -- if `lineprior`'s
# actual output uses different keys, the jq expressions in the report-generation step below are
# the one place to fix it.
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
  -h, --help        Show this help
EOF
}

GAMES=""
LINEPRIOR_BIN=""
OUT_DIR=""
SOURCE_NAME=""
SHOGIESA_BIN="target/release/shogiesa"
MAX_PLY="80"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --games) GAMES="$2"; shift 2 ;;
    --lineprior) LINEPRIOR_BIN="$2"; shift 2 ;;
    --out) OUT_DIR="$2"; shift 2 ;;
    --source) SOURCE_NAME="$2"; shift 2 ;;
    --shogiesa) SHOGIESA_BIN="$2"; shift 2 ;;
    --max-ply) MAX_PLY="$2"; shift 2 ;;
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

mkdir -p "$OUT_DIR"
echo "run directory: $OUT_DIR"

shogiesa() { $SHOGIESA_BIN "$@"; }
lineprior() { "$LINEPRIOR_BIN" "$@"; }

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

echo "== tune =="
lineprior tune "$OBSERVATIONS" \
  --split-by sequence \
  --train-ratio 0.8 \
  --param confidence-mode=heuristic,wilson-lower-bound,hybrid \
  --param min-confidence=0.0,0.3,0.5,0.7 \
  --param smoothing-alpha=1.0,5.0,10.0 \
  --objective covered-mrr \
  --save-best-config "$BEST_CONFIG" \
  --out "$TUNE_REPORT"

echo "== eval =="
lineprior eval "$OBSERVATIONS" \
  --config "$BEST_CONFIG" \
  --calibration-bins 10 \
  --thresholds 0.3,0.5,0.7,0.9 \
  --out "$EVAL_REPORT"

echo "== report =="
report="$OUT_DIR/report.md"
{
  echo "# lineprior dogfood report"
  echo
  echo "- generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "- games: \`$GAMES\`"
  echo "- source: $SOURCE_NAME, max-ply: $MAX_PLY"
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
    value=$(jq -r --arg f "$field" '.[$f] // "n/a"' "$EVAL_REPORT")
    echo "| $field | $value |"
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
  echo "lineprior tune $OBSERVATIONS --split-by sequence --train-ratio 0.8 \\"
  echo "  --param confidence-mode=heuristic,wilson-lower-bound,hybrid \\"
  echo "  --param min-confidence=0.0,0.3,0.5,0.7 \\"
  echo "  --param smoothing-alpha=1.0,5.0,10.0 \\"
  echo "  --objective covered-mrr --save-best-config $BEST_CONFIG --out $TUNE_REPORT"
  echo
  echo "lineprior eval $OBSERVATIONS --config $BEST_CONFIG \\"
  echo "  --calibration-bins 10 --thresholds 0.3,0.5,0.7,0.9 --out $EVAL_REPORT"
  echo '```'
} > "$report"

echo "done: $report"
