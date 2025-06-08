#!/bin/bash
set -e

echo "generating supergraph file for test..."
npx graphql-federation-audit supergraph --cwd . --test $1

export RUST_LOG=debug

echo "running gateway..."
cargo gateway supergraph.graphql
