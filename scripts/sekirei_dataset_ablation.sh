#!/usr/bin/env bash
set -euo pipefail

# Why: whether a dataset condition (a tuning preset) actually moves Sekirei's playing strength
# can only be answered by ablation -- training on each candidate dataset and gating the result --
# not by dataset-side coverage/mismatch metrics alone. shogiesa produces data, Sekirei trains,
# veridict judges; this script only wires the three together for measurement. It does not embed
# Sekirei's training or gating logic -- both are external hooks (see SEKIREI_TRAIN_CMD /
# SEKIREI_GATE_CMD below) so that boundary stays intact.
#
# Arms: baseline, tune-broad, tune-balanced, tune-strict. `tune --preset-out` is run once (it
# already resolves all three broad/balanced/strict candidates in one pass); each tune-* arm then
# runs `filter --preset tuning.json:<arm>` against that same file. `baseline` runs `filter` with
# no gates at all, so every arm goes through the same filter/manifest code path.
#
# Requires: jq (for reading manifest.json fields into report.md).

usage() {
  cat <<'EOF'
Usage: scripts/sekirei_dataset_ablation.sh --input LABELED.jsonl --teacher-depth N \
         --student-depths "6,8,10" [--sweep-policy-margin "0,40,80,120"] \
         [--sweep-score-swing "50,100,150,200"] [--shogiesa PATH] [--out-dir PATH]

Required:
  --input PATH             Labeled positions JSONL (must contain both --teacher-depth and every
                            depth in --student-depths, e.g. from one
                            `label --depths 6,8,10,14` run)
  --teacher-depth N         Depth treated as ground truth for tune's teacher/student compare
  --student-depths LIST     Comma-separated shallower depths to compare (e.g. "6,8,10")

At least one of:
  --sweep-policy-margin LIST   Comma-separated policy-margin values to sweep (e.g. "0,40,80,120")
  --sweep-score-swing LIST     Comma-separated score-swing values to sweep (e.g. "50,100,150,200")

Options:
  --shogiesa PATH   Path to the shogiesa binary (default: "cargo run --release -p shogiesa-cli --")
  --out-dir PATH    Parent directory for this run (default: runs/<UTC timestamp>)
  -h, --help        Show this help

Environment (Sekirei hooks -- both optional; an arm's training/gate step is skipped with a
"not configured" note in report.md if unset, rather than guessing Sekirei's CLI shape):
  SEKIREI_DIR         Path to a Sekirei checkout. Only used to record its commit SHA in
                      report.md (via `git -C "$SEKIREI_DIR" rev-parse HEAD`); never invoked
                      directly.
  SEKIREI_TRAIN_CMD   Command run as: $SEKIREI_TRAIN_CMD <train.jsonl> <arm-dir>
                      Expected to leave a model artifact and its own logs under <arm-dir>; this
                      script additionally captures its stdout/stderr to
                      <arm-dir>/sekirei_train.log.
  SEKIREI_GATE_CMD    Command run as: $SEKIREI_GATE_CMD <arm-dir>
                      Expected to write <arm-dir>/gate_result.json itself. If it doesn't, this
                      script writes a minimal stub there instead of failing the whole run.
EOF
}

SHOGIESA_INPUT=""
TEACHER_DEPTH=""
STUDENT_DEPTHS=""
SWEEP_POLICY_MARGIN=""
SWEEP_SCORE_SWING=""
SHOGIESA_BIN="cargo run --quiet --release -p shogiesa-cli --"
OUT_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --input) SHOGIESA_INPUT="$2"; shift 2 ;;
    --teacher-depth) TEACHER_DEPTH="$2"; shift 2 ;;
    --student-depths) STUDENT_DEPTHS="$2"; shift 2 ;;
    --sweep-policy-margin) SWEEP_POLICY_MARGIN="$2"; shift 2 ;;
    --sweep-score-swing) SWEEP_SCORE_SWING="$2"; shift 2 ;;
    --shogiesa) SHOGIESA_BIN="$2"; shift 2 ;;
    --out-dir) OUT_DIR="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 1 ;;
  esac
done

if [[ -z "$SHOGIESA_INPUT" || -z "$TEACHER_DEPTH" || -z "$STUDENT_DEPTHS" ]]; then
  echo "error: --input, --teacher-depth, and --student-depths are required" >&2
  usage >&2
  exit 1
fi
if [[ -z "$SWEEP_POLICY_MARGIN" && -z "$SWEEP_SCORE_SWING" ]]; then
  echo "error: at least one of --sweep-policy-margin / --sweep-score-swing is required" >&2
  usage >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required (used to read manifest.json fields into report.md)" >&2
  exit 1
fi

if [[ -z "$OUT_DIR" ]]; then
  OUT_DIR="runs/$(date -u +%Y%m%d-%H%M)"
