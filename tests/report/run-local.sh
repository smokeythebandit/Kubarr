#!/usr/bin/env bash
# Isolated SQLite/unit/mock-browser lanes only. No database URL or cluster access.
set -euo pipefail
cd "$(dirname "$0")/../.."
result_dir=code/frontend/test-results
mkdir -p "$result_dir"
run_dir=$(mktemp -d "$result_dir/ledger-input.XXXXXX")
rm -f "$result_dir/unit.json" "$result_dir/browser-results.json"
report() {
  python3 -B tests/report/generate.py \
    --rust "$run_dir/rust-lib.log" \
    --rust "$run_dir/rust-settings.log" \
    --rust "$run_dir/rust-registration.log" \
    --vitest "$result_dir/unit.json" \
    --playwright "browser=$result_dir/browser-results.json" \
    --output "$result_dir/ledger"
}
trap report EXIT
(
  cd code/api
  bash ../../tests/report/run-rust.sh "../../$run_dir/rust-lib.log" cargo test --locked --lib
  bash ../../tests/report/run-rust.sh "../../$run_dir/rust-settings.log" cargo test --locked --test settings_endpoint_tests
  bash ../../tests/report/run-rust.sh "../../$run_dir/rust-registration.log" cargo test --locked --test registration_settings_tests
)
(
  cd code/frontend
  node node_modules/vitest/vitest.mjs run --reporter=default --reporter=json --outputFile.json=test-results/unit.json
  node node_modules/@playwright/test/cli.js test --config playwright.browser.config.ts
)
