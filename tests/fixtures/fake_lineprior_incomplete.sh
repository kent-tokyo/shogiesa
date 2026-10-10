#!/usr/bin/env bash
set -euo pipefail

# Test-only stand-in for current lineprior's EvalReport shape with one required metric omitted.
# FAKE_LINEPRIOR_OMIT selects top3, top5, or mrr for fail-closed regression coverage.

if [[ "${1:-}" == "--version" ]]; then
  echo "lineprior 0.12.3-incomplete-fixture"
  exit 0
fi

subcommand="${1:-}"
shift || true

out=""
save_best_config=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) out="$2"; shift 2 ;;
    --save-best-config) save_best_config="$2"; shift 2 ;;
    *) shift ;;
  esac
done

case "$subcommand" in
  tune)
    [[ -n "$save_best_config" ]] && echo '{"confidence-mode":"hybrid","min-confidence":0.5,"smoothing-alpha":5.0}' > "$save_best_config"
    [[ -n "$out" ]] && echo '{"best_arm":"hybrid"}' > "$out"
    ;;
  eval)
    case "${FAKE_LINEPRIOR_OMIT:-top5}" in
      top3)
        [[ -n "$out" ]] && cat > "$out" <<'EOF'
{
  "coverage": 0.42,
  "fallback_rate": 0.18,
  "top1_hit_rate": 0.31,
  "topk_hit_rate": [
    {"k": 1, "hit_rate": 0.31},
    {"k": 5, "hit_rate": 0.67}
  ],
  "mean_reciprocal_rank": 0.44
}
EOF
        ;;
      top5)
        [[ -n "$out" ]] && cat > "$out" <<'EOF'
{
  "coverage": 0.42,
  "fallback_rate": 0.18,
  "top1_hit_rate": 0.31,
  "topk_hit_rate": [
    {"k": 1, "hit_rate": 0.31},
    {"k": 3, "hit_rate": 0.55}
  ],
  "mean_reciprocal_rank": 0.44
}
EOF
        ;;
      mrr)
        [[ -n "$out" ]] && cat > "$out" <<'EOF'
{
  "coverage": 0.42,
  "fallback_rate": 0.18,
  "top1_hit_rate": 0.31,
  "topk_hit_rate": [
    {"k": 1, "hit_rate": 0.31},
    {"k": 3, "hit_rate": 0.55},
    {"k": 5, "hit_rate": 0.67}
  ]
}
EOF
        ;;
      *)
        echo "fake_lineprior_incomplete.sh: unsupported FAKE_LINEPRIOR_OMIT=${FAKE_LINEPRIOR_OMIT}" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "fake_lineprior_incomplete.sh: unknown subcommand $subcommand" >&2
    exit 1
    ;;
esac
