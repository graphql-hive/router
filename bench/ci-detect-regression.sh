#!/bin/bash

set -euo pipefail

# This script is meant to be run in CI and locally.
#
# It detects throughput regressions between two normalized benchmark summaries:
#   - ./bench/results/pr/summary.json
#   - ./bench/results/main/summary.json
#
# Expected summary shape:
# {
#   "rate_rps": 1234.56,
#   ...
# }

if ! command -v jq &> /dev/null; then
  echo "jq could not be found. Please install jq to run this script."
  exit 1
fi

if ! command -v bc &> /dev/null; then
  echo "bc could not be found. Please install bc to run this script."
  exit 1
fi

PR_SUMMARY="./bench/results/pr/summary.json"
MAIN_SUMMARY="./bench/results/main/summary.json"
REGRESSION_THRESHOLD="-5"

if [ ! -f "$PR_SUMMARY" ] || [ ! -f "$MAIN_SUMMARY" ]; then
  echo "Benchmark summary files ($PR_SUMMARY and/or $MAIN_SUMMARY) not found."
  exit 1
fi

MAIN_RATE=$(jq -r '.rate_rps' "$MAIN_SUMMARY")
PR_RATE=$(jq -r '.rate_rps' "$PR_SUMMARY")

if [ -z "$MAIN_RATE" ] || [ -z "$PR_RATE" ] || [ "$MAIN_RATE" = "null" ] || [ "$PR_RATE" = "null" ]; then
  echo "Could not extract rate_rps from one or both summary files."
  exit 1
fi

if ! [[ "$MAIN_RATE" =~ ^[0-9]+([.][0-9]+)?$ ]] || ! [[ "$PR_RATE" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  echo "Invalid numeric rate_rps value(s): main=$MAIN_RATE pr=$PR_RATE"
  exit 1
fi

if (( $(echo "$MAIN_RATE == 0" | bc -l) )); then
  echo "Main branch rate is zero, cannot calculate percentage change."
  exit 0
fi

diff=$(echo "scale=6; (($PR_RATE - $MAIN_RATE) / $MAIN_RATE) * 100" | bc -l)

echo "Main rate (rps):  $MAIN_RATE"
echo "PR rate (rps):    $PR_RATE"
printf "Difference: %.2f%%\n" "$diff"

is_regression=$(echo "$diff < $REGRESSION_THRESHOLD" | bc)
if [ "$is_regression" -eq 1 ]; then
  echo "Performance regression detected! The PR is more than 5% slower than main."
  exit 1
fi

echo "No significant performance regression detected."
exit 0
