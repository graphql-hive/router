---
hive-router: minor
hive-router-config: minor
hive-router-internal: minor
hive-router-query-planner: minor
graphql-tools: minor
hive-router-plan-executor: minor
hive-console-sdk: minor
---

# Plugin System

This release introduces a Plugin System that allows users to extend the functionality of Hive Router by creating custom plugins.

```rust
use hive_router::plugins::plugin_trait::RouterPlugin;
use hive_router::async_trait;
 
struct MyPlugin;
 
#[async_trait]
impl RouterPlugin for MyPlugin {
    type Config = ();
 
    fn plugin_name() -> &'static str {
        "my_plugin"
    }
}
```

You can learn more about the plugin system in the [technical documentation](https://the-guild.dev/graphql/hive/docs/router/plugin-system) and in [Extending the Router guide](https://the-guild.dev/graphql/hive/docs/router/guides/extending-the-router).

This new feaure also exposes many of the Router's internals through the [`hive-router` crate](https://crates.io/crates/hive-router).
