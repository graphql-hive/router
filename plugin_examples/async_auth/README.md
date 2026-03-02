# Async Authentication

This example plugin demonstrates how to perform an asynchronous authentication in a plugin. The plugin reads client IDs from a file and allows requests with those client IDs to pass through, while rejecting others using `on_graphql_params` hook.

## How to run?

```bash
cargo run --package async-auth-plugin-example
```

## Using the configuration

In the plugin implementation, we allow users to specify the path to the file containing allowed client IDs and the header name from which to read the client ID. 
```rust
#[derive(Deserialize)]
pub struct AllowClientIdConfig {
    pub header: String,
    pub path: String,
}
```

Then in the `on_plugin_init` hook, we get the configuration;
```rust
fn on_plugin_init(payload: OnPluginInitPayload<Self>) -> OnPluginInitResult<Self> {
        let config = payload.config()?;
        payload.initialize_plugin(Self {
            header_key: config.header,
            allowed_ids_path: PathBuf::from(config.path),
        })
}
```

The configuration can be provided in the router configuration file as follows:
```yaml
plugins:
  allow_client_id_from_file:
    enabled: true
    config:
        path: "./allowed_clients.json"
        header: "x-client-id"
```

