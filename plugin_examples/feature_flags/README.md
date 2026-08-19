# Feature flags plugin

This example builds and retains one `Arc<Supergraph>` per feature-flag combination. Each variant includes a complete `SupergraphOptions` snapshot, so executor settings, error masking, and Hive target stay pinned to the same generation as its schema.

Configured sources derive these options from router YAML. Plugin-owned variants must provide them explicitly and keep the owner alive while the variant remains selectable.

## How to run?

This plugin owns the supergraph, so `router.config.yaml` must set `supergraph: source: plugin`.

```bash
cargo run --package feature-flags-plugin-example
```

## Selecting a variant

Clients select a variant by sending the `x-feature-flags` request header (a comma-separated list of enabled flags). The schema uses the `@feature(name: ...)` directive to mark fields/types that should be stripped out for supergraph variants where the flag isn't enabled.
