#!/bin/bash

set -e
set -o pipefail

IGNORED_CRATES=("node-addon")
IGNORED_JSON=$(printf '%s\n' "${IGNORED_CRATES[@]}" | jq -R . | jq -s .)

WORKSPACE_DIRS=("." "apollo-router-workspace")

publish_list=""

for dir in "${WORKSPACE_DIRS[@]}"; do
    echo "fetching workspace metadata from Cargo in '$dir'..."

    METADATA=$(cargo metadata --format-version 1 --no-deps --manifest-path "$dir/Cargo.toml")

    CRATES_TO_CHECK=$(echo "$METADATA" | jq -r --argjson ignored "$IGNORED_JSON" '
        .workspace_members as $members | .packages[] |
        select(.id | IN($members[])) |
        select(.publish != []) |
        select(.name | IN($ignored[]) | not) |
        [.name, .version, .manifest_path] | @tsv
    ')

    if [ -z "$CRATES_TO_CHECK" ]; then
        echo "🤷 No publishable workspace crates found in '$dir'."
        continue
    fi

    echo "---"

    while IFS=$'\t' read -r name version manifest_path; do
        echo "checking $name@$version (manifest: $manifest_path) ..."

        if ! cargo info "$name@$version" --registry crates-io; then
          echo "   [ ] NOT PUBLISHED"
          publish_list="${publish_list}${name}\t${manifest_path}\n"
        else
          echo "   [x] ALREADY PUBLISHED"
        fi
        echo "---"
    done <<< "$CRATES_TO_CHECK"
done

CRATES_TO_PUBLISH_JSON=$(echo -e "$publish_list" | jq -R -s '
    split("\n") |                 # Split the input string by newlines
    map(select(length > 0)) |     # Remove any empty lines
    map(split("\t")) |            # Split each line by the tab character
    map({key: .[0], value: .[1]}) | # Format as key-value pairs
    from_entries                  # Convert the array into an object
')

echo "Crates to publish:"
echo "$CRATES_TO_PUBLISH_JSON" | jq .

if [ -n "$GITHUB_OUTPUT" ]; then
    echo "Setting GitHub Actions output 'crates_to_publish'..."
    echo "crates_to_publish=$(echo "$CRATES_TO_PUBLISH_JSON" | jq -c .)" >> "$GITHUB_OUTPUT"
fi
