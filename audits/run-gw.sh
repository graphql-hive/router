#!/bin/bash
set -e

echo "generating supergraph file for test..."
npx graphql-federation-audit supergraph --cwd . --test $1
mv supergraph.graphql fed-audit-supergraph.graphql

export RUST_LOG=debug

echo "running gateway..."

cd ..
cargo gateway audits/fed-audit-supergraph.graphql
