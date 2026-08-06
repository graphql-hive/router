# Replace schema plugin

This example selects a plugin-owned `Supergraph` when `x-schema-variant: basic` is present. A plugin-owned supergraph must be constructed with a complete `SupergraphOptions` value and retained in an `Arc<Supergraph>` for as long as requests may select it.

The configured supergraph receives graph-bound options from the existing router YAML. The basic variant supplies its own executor traffic shaping, error masking, and Hive target. It does not inherit those values from the configured supergraph.
