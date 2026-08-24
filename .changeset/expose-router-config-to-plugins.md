---
hive-router: minor
---

Expose read-only router config to the plugin system

Plugins can now access the fully resolved router configuration during `on_plugin_init` via `payload.router_config()`, in addition to their own scoped plugin config.

```rust
fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
    let graphql_path = payload.router_config().graphql_path();
    // use router-wide settings to initialize the plugin...
    payload.initialize_plugin_with_defaults()
}
```

This lets plugins adapt their behavior based on router-wide settings.

Closes https://github.com/graphql-hive/router/issues/1437
