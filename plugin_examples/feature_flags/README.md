# Feature flags plugin

This example builds and retains one `Arc<Supergraph>` per feature-flag combination. Each variant includes a complete `SupergraphOptions` snapshot, so executor settings, error masking, and Hive target stay pinned to the same generation as its schema.

Configured sources derive these options from router YAML. Plugin-owned variants must provide them explicitly and keep the owner alive while the variant remains selectable.