fi
mkdir -p "$OUT_DIR"
echo "run directory: $OUT_DIR"

shogiesa() { $SHOGIESA_BIN "$@"; }

# --- Step 1: tune once, resolving broad/balanced/strict together -----------------------------
TUNE_ARGS=(tune --input "$SHOGIESA_INPUT" --teacher-depth "$TEACHER_DEPTH" \
  --student-depths "$STUDENT_DEPTHS" \
  --out "$OUT_DIR/tune.csv" --report "$OUT_DIR/tune_report.md" \
  --preset-out "$OUT_DIR/tuning.json")
[[ -n "$SWEEP_POLICY_MARGIN" ]] && TUNE_ARGS+=(--sweep-policy-margin "$SWEEP_POLICY_MARGIN")
[[ -n "$SWEEP_SCORE_SWING" ]] && TUNE_ARGS+=(--sweep-score-swing "$SWEEP_SCORE_SWING")
echo "== tune =="
shogiesa "${TUNE_ARGS[@]}"

# --- Step 2: per-arm dataset prep + Sekirei hooks ---------------------------------------------
ARMS=(baseline tune-broad tune-balanced tune-strict)

sekirei_commit="unavailable"
if [[ -n "${SEKIREI_DIR:-}" ]] && git -C "$SEKIREI_DIR" rev-parse HEAD >/dev/null 2>&1; then
  sekirei_commit="$(git -C "$SEKIREI_DIR" rev-parse HEAD)"
fi

for arm in "${ARMS[@]}"; do
  arm_dir="$OUT_DIR/$arm"
  mkdir -p "$arm_dir"
  echo "== $arm: filter =="

  if [[ "$arm" == "baseline" ]]; then
    shogiesa filter --input "$SHOGIESA_INPUT" \
      --out "$arm_dir/train.jsonl" --manifest "$arm_dir/manifest.json"
  else
    preset_label="${arm#tune-}"
    shogiesa filter --input "$SHOGIESA_INPUT" --preset "$OUT_DIR/tuning.json:$preset_label" \
      --out "$arm_dir/train.jsonl" --manifest "$arm_dir/manifest.json"
  fi

  if [[ -n "${SEKIREI_TRAIN_CMD:-}" ]]; then
    echo "== $arm: train =="
    # shellcheck disable=SC2086 -- SEKIREI_TRAIN_CMD is a user-supplied command line, meant to
    # be word-split (e.g. "python train.py --epochs 3").
    $SEKIREI_TRAIN_CMD "$arm_dir/train.jsonl" "$arm_dir" >"$arm_dir/sekirei_train.log" 2>&1
  else
    echo "not configured: SEKIREI_TRAIN_CMD unset, skipping training" | tee "$arm_dir/sekirei_train.log" >/dev/null
  fi

  if [[ -n "${SEKIREI_GATE_CMD:-}" ]]; then
    echo "== $arm: gate =="
    # shellcheck disable=SC2086
    $SEKIREI_GATE_CMD "$arm_dir"
    if [[ ! -f "$arm_dir/gate_result.json" ]]; then
      jq -n --arg arm "$arm" \
        '{status: "gate_cmd_ran_but_wrote_no_gate_result_json", arm: $arm}' \
        > "$arm_dir/gate_result.json"
    fi
  else
    jq -n --arg arm "$arm" '{status: "skipped", reason: "SEKIREI_GATE_CMD not set", arm: $arm}' \
      > "$arm_dir/gate_result.json"
  fi
done

# --- Step 3: report.md summarizing every arm --------------------------------------------------
report="$OUT_DIR/report.md"
{
  echo "# Sekirei dataset ablation"
  echo
  echo "- generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "- input: \`$SHOGIESA_INPUT\`"
  echo "- teacher depth: $TEACHER_DEPTH, student depths: $STUDENT_DEPTHS"
  echo "- sekirei commit: \`$sekirei_commit\`"
  echo
  echo "| arm | records_kept | records_dropped | gate status |"
  echo "|---|---|---|---|"
  for arm in "${ARMS[@]}"; do
    arm_dir="$OUT_DIR/$arm"
    kept=$(jq -r '.records_kept // "n/a"' "$arm_dir/manifest.json")
    dropped=$(jq -r '.records_dropped // "n/a"' "$arm_dir/manifest.json")
    gate_status=$(jq -r '.status // "ok"' "$arm_dir/gate_result.json")
    echo "| $arm | $kept | $dropped | $gate_status |"
  done
  echo
  echo "Per-arm detail: \`$OUT_DIR/<arm>/manifest.json\`, \`$OUT_DIR/<arm>/sekirei_train.log\`, \`$OUT_DIR/<arm>/gate_result.json\`."
} > "$report"

echo "done: $report"
