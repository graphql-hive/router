#!/usr/bin/env sh

export GRAPH=$(cargo run graph $1) # cargo run graph $1
export URL_ENCODED_GRAPH=$(printf %s "$GRAPH" | jq -sRr @uri) # encodeUriComponent
export DECODED_GRAPH=$(echo $URL_ENCODED_GRAPH | base64) # btoa
export PLAYGROUND_FILE=$(realpath ./lib/query-planner/src/graph/playground.html)

echo "file://$PLAYGROUND_FILE?graph=$DECODED_GRAPH" | pbcopy

echo "✅ Playground URL copied to clipboard"
