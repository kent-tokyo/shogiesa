#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

printf '%s\n' '== repository contract =='
bash scripts/check_repository_contract.sh

printf '%s\n' '== formatting =='
cargo fmt --all -- --check

printf '%s\n' '== fixture-backed measurement smoke =='
for test_name in \
  report_bounded_streaming_matches_pre_refactor_golden_output \
  conflict_report_excludes_unknown_draw_and_mate_and_counts_cp_sign_conflicts \
  split_train_valid_test_deterministic_with_seed
do
  cargo test --offline -p shogiesa-cli --test cli_test "$test_name" -- --exact
done

printf '%s\n' 'local measurement smoke: PASS'
