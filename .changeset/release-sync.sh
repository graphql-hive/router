#!/bin/sh
set -e

CHANGESET_DIR=".changeset"

# Define your list of target directories here (space-separated)
# You can also override this by passing arguments to the script: TARGET_DIRS="${@:-.}"
TARGET_DIRS=". ./apollo-router-workspace"

# Function to extract package names and their corresponding filenames
get_pkg_file_pairs() {
    grep -rE "^[^:]+: (patch|minor|major)" "$CHANGESET_DIR"/*.md 2>/dev/null | \
    sed 's/\.md:/.md|/' | sed 's/:.*//' | awk -F'|' '{print $2":"$1}'
}

echo "🔍 Cascading changes (scoped per file)..."

for DIR in $TARGET_DIRS; do
    echo "📂 Processing workspace: $DIR"

    MANIFEST_PATH="$DIR/Cargo.toml"

    if [ ! -f "$MANIFEST_PATH" ]; then
        echo "⚠️  No Cargo.toml found at $MANIFEST_PATH. Skipping..."
        continue
    fi

    # Fetch metadata for the specific directory
    METADATA=$(cargo metadata --manifest-path "$MANIFEST_PATH" --format-version 1 --no-deps)

    while true; do
        PAIRS=$(get_pkg_file_pairs)
        NEW_CHANGE_FOUND=false

        for PAIR in $PAIRS; do
            PKG=$(echo "$PAIR" | cut -d: -f1)
            FILE=$(echo "$PAIR" | cut -d: -f2)

            # Find publishable packages that depend on $PKG
            DEPENDENTS=$(echo "$METADATA" | jq -r ".packages[] |
                select(.dependencies[].name == \"$PKG\") |
                select(.publish == null or (.publish | length > 0)) |
                .name")

            for DEP in $DEPENDENTS; do
                # SCOPED CHECK: Only check if the dependency is missing FROM THIS SPECIFIC FILE
                if ! grep -qE "^$DEP:" "$FILE" 2>/dev/null; then
                    echo "🔗 In $FILE: $DEP depends on $PKG. Adding..."

                    # macOS/BSD compatible sed insertion
                    sed -i '' "2,\$s/^---$/$DEP: patch\\
---/" "$FILE"

                    NEW_CHANGE_FOUND=true
                fi
            done
        done

        # If no files were modified in this pass for this directory, break the while loop
        if [ "$NEW_CHANGE_FOUND" = false ]; then
            break
        fi
    done
done

echo "✅ All changesets internally consistent across all directories."
