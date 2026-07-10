#!/usr/bin/env bash
set -euo pipefail

# Test-only stand-in for the real (external, not-in-this-repo) `lineprior` binary. Recognizes just
# enough of `tune`/`eval`'s flag surface to exercise scripts/lineprior_dogfood.sh's plumbing
# (arg-passing, output-file wiring, report.md's jq extraction) without needing the real tool
# anywhere, including in CI. Writes fixed canned JSON -- it does not compute anything.

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
  "top3_hit_rate": 0.55,
  "top5_hit_rate": 0.67,
  "mrr": 0.44
}
EOF
    ;;
  *)
    echo "fake_lineprior.sh: unknown subcommand $subcommand" >&2
    exit 1
    ;;
esac
