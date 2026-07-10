#!/usr/bin/env bash
set -euo pipefail

# Test-only stand-in for a `lineprior` whose JSON output uses different field names than
# scripts/lineprior_dogfood.sh's jq expressions expect -- same shape as fake_lineprior.sh, but
# `eval`'s output is missing top5_hit_rate/mrr, to exercise --strict-report-fields's failure path
# without needing a real mismatched install anywhere, including in CI.

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
    [[ -n "$out" ]] && cat > "$out" <<'EOF'
{
  "coverage": 0.42,
  "fallback_rate": 0.18,
  "top1_hit_rate": 0.31,
  "top3_hit_rate": 0.55
}
EOF
    ;;
  *)
    echo "fake_lineprior_incomplete.sh: unknown subcommand $subcommand" >&2
    exit 1
    ;;
esac
