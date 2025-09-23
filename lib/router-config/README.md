# Router Configuration

This crate manages the configuration for the GraphQL router.

The following are supported:

- YAML
- JSON (including JSON5)
- Env vars

## JSON Schema

TL;DR: Use `cargo make config` to re-generate the config file.

> Install `cargo-make` by running `cargo install cargo-make`.

To view the JSON schema of the configuration, use the following command:

```
cargo run --release -p hive-router-config
```

To generate a JSON schema file, use the following command:

```
cargo make config
```
