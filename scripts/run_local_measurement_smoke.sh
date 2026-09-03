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
  dataset_diff_fixture_reports_semantic_changes_independent_of_input_order \
  split_train_valid_test_deterministic_with_seed \
  pack_fixture_round_trip_and_manifest_hashes_are_stable
do
  cargo test --offline -p shogiesa-cli --test cli_test "$test_name" -- --exact
done

printf '%s\n' 'local measurement smoke: PASS'
