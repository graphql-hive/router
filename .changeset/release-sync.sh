#!/bin/sh
set -e

CHANGESET_DIR=".changeset"

# Function to extract package names and their corresponding filenames
get_pkg_file_pairs() {
    grep -rE "^[^:]+: (patch|minor|major)" "$CHANGESET_DIR"/*.md 2>/dev/null | \
    sed 's/\.md:/.md|/' | sed 's/:.*//' | awk -F'|' '{print $2":"$1}'
}

echo "🔍 Cascading changes (scoped per file)..."

METADATA=$(cargo metadata --format-version 1 --no-deps)

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

    # If no files were modified in this pass, we are done
    if [ "$NEW_CHANGE_FOUND" = false ]; then
        break
    fi
done

echo "✅ All changesets internally consistent."
