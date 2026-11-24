#!/usr/bin/env bash
# Prints every criterion result from the last `cargo bench` as one flat JSON object:
#   {"<suite>/<bench id>": <median ns>, ...}
# The suite name keeps ids from different CI jobs apart when they get merged.
set -euo pipefail

suite="${1:?usage: $0 <suite name> [criterion dir]}"
dir="${2:-${CARGO_TARGET_DIR:-target}/criterion}"

# each bench leaves new/estimates.json (the numbers) next to new/benchmark.json (its id)
find "$dir" -path '*/new/estimates.json' -print0 | while IFS= read -r -d '' estimates; do
  jq -n --arg suite "$suite" \
    --slurpfile bench "${estimates%estimates.json}benchmark.json" \
    --slurpfile est "$estimates" \
    '{($suite + "/" + $bench[0].full_id): $est[0].median.point_estimate}'
done | jq -s 'add // {}'
