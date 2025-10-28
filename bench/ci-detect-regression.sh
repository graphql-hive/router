#!/bin/bash

# This script is meant to be run in CI (./github/workflows/ci.yaml#router-benchmark).
#
# It's a script to detect performance regression between two k6 benchmark summary files.
# It compares the 'http_reqs' rate metric from the PR summary against the main branch
# summary and determines if there is a regression of more than 5%.
# If a regression is detected, the script exits with a non-zero status code.

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

PR_SUMMARY="./bench/results/pr/k6_summary.json"
MAIN_SUMMARY="./bench/results/main/k6_summary.json"

# Check if the summary files exist
if [ ! -f "$PR_SUMMARY" ] || [ ! -f "$MAIN_SUMMARY" ]; then
    echo "Benchmark summary files ($PR_SUMMARY and/or $MAIN_SUMMARY) not found."
    exit 1
fi

# Extract the rate values
MAIN_RATE=$(jq '.metrics.http_reqs.values.rate' "$MAIN_SUMMARY")
PR_RATE=$(jq '.metrics.http_reqs.values.rate' "$PR_SUMMARY")

# Check if jq successfully extracted the rates
if [ -z "$MAIN_RATE" ] || [ -z "$PR_RATE" ] || [ "$MAIN_RATE" == "null" ] || [ "$PR_RATE" == "null" ]; then
    echo "Could not extract rate from one or both summary files."
    exit 1
fi

# Handle case where main rate is 0 to avoid division by zero
if (( $(echo "$MAIN_RATE == 0" | bc -l) )); then
    echo "Main branch rate is zero, cannot calculate percentage change."
    # If the main rate is 0, any positive PR rate is an improvement, not a regression.
    exit 0
fi

# Calculate the percentage difference using bc
# scale determines the number of decimal places
diff=$(echo "scale=4; (($PR_RATE - $MAIN_RATE) / $MAIN_RATE) * 100" | bc)

# Print the results
echo "Main branch http_reqs rate: $MAIN_RATE"
echo "PR branch http_reqs rate:   $PR_RATE"
printf "Difference: %.2f%%\n" "$diff"

# Check if the difference is a regression of more than 5%
# bc returns 1 for true, 0 for false. We compare with a negative number.
is_regression=$(echo "$diff < -5" | bc)

if [ "$is_regression" -eq 1 ]; then
    echo "Performance regression detected! The PR is more than 2% slower than main."
    exit 1
else
    echo "No significant performance regression detected."
    exit 0
fi
