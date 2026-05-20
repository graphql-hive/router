#!/bin/bash

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

# Ensure jq and bc are installed
if ! command -v jq &> /dev/null
then
    echo "jq could not be found. Please install jq to run this script."
    exit 1
fi
if ! command -v bc &> /dev/null
then
    echo "bc could not be found. Please install bc to run this script."
    exit 1
fi

PR_SUMMARY="./bench/results/pr/summary.json"
MAIN_SUMMARY="./bench/results/main/summary.json"
REGRESSION_THRESHOLD="-5"

# Check if the summary files exist
if [ ! -f "$PR_SUMMARY" ] || [ ! -f "$MAIN_SUMMARY" ]; then
    echo "Benchmark summary files ($PR_SUMMARY and/or $MAIN_SUMMARY) not found."
    exit 1
fi

# Extract the rate values
MAIN_RATE=$(jq -r '.rate_rps' "$MAIN_SUMMARY")
PR_RATE=$(jq -r '.rate_rps' "$PR_SUMMARY")
MAIN_VALIDATION_FAILURES=$(jq -r '.validation_total_failures // 0' "$MAIN_SUMMARY")
PR_VALIDATION_FAILURES=$(jq -r '.validation_total_failures // 0' "$PR_SUMMARY")

# Check if jq successfully extracted the rates
if [ -z "$MAIN_RATE" ] || [ -z "$PR_RATE" ] || [ "$MAIN_RATE" = "null" ] || [ "$PR_RATE" = "null" ]; then
    echo "Could not extract rate_rps from one or both summary files."
    exit 1
fi

if ! [[ "$MAIN_RATE" =~ ^[0-9]+([.][0-9]+)?$ ]] || ! [[ "$PR_RATE" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
    echo "Invalid numeric rate_rps value(s): main=$MAIN_RATE pr=$PR_RATE"
    exit 1
fi

if ! [[ "$MAIN_VALIDATION_FAILURES" =~ ^[0-9]+$ ]] || ! [[ "$PR_VALIDATION_FAILURES" =~ ^[0-9]+$ ]]; then
    echo "Invalid validation_total_failures value(s): main=$MAIN_VALIDATION_FAILURES pr=$PR_VALIDATION_FAILURES"
    exit 1
fi

# Handle case where main rate is 0 to avoid division by zero
if (( $(echo "$MAIN_RATE == 0" | bc -l) )); then
    echo "Main branch rate is zero, cannot calculate percentage change."
    # If the main rate is 0, any positive PR rate is an improvement, not a regression.
    exit 0
fi

if [ "$MAIN_VALIDATION_FAILURES" -gt 0 ] || [ "$PR_VALIDATION_FAILURES" -gt 0 ]; then
    echo "Validation failures found in benchmark summaries."
    echo "Main validation_total_failures: $MAIN_VALIDATION_FAILURES"
    echo "PR validation_total_failures:   $PR_VALIDATION_FAILURES"
    exit 1
fi

# Calculate the percentage difference using bc
diff=$(echo "scale=6; (($PR_RATE - $MAIN_RATE) / $MAIN_RATE) * 100" | bc -l)

# Print the results
echo "Main rate (rps):  $MAIN_RATE"
echo "PR rate (rps):    $PR_RATE"
printf "Difference: %.2f%%\n" "$diff"

# Check if the difference is a regression of more than 5%
# bc returns 1 for true, 0 for false. We compare with a negative number.
is_regression=$(echo "$diff < $REGRESSION_THRESHOLD" | bc)

if [ "$is_regression" -eq 1 ]; then
    echo "Performance regression detected! The PR is more than 5% slower than main."
    exit 1
else
    echo "No significant performance regression detected."
    exit 0
fi
