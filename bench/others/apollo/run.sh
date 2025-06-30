#!/bin/sh

RUST_LOG=error ./router --supergraph ../../supergraph.graphql --config router.yaml
