---
hive-router: patch
---

# Supergraph options can inherit from router config

Plugins can now use `SupergraphOptions::try_from(&HiveRouterConfig)` to inherit the router's graph-bound settings instead of copying each field manually. The router uses the same conversion when building its configured supergraph.

For example, the router passes an `OnPluginInitPayload` to the plugin's `on_plugin_init` hook. Its `router_config()` method provides read-only access to the router configuration. This plugin loads a `supergraph.graphql` file alongside its source, inherits the router settings, and overrides one field:

```rust
use std::sync::Arc;

use hive_router::{
    async_trait,
    plugins::{
        hooks::{
            on_plugin_init::{OnPluginInitPayload, OnPluginInitResult},
            on_supergraph_load::{Supergraph, SupergraphOptions},
        },
        plugin_trait::RouterPlugin,
    },
};

pub struct PreviewPlugin {
    supergraph: Arc<Supergraph>,
}

#[async_trait]
impl RouterPlugin for PreviewPlugin {
    type Config = ();

    fn plugin_name() -> &'static str {
        "preview"
    }

    fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let mut options = SupergraphOptions::try_from(payload.router_config())?;
        options.traffic_shaping.all.forward_operation_name = true;

        let supergraph = Arc::new(Supergraph::from_sdl(
            "preview",
            include_str!("supergraph.graphql"),
            options,
        )?);

        payload.initialize_plugin(Self { supergraph })
    }
}
```

The conversion inherits query planner options, subgraph traffic shaping, subgraph URL overrides, headers, override labels, demand control, subscription transports, error masking, persisted documents, and the Hive telemetry target. Plugins can then modify individual fields before constructing their supergraph and retain its `Arc<Supergraph>` while it remains selectable.

The Hive telemetry target is resolved, including any configured expression. This makes the conversion fallible, so it uses `TryFrom` rather than `From`; `?` propagates resolution errors from the plugin initialization hook.

Cache overrides remain unset, so the plugin's supergraph inherits the router's `cache.supergraph` limits. Plugins can still override individual cache limits through `options.cache`.
